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
use tokio::time::{timeout, timeout_at, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use teryon_core::protocol::{Answer, ClientMessage, Event, Outcome, Request, ServiceMessage};

use crate::service_file::{ServiceFile, ServiceFileError};

/// How long a client waits for the service to answer an ordinary call or handshake.
const PATIENCE: Duration = Duration::from_secs(10);

/// How long a client waits for calls that stop an agent, run git, or talk to GitHub.
///
/// Discarding a running thread waits up to 30 seconds for the agent to stop,
/// applying re-checks every change in the project, and sending for review
/// pushes and runs `gh`. Three minutes gives those calls room to finish while
/// still failing well before an agent's tool timeout if the service is stuck.
const SLOW_CALL_PATIENCE: Duration = Duration::from_secs(180);

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
        Self::connect_for(data_folder, None).await
    }

    /// Connects to the service in `data_folder`, optionally acting for `thread`.
    pub async fn connect_for(
        data_folder: &Path,
        thread: Option<Uuid>,
    ) -> Result<Self, ClientError> {
        let file = ServiceFile::read(data_folder)?.ok_or(ClientError::NotRunning)?;
        Self::connect_as(&file, thread).await
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
        Self::connect_as(file, None).await
    }

    /// Connects to a service at a port, with its token and an optional acting thread.
    pub async fn connect_as(
        file: &ServiceFile,
        thread: Option<Uuid>,
    ) -> Result<Self, ClientError> {
        if !file.speaks_this_version() {
            return Err(ClientError::DifferentVersion);
        }
        let mut client = Self::greet(file, thread).await?;
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
        match Self::greet(file, None).await {
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
        let mut client = Self::greet(file, None).await?;
        client.receive_text().await?;
        client
            .send(&ClientMessage::Call {
                id: client.next_id,
                request: Request::Shutdown,
            })
            .await
    }

    /// Connects and shows the token, without reading anything back.
    async fn greet(file: &ServiceFile, thread: Option<Uuid>) -> Result<Self, ClientError> {
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
                thread,
            })
            .await?;
        Ok(client)
    }

    /// Sends a request and waits for its answer. Events arriving meanwhile are kept for [`Client::event`].
    ///
    /// Ordinary calls time out after [`PATIENCE`] so a stuck service never
    /// hangs a command indefinitely; git, GitHub, and agent-stopping calls
    /// receive [`SLOW_CALL_PATIENCE`].
    pub async fn call(&mut self, request: Request) -> Result<Answer, ClientError> {
        let deadline = Instant::now() + call_patience(&request);
        let id = self.next_id;
        self.next_id += 1;
        self.send(&ClientMessage::Call { id, request }).await?;
        loop {
            match self.receive_until(deadline).await? {
                Some(ServiceMessage::Reply {
                    id: replied,
                    outcome,
                }) if replied == id => {
                    return match outcome {
                        Outcome::Done { answer } => Ok(answer),
                        Outcome::Failed { message } => Err(ClientError::Refused(message)),
                    };
                }
                Some(ServiceMessage::Reply { .. }) => {}
                Some(ServiceMessage::Event { event }) => self.waiting.push_back(*event),
                None => return Err(ClientError::Unreachable("it took too long".to_string())),
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

    /// The next event arriving before `deadline`, or `None` when the deadline passes.
    ///
    /// Commands such as `teryon thread wait` may wait minutes for another
    /// thread's agent to finish, so they need a deadline of their own instead
    /// of the short read timeout used for ordinary calls.
    pub async fn event_until(&mut self, deadline: Instant) -> Result<Option<Event>, ClientError> {
        if let Some(event) = self.waiting.pop_front() {
            return Ok(Some(event));
        }
        loop {
            let text = match self.receive_text_until(deadline).await? {
                Some(text) => text,
                None => return Ok(None),
            };
            if let Ok(ServiceMessage::Event { event }) = serde_json::from_str::<ServiceMessage>(&text)
            {
                return Ok(Some(*event));
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

    /// The next message this program understands arriving before `deadline`, skipping any it does not.
    async fn receive_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<ServiceMessage>, ClientError> {
        loop {
            let Some(text) = self.receive_text_until(deadline).await? else {
                return Ok(None);
            };
            if let Ok(message) = serde_json::from_str(&text) {
                return Ok(Some(message));
            }
        }
    }

    /// The next text message, whether this program understands it or not.
    async fn receive_text(&mut self) -> Result<String, ClientError> {
        match self.receive_text_until(Instant::now() + PATIENCE).await? {
            Some(text) => Ok(text),
            None => Err(ClientError::Unreachable("it took too long".to_string())),
        }
    }

    /// The next text message arriving before `deadline`, or `None` when it passes.
    async fn receive_text_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<String>, ClientError> {
        loop {
            let Ok(message) = timeout_at(deadline, self.socket.next()).await else {
                return Ok(None);
            };
            match message {
                Some(Ok(Message::Text(text))) => return Ok(Some(text.to_string())),
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    return Err(ClientError::Closed)
                }
                Some(Ok(_)) => {}
            }
        }
    }
}

/// How long [`Client::call`] waits for `request` before reporting a timeout.
fn call_patience(request: &Request) -> Duration {
    match request {
        Request::StartChange { .. }
        | Request::ApplyChange { .. }
        | Request::CatchUp { .. }
        | Request::DiscardChange { .. }
        | Request::SendForReview { .. } => SLOW_CALL_PATIENCE,
        _ => PATIENCE,
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn name_thread() -> Request {
        Request::NameThread {
            thread_id: Uuid::nil(),
            turn: Uuid::nil(),
            title: "Fix".to_string(),
        }
    }

    fn stop() -> Request {
        Request::Stop {
            thread_id: Uuid::nil(),
        }
    }

    fn start_change() -> Request {
        Request::StartChange {
            thread_id: Uuid::nil(),
            turn: Uuid::nil(),
            name: "fix".to_string(),
            from: None,
        }
    }

    fn apply_change() -> Request {
        Request::ApplyChange {
            thread_id: Uuid::nil(),
            description: "Fix".to_string(),
        }
    }

    fn catch_up() -> Request {
        Request::CatchUp {
            thread_id: Uuid::nil(),
        }
    }

    fn discard_change() -> Request {
        Request::DiscardChange {
            thread_id: Uuid::nil(),
        }
    }

    fn send_for_review() -> Request {
        Request::SendForReview {
            thread_id: Uuid::nil(),
            description: "Fix".to_string(),
        }
    }

    lets_expect! {
        // requirement: slow git, GitHub and agent-stopping calls get a longer deadline, while ordinary calls keep the short one
        expect(call_patience(&request)) {
            let request = stop();
            when(request = name_thread()) as own_thread_title to use_short_patience {
                equal(PATIENCE)
            }
            when(request = stop()) as stop_call to use_short_patience {
                equal(PATIENCE)
            }
            when(request = start_change()) as start_change_call to use_slow_patience {
                equal(SLOW_CALL_PATIENCE)
            }
            when(request = apply_change()) as apply_change_call to use_slow_patience {
                equal(SLOW_CALL_PATIENCE)
            }
            when(request = catch_up()) as catch_up_call to use_slow_patience {
                equal(SLOW_CALL_PATIENCE)
            }
            when(request = discard_change()) as discard_change_call to use_slow_patience {
                equal(SLOW_CALL_PATIENCE)
            }
            when(request = send_for_review()) as send_for_review_call to use_slow_patience {
                equal(SLOW_CALL_PATIENCE)
            }
        }
    }
}
