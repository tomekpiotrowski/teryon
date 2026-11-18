//! Request specs for the local service, over real WebSocket connections.
//!
//! Each test starts a service of its own in a temporary home folder, on a
//! free port, with the fake agents from the core library's samples. The
//! rules for projects and threads are covered by the core library's specs;
//! these cover who may connect, and the conversation on top.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use lets_expect::lets_expect;
use tempfile::tempdir;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use common::{program, project_id, scene, thread_id, wait_for, Scene, DEVELOPMENT};
use teryon::client::{Client, ClientError};
use teryon::service::{Service, ServiceError};
use teryon::service_file::ServiceFile;
use teryon::starting::running_service;
use teryon_core::paths::Folders;
use teryon_core::protocol::{Answer, Event, Request};
use teryon_core::store::Status;

impl Scene {
    /// Opens a WebSocket with the given headers and says whether it was accepted.
    async fn opens_with(&self, host: Option<&str>, origin: Option<&str>) -> bool {
        let mut request = format!("ws://127.0.0.1:{}/", self.file.port)
            .into_client_request()
            .unwrap();
        let headers = request.headers_mut();
        match host {
            Some(host) => {
                headers.insert("Host", HeaderValue::from_str(host).unwrap());
            }
            None => {
                headers.remove("Host");
            }
        }
        if let Some(origin) = origin {
            headers.insert("Origin", HeaderValue::from_str(origin).unwrap());
        }
        connect_async(request).await.is_ok()
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.file.port)
    }

    /// Connects, sends `first` as the first message, and says whether the service then talks.
    async fn talks_after(&self, first: Option<String>) -> bool {
        let address = format!("ws://127.0.0.1:{}/", self.file.port);
        let (mut socket, _) = connect_async(address).await.unwrap();
        if let Some(first) = first {
            socket.send(Message::text(first)).await.unwrap();
        }
        match timeout(Duration::from_secs(8), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text.contains("snapshot"),
            _ => false,
        }
    }

    fn hello(&self, token: &str) -> Option<String> {
        Some(format!(r#"{{"type":"hello","token":"{token}"}}"#))
    }
}

/// The token with its last character changed, so it is wrong in one place only.
fn nearly(token: &str) -> String {
    let last = if token.ends_with('0') { '1' } else { '0' };
    format!("{}{last}", &token[..token.len() - 1])
}

fn no_entries() -> Answer {
    Answer::Entries {
        entries: Vec::new(),
    }
}

const SHUTDOWN_FIRST: &str = r#"{"type":"call","id":1,"request":{"type":"shutdown"}}"#;
const UNKNOWN_REQUEST: &str = r#"{"type":"call","id":5,"request":{"type":"dance"}}"#;
const UNFOLLOW: &str = r#"{"type":"call","id":6,"request":{"type":"unfollow_thread","thread_id":"00000000-0000-0000-0000-000000000000"}}"#;
const SIXTH: &str = r#""id":6"#;

lets_expect! { #tokio_test
    // requirement: a connection is accepted only from 127.0.0.1 or localhost at the service's port
    // requirement: the Origin header is absent, the desktop app's own, or listed in the configuration
    story who_may_open_a_connection {
        let scene = scene().await;
        let host = scene.host();
        let named = format!("localhost:{}", scene.file.port);

        expect(scene.opens_with(Some(&host), None).await) to be_true
        expect(scene.opens_with(Some(&named), None).await) to be_true
        expect(scene.opens_with(Some(&host), Some("tauri://localhost")).await) to be_true
        expect(scene.opens_with(Some(&host), Some("http://tauri.localhost")).await) to be_true
        expect(scene.opens_with(Some(&host), Some(DEVELOPMENT)).await) to be_true
        expect(scene.opens_with(Some(&host), Some("https://example.com")).await) to be_false
        expect(scene.opens_with(Some(&host), Some("null")).await) to be_false
        expect(scene.opens_with(Some("evil.example.com"), None).await) to be_false
        expect(scene.opens_with(Some(&format!("evil.example.com:{}", scene.file.port)), Some(DEVELOPMENT)).await) to be_false
        expect(scene.opens_with(Some("127.0.0.1:1"), None).await) to be_false
    }

    // requirement: the first message carries the current token
    // requirement: a wrong token, a missing token and requests before the token are refused
    story the_service_talks_only_after_the_token {
        let scene = scene().await;
        let token = scene.file.token.clone();
        let nearly = nearly(&token);

        expect(scene.talks_after(scene.hello(&token)).await) to be_true
        expect(scene.talks_after(scene.hello(&nearly)).await) to be_false
        expect(scene.talks_after(scene.hello("")).await) to be_false
        expect(scene.talks_after(Some(SHUTDOWN_FIRST.to_string())).await) to be_false
        expect(scene.talks_after(Some("not json".to_string())).await) to be_false
        expect(scene.file.token.len()) to equal(64)
        expect(ServiceFile::read(&Folders::inside(scene.home.path()).data).unwrap().is_some()) to be_true
    }

    // requirement: a connection that sends no token within 5 seconds is closed
    story a_silent_connection_is_closed {
        let scene = scene().await;
        expect(scene.talks_after(None).await) to be_false
    }

    // requirement: a token from an earlier start is refused
    story a_token_from_an_earlier_start_is_refused {
        let first = scene().await;
        let earlier = first.file.clone();
        let second = scene().await;
        let mismatched = ServiceFile { port: second.file.port, ..earlier };

        expect(Client::connect_to(&mismatched).await.is_err()) to be_true
    }

    // requirement: a second service for the same person sees the first one running and ends
    story a_second_service_is_refused {
        let scene = scene().await;
        let second = Service::start(Folders::inside(scene.home.path()), program()).await;
        let check_1 = matches!(second, Err(ServiceError::AlreadyRunning));
        expect(check_1) to be_true
    }

    // requirement: after the token, the service sends the projects and threads, then every change
    // requirement: a client following a thread receives its entries so far, then each new one
    story a_conversation_over_the_connection {
        let scene = scene().await;
        let folder = tempdir().unwrap();
        let mut client = scene.client().await;
        let mut watcher = scene.client().await;
        let check_2 = matches!(client.event().await.unwrap(), Event::Snapshot { .. });
        expect(check_2) to be_true
        let check_3 = matches!(watcher.event().await.unwrap(), Event::Snapshot { .. });
        expect(check_3) to be_true

        let project = project_id(client.call(Request::AddProject { name: "Sample".to_string(), directory: folder.path().to_string_lossy().into_owned() }).await.unwrap());
        let added = wait_for(&mut watcher, |event| matches!(event, Event::ProjectAdded { .. })).await;
        let check_4 = matches!(added, Event::ProjectAdded { project: ref added } if added.id == project);
        expect(check_4) to be_true

        let thread = thread_id(client.call(Request::StartThread { project_id: project, agent: "claude".to_string(), title: String::new() }).await.unwrap());
        let followed = client.call(Request::FollowThread { thread_id: thread }).await.unwrap();
        expect(followed) to equal(no_entries())

        client.call(Request::SendMessage { thread_id: thread, text: "hello".to_string() }).await.unwrap();
        let summary = wait_for(&mut client, |event| matches!(event, Event::EntryAdded { entry } if entry.kind == "turn_summary")).await;
        let check_5 = matches!(summary, Event::EntryAdded { .. });
        expect(check_5) to be_true
        let idle = wait_for(&mut watcher, |event| matches!(event, Event::ThreadChanged { thread: changed } if changed.status == Status::Idle && changed.unread)).await;
        let check_6 = matches!(idle, Event::ThreadChanged { .. });
        expect(check_6) to be_true

        let seen = client.call(Request::MarkSeen { thread_id: thread }).await;
        expect(seen.is_ok()) to be_true
    }

    // requirement: entries are sent only for threads the client follows
    story entries_of_other_threads_are_not_sent {
        let scene = scene().await;
        let folder = tempdir().unwrap();
        let mut client = scene.client().await;
        let project = project_id(client.call(Request::AddProject { name: "Sample".to_string(), directory: folder.path().to_string_lossy().into_owned() }).await.unwrap());
        let thread = thread_id(client.call(Request::StartThread { project_id: project, agent: "claude".to_string(), title: String::new() }).await.unwrap());

        client.call(Request::SendMessage { thread_id: thread, text: "hello".to_string() }).await.unwrap();
        wait_for(&mut client, |event| matches!(event, Event::ThreadChanged { thread: changed } if changed.status == Status::Idle)).await;
        let mut entries = 0;
        while let Ok(Ok(event)) = timeout(Duration::from_millis(200), client.event()).await {
            if matches!(event, Event::EntryAdded { .. }) { entries += 1; }
        }
        expect(entries) to equal(0)
    }

    // requirement: an unknown request, or a request missing a field, gets an error reply and the connection stays open
    story a_request_the_service_does_not_know_is_answered_with_an_error {
        let scene = scene().await;
        let address = format!("ws://127.0.0.1:{}/", scene.file.port);
        let (mut socket, _) = connect_async(address).await.unwrap();
        socket.send(Message::text(scene.hello(&scene.file.token).unwrap())).await.unwrap();
        socket.next().await;
        socket.send(Message::text(UNKNOWN_REQUEST)).await.unwrap();
        let reply = socket.next().await.unwrap().unwrap().into_text().unwrap().to_string();
        expect(reply.contains("failed")) to be_true

        socket.send(Message::text(UNFOLLOW)).await.unwrap();
        let next = socket.next().await.unwrap().unwrap().into_text().unwrap().to_string();
        expect(next.contains(SIXTH)) to be_true
    }

    // requirement: a refused request is answered with a message meant for the person
    story a_refused_request_says_why {
        let scene = scene().await;
        let mut client = scene.client().await;
        let refused = client.call(Request::AddProject { name: " ".to_string(), directory: "/".to_string() }).await;
        let check_7 = matches!(refused, Err(ClientError::Refused(ref message)) if message == "A project needs a name.");
        expect(check_7) to be_true
    }

    // requirement: teryon stop stops the running service; it removes service.json on the way out
    story the_service_stops_when_asked {
        let scene = scene().await;
        let mut client = scene.client().await;
        expect(client.call(Request::Shutdown).await.is_ok()) to be_true

        let stopped = timeout(Duration::from_secs(15), scene.stopped).await;
        expect(stopped.is_ok()) to be_true
        expect(ServiceFile::read(&Folders::inside(scene.home.path()).data).unwrap()) to equal(None)
    }
}

lets_expect! { #tokio_test
    // requirement: a client starts the service when none is running, and finds it when one is
    story the_service_is_started_when_needed {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());

        let started = running_service(&program(), &folders.data, &folders.logs, Some(home.path())).await;
        expect(started.is_ok()) to be_true
        let first = started.unwrap();
        let again = running_service(&program(), &folders.data, &folders.logs, Some(home.path())).await.unwrap();
        expect(again.clone()) to equal(first.clone())

        let mut client = Client::connect_to(&first).await.unwrap();
        client.call(Request::Shutdown).await.unwrap();
    }

    // requirement: when the service cannot be started, the client says so
    story a_program_that_is_not_there_is_reported {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());
        let missing = PathBuf::from("/nowhere/teryon");

        let started = running_service(&missing, &folders.data, &folders.logs, Some(home.path())).await;
        expect(started.map_err(|problem| problem.to_string()).err().unwrap_or_default().starts_with("Could not start Teryon's service")) to be_true
    }
}
