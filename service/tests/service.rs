//! Request specs for the local service, over real WebSocket connections.
//!
//! Each test starts a service of its own in a temporary home folder, on a
//! free port, with the fake agents from the core library's samples. The
//! rules for projects and threads are covered by the core library's specs;
//! these cover who may connect, and the conversation on top.

mod common;

use std::fs::{create_dir_all, write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use lets_expect::lets_expect;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::task::{spawn, JoinHandle};
use tokio::time::timeout;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use common::{free_port, program, project_id, scene, thread_id, wait_for, Scene, DEVELOPMENT};
use teryon::client::{Client, ClientError};
use teryon::service::{Service, ServiceError};
use teryon::service_file::ServiceFile;
use teryon::starting::{restarted_service, running_service, StartError};
use teryon::stopping::{stop_service, Stopped};
use teryon_core::agents::Voice;
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

/// A stand-in for a service of an older version, in `data`.
///
/// It accepts its token and then sends a snapshot no current client can
/// read. It ends, removing its `service.json` the way a real service does,
/// when asked to stop. The handle says whether it was asked.
async fn older_service(data: &Path) -> (ServiceFile, JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let file = ServiceFile {
        port: listener.local_addr().unwrap().port(),
        token: "older".to_string(),
        process: 1,
        protocol: String::new(),
    };
    create_dir_all(data).unwrap();
    write(
        ServiceFile::path(data),
        format!(r#"{{"port":{},"token":"older","process":1}}"#, file.port),
    )
    .unwrap();
    let data = data.to_path_buf();
    let asked = spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok(mut socket) = accept_async(stream).await else {
                continue;
            };
            let Some(Ok(Message::Text(hello))) = socket.next().await else {
                continue;
            };
            if !hello.contains("older") {
                continue;
            }
            let snapshot =
                r#"{"type":"event","event":{"type":"snapshot","shape":"from long ago"}}"#;
            if socket.send(Message::text(snapshot)).await.is_err() {
                continue;
            }
            if let Ok(Some(Ok(Message::Text(call)))) =
                timeout(Duration::from_secs(5), socket.next()).await
            {
                if call.contains("shutdown") {
                    let _ = std::fs::remove_file(ServiceFile::path(&data));
                    return true;
                }
            }
        }
    });
    (file, asked)
}

/// A program standing in for `teryon` that, instead of starting a service, points `service.json` at `file`.
///
/// Like an old `teryon` program left next to a newer app.
fn program_pointing_at(folder: &Path, file: &ServiceFile, data: &Path) -> PathBuf {
    let program = folder.join("old-teryon");
    write(
        &program,
        format!(
            "#!/bin/sh\nprintf '{{\"port\":{},\"token\":\"{}\",\"process\":1}}' > '{}'\n",
            file.port,
            file.token,
            ServiceFile::path(data).display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    program
}

fn is_different_version_running(started: &Result<ServiceFile, StartError>) -> bool {
    matches!(started, Err(StartError::DifferentVersionRunning))
}

fn is_different_version_started(started: &Result<ServiceFile, StartError>) -> bool {
    matches!(started, Err(StartError::DifferentVersionStarted { .. }))
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

        let thread = thread_id(client.call(Request::StartThread { project_id: project, agent: "claude".to_string(), model: None, title: String::new() }).await.unwrap());
        let followed = client.call(Request::FollowThread { thread_id: thread }).await.unwrap();
        expect(followed) to equal(no_entries())

        client.call(Request::SendMessage { thread_id: thread, text: "hello".to_string(), to: Voice::Author }).await.unwrap();
        let summary = wait_for(&mut client, |event| matches!(event, Event::EntryAdded { entry } if entry.kind == "turn_summary")).await;
        let check_5 = matches!(summary, Event::EntryAdded { .. });
        expect(check_5) to be_true
        let idle = wait_for(&mut watcher, |event| matches!(event, Event::ThreadChanged { thread: changed } if changed.status == Status::Idle && changed.unread)).await;
        let check_6 = matches!(idle, Event::ThreadChanged { .. });
        expect(check_6) to be_true

        let seen = client.call(Request::MarkSeen { thread_id: thread }).await;
        expect(seen.is_ok()) to be_true
    }

    // requirement: clients are offered each agent's models, with Grok's once the service has asked it
    // requirement: a thread starts on a chosen model, which can be changed between messages
    story models_over_the_connection {
        let scene = scene().await;
        let folder = tempdir().unwrap();
        let mut client = scene.client().await;
        let snapshot = match client.event().await.unwrap() {
            Event::Snapshot { models, .. } => Some(models),
            _ => None,
        };
        expect(snapshot.as_ref().map(|models| models.claude.clone())) to equal(Some(vec!["opus".to_string(), "sonnet".to_string(), "haiku".to_string()]))
        // Grok is asked when the service starts, which may finish before or after this client connects:
        // its models arrive in the snapshot or in a later update.
        let grok = match snapshot.map(|models| models.grok).filter(|grok| !grok.is_empty()) {
            Some(grok) => grok,
            None => match wait_for(&mut client, |event| matches!(event, Event::ModelsChanged { .. })).await {
                Event::ModelsChanged { models } => models.grok,
                _ => Vec::new(),
            },
        };
        expect(grok.first().cloned()) to equal(Some("grok-4.7".to_string()))

        let project = project_id(client.call(Request::AddProject { name: "Sample".to_string(), directory: folder.path().to_string_lossy().into_owned() }).await.unwrap());
        let started = client.call(Request::StartThread { project_id: project, agent: "claude".to_string(), model: Some("sonnet".to_string()), title: String::new() }).await.unwrap();
        let thread = match started { Answer::Thread { thread } => thread, _ => panic!("a thread") };
        expect(thread.model.clone()) to equal(Some("sonnet".to_string()))

        let changed = client.call(Request::ChangeModel { thread_id: thread.id, to: Voice::Author, model: Some("opus".to_string()) }).await.unwrap();
        let model = match changed { Answer::Thread { thread } => thread.model, _ => None };
        expect(model) to equal(Some("opus".to_string()))
        let refused = client.call(Request::ChangeModel { thread_id: thread.id, to: Voice::Reviewer, model: None }).await;
        expect(refused.is_err()) to be_true
    }

    // requirement: entries are sent only for threads the client follows
    story entries_of_other_threads_are_not_sent {
        let scene = scene().await;
        let folder = tempdir().unwrap();
        let mut client = scene.client().await;
        let project = project_id(client.call(Request::AddProject { name: "Sample".to_string(), directory: folder.path().to_string_lossy().into_owned() }).await.unwrap());
        let thread = thread_id(client.call(Request::StartThread { project_id: project, agent: "claude".to_string(), model: None, title: String::new() }).await.unwrap());

        client.call(Request::SendMessage { thread_id: thread, text: "hello".to_string(), to: Voice::Author }).await.unwrap();
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

    // requirement: teryon stop waits until the service has stopped every agent and ended
    story stopping_waits_for_the_service_to_end {
        let scene = scene().await;
        expect(stop_service(&scene.data()).await.unwrap()) to equal(Stopped::Stopped)
        expect(scene.stopped.is_finished()) to be_true
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
    // requirement: a service of another version is reported at once, not waited on, and no second one is started
    story a_different_version_running_is_reported {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());
        let (older, _asked) = older_service(&folders.data).await;

        let started = timeout(Duration::from_secs(5), running_service(&program(), &folders.data, &folders.logs, Some(home.path()))).await;
        expect(started.is_ok()) to be_true
        let started = started.unwrap();
        expect(is_different_version_running(&started)) to be_true
        expect(Client::answers(&older).await) to be_true
        expect(ServiceFile::read(&folders.data).unwrap().map(|file| file.port)) to equal(Some(older.port))
    }

    // requirement: a client does not try to talk to a service of another version
    story connecting_to_a_different_version_is_refused {
        let home = tempdir().unwrap();
        let (older, _asked) = older_service(&Folders::inside(home.path()).data).await;
        let check = matches!(Client::connect_to(&older).await, Err(ClientError::DifferentVersion));
        expect(check) to be_true
    }

    // requirement: when the started program is itself of another version, the client says so instead of waiting
    story a_different_version_started_is_reported {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());
        let elsewhere = tempdir().unwrap();
        let (older, _asked) = older_service(&elsewhere.path().join("data")).await;
        let old_program = program_pointing_at(elsewhere.path(), &older, &folders.data);
        create_dir_all(&folders.data).unwrap();

        let started = running_service(&old_program, &folders.data, &folders.logs, Some(home.path())).await;
        expect(is_different_version_started(&started)) to be_true
    }

    // requirement: teryon stop stops a service of another version, too
    story a_different_version_is_stopped {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());
        let (_older, asked) = older_service(&folders.data).await;

        expect(stop_service(&folders.data).await.unwrap()) to equal(Stopped::Stopped)
        expect(timeout(Duration::from_secs(5), asked).await.unwrap().unwrap()) to be_true
        expect(ServiceFile::read(&folders.data).unwrap()) to equal(None)
    }

    // requirement: restarting replaces a service of another version with this version's
    story a_different_version_is_replaced_on_restart {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());
        let (_older, asked) = older_service(&folders.data).await;

        let restarted = restarted_service(&program(), &folders.data, &folders.logs, Some(home.path())).await.unwrap();
        expect(timeout(Duration::from_secs(5), asked).await.unwrap().unwrap()) to be_true
        expect(restarted.speaks_this_version()) to be_true
        expect(Client::connect_to(&restarted).await.is_ok()) to be_true

        expect(stop_service(&folders.data).await.unwrap()) to equal(Stopped::Stopped)
    }

    // requirement: teryon stop says so when nothing is running
    story stopping_when_nothing_runs {
        let home = tempdir().unwrap();
        let folders = Folders::inside(home.path());
        expect(stop_service(&folders.data).await.unwrap()) to equal(Stopped::NotRunning)

        let unused = free_port().await;
        create_dir_all(&folders.data).unwrap();
        write(ServiceFile::path(&folders.data), format!(r#"{{"port":{unused},"token":"gone","process":1}}"#)).unwrap();
        expect(stop_service(&folders.data).await.unwrap()) to equal(Stopped::NotRunning)
    }
}
