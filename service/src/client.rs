//! Talks to the local service from another program on the same machine.
//!
//! `teryon stop`, the terminal app and the tests use this. It reads
//! `service.json`, connects, shows the token, and then sends calls and
//! receives replies and events.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use teryon_core::protocol::{Answer, ClientMessage, Event, Outcome, Request, ServiceMessage};

use crate::service_file::{ServiceFile, ServiceFileError};

/// How long a client waits for the service to answer anything.
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Teryon is not running.")]
    NotRunning,
    #[error(
        "A different version of Teryon is running. Run `teryon stop`, then start Teryon again."
    )]
    DifferentVersion,
    #[error("The Teryon service did not answer: {0}")]
    Unreachable(String),
    #[error("The Teryon service closed the connection.")]
    Closed,
    /// The service refused the call; the text says why.
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    ServiceFile(#[from] ServiceFileError),
}

/// A connection to the service, past the token.
pub struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    /// Events that arrived while waiting for a reply, in order.
    waiting: VecDeque<Event>,
}

impl Client {
    /// Connects to the service named in `service.json` in `data_folder`.
    pub async fn connect(data_folder: &Path) -> Result<Self, ClientError> {
        let file = ServiceFile::read(data_folder)?.ok_or(ClientError::NotRunning)?;
        Self::connect_to(&file).await
    }

    /// Connects to a service at a port, with its token.
    ///
    /// The first event, the snapshot of every project and thread, is how a
    /// client knows the token was accepted: a refused client is disconnected
    /// without one.
    ///
    /// A service of another version is refused before connecting: its
    /// messages may not be understood here, and waiting for them would only
    /// time out.
    pub async fn connect_to(file: &ServiceFile) -> Result<Self, ClientError> {
        if !file.speaks_this_version() {
            return Err(ClientError::DifferentVersion);
        }
        let mut client = Self::greet(file).await?;
        match client.receive().await? {
            ServiceMessage::Event { event } => client.waiting.push_back(*event),
            ServiceMessage::Reply { .. } => return Err(ClientError::Closed),
        }
        Ok(client)
    }

    /// Whether a service of any version answers at the port in `file`, and accepts its token.
    ///
    /// Telling a service that is not there from one this program cannot
    /// understand needs this: [`Client::connect_to`] fails on both.
    pub async fn answers(file: &ServiceFile) -> bool {
        match Self::greet(file).await {
            Ok(mut client) => client.receive_text().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Asks the service to stop, without waiting for it to.
    ///
    /// It works with a service of another version too: only the token and
    /// the request to stop must be understood, and the reply is not read,
    /// because a service of another version may answer in a way this
    /// program cannot read.
    pub async fn ask_to_stop(file: &ServiceFile) -> Result<(), ClientError> {
        let mut client = Self::greet(file).await?;
        client.receive_text().await?;
        client
            .send(&ClientMessage::Call {
                id: client.next_id,
                request: Request::Shutdown,
            })
            .await
    }

    /// Connects and shows the token, without reading anything back.
    async fn greet(file: &ServiceFile) -> Result<Self, ClientError> {
        let address = format!("ws://127.0.0.1:{}/", file.port);
        let (socket, _) = timeout(PATIENCE, connect_async(&address))
            .await
            .map_err(|_| ClientError::Unreachable("it took too long".to_string()))?
            .map_err(|problem| ClientError::Unreachable(problem.to_string()))?;
        let mut client = Self {
            socket,
            next_id: 1,
            waiting: VecDeque::new(),
        };
        client
            .send(&ClientMessage::Hello {
                token: file.token.clone(),
            })
            .await?;
        Ok(client)
    }

    /// Sends a request and waits for its answer. Events arriving meanwhile are kept for [`Client::event`].
    pub async fn call(&mut self, request: Request) -> Result<Answer, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&ClientMessage::Call { id, request }).await?;
        loop {
            match self.receive().await? {
                ServiceMessage::Reply {
                    id: replied,
                    outcome,
                } if replied == id => {
                    return match outcome {
                        Outcome::Done { answer } => Ok(answer),
                        Outcome::Failed { message } => Err(ClientError::Refused(message)),
                    };
                }
                ServiceMessage::Reply { .. } => {}
                ServiceMessage::Event { event } => self.waiting.push_back(*event),
            }
        }
    }

    /// The next event, waiting for one when none has arrived yet.
    pub async fn event(&mut self) -> Result<Event, ClientError> {
        if let Some(event) = self.waiting.pop_front() {
            return Ok(event);
        }
        loop {
            if let ServiceMessage::Event { event } = self.receive().await? {
                return Ok(*event);
            }
        }
    }

    async fn send(&mut self, message: &ClientMessage) -> Result<(), ClientError> {
        let text = serde_json::to_string(message)
            .map_err(|problem| ClientError::Refused(problem.to_string()))?;
        self.socket
            .send(Message::text(text))
            .await
            .map_err(|_| ClientError::Closed)
    }

    /// The next message this program understands, skipping any it does not.
    async fn receive(&mut self) -> Result<ServiceMessage, ClientError> {
        loop {
            if let Ok(message) = serde_json::from_str(&self.receive_text().await?) {
                return Ok(message);
            }
        }
    }

    /// The next text message, whether this program understands it or not.
    async fn receive_text(&mut self) -> Result<String, ClientError> {
        loop {
            let message = timeout(PATIENCE, self.socket.next())
                .await
                .map_err(|_| ClientError::Unreachable("it took too long".to_string()))?;
            match message {
                Some(Ok(Message::Text(text))) => return Ok(text.to_string()),
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    return Err(ClientError::Closed)
                }
                Some(Ok(_)) => {}
            }
        }
    }
}
