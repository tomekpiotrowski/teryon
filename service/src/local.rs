//! The WebSocket that programs on this machine connect to.
//!
//! Before anything else, every connection passes the checks in
//! [`crate::guard`]: the headers before the WebSocket is accepted, the token
//! as the first message after. Then it becomes a [`Session`], which runs until
//! either side closes.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::time::timeout;
use tracing::{info, warn};

use teryon_core::protocol::{ClientMessage, Event, Outcome, ServiceMessage};
use teryon_core::workspace::Workspace;

use crate::guard::{admit, token_matches};
use crate::session::{Aftermath, Origin, Session};

/// How long a new connection has to show the token.
const HELLO_PATIENCE: Duration = Duration::from_secs(5);

/// The largest message accepted from a client.
const LARGEST_MESSAGE: usize = 1024 * 1024;

/// How many ports after the usual one are tried when it is taken.
const PORTS_TRIED: u16 = 20;

/// What every connection needs to know.
pub struct Listener {
    pub workspace: Workspace,
    pub token: String,
    pub port: u16,
    pub allowed_origins: Vec<String>,
    /// Set when a client asks the service to stop.
    pub stopping: watch::Sender<bool>,
}

/// Binds to `127.0.0.1` on `port`, or on the first free port after it.
pub async fn bind(port: u16) -> Result<TcpListener, std::io::Error> {
    let mut last = None;
    for candidate in port..port.saturating_add(PORTS_TRIED) {
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, candidate))).await {
            Ok(listener) => return Ok(listener),
            Err(problem) => last = Some(problem),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no port to try")))
}

pub fn router(listener: Arc<Listener>) -> Router {
    Router::new().route("/", get(connect)).with_state(listener)
}

async fn connect(
    State(listener): State<Arc<Listener>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    if admit(host, origin, listener.port, &listener.allowed_origins).is_err() {
        warn!("refused a connection from host {host:?}, origin {origin:?}");
        return StatusCode::FORBIDDEN.into_response();
    }
    upgrade
        .max_message_size(LARGEST_MESSAGE)
        .on_upgrade(move |socket| serve(socket, listener))
}

/// Waits for the token, then runs the session until either side closes.
async fn serve(mut socket: WebSocket, listener: Arc<Listener>) {
    let greeted = match timeout(HELLO_PATIENCE, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Hello { token }) => token_matches(&listener.token, &token),
            _ => false,
        },
        _ => false,
    };
    if !greeted {
        warn!("refused a connection that did not show the token");
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    // Listening starts before the snapshot is taken, so nothing in between is missed.
    let mut changes = listener.workspace.subscribe();
    let mut session = Session::new(listener.workspace.clone(), Origin::Local);
    if send(&mut socket, &event(listener.workspace.snapshot()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(_)) => continue,
                };
                let (id, request) = match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(ClientMessage::Call { id, request }) => (id, request),
                    // Showing the token again changes nothing.
                    Ok(ClientMessage::Hello { .. }) => continue,
                    Err(problem) => {
                        let message = format!("Not a request Teryon knows: {problem}");
                        let reply = ServiceMessage::Reply { id: 0, outcome: Outcome::Failed { message } };
                        if send(&mut socket, &reply).await.is_err() { return; }
                        continue;
                    }
                };
                let (answer, aftermath) = session.answer(request).await;
                let outcome = match answer {
                    Ok(answer) => Outcome::Done { answer },
                    Err(message) => Outcome::Failed { message },
                };
                if send(&mut socket, &ServiceMessage::Reply { id, outcome }).await.is_err() {
                    return;
                }
                if aftermath == Aftermath::ShutDown {
                    info!("stopping because a client asked to");
                    let _ = listener.stopping.send(true);
                }
            }
            change = changes.recv() => {
                let message = match change {
                    Ok(change) if session.wants(&change) => event(change),
                    Ok(_) => continue,
                    // Fell behind: start over from everything as it is now.
                    Err(RecvError::Lagged(_)) => event(listener.workspace.snapshot()),
                    Err(RecvError::Closed) => return,
                };
                if send(&mut socket, &message).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn event(event: Event) -> ServiceMessage {
    ServiceMessage::Event { event }
}

async fn send(socket: &mut WebSocket, message: &ServiceMessage) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    socket
        .send(Message::Text(Utf8Bytes::from(text)))
        .await
        .map_err(|_| ())
}
