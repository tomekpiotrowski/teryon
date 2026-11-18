//! What the service's specs share: a running service of their own, and a few ways to talk to it.
//!
//! Each test starts a service in a temporary home folder, on a free port,
//! with the fake agents from the core library's samples, so nothing reaches
//! a model provider or a person's real projects.
//!
//! Each test file compiles this module for itself and uses only part of it.
#![allow(dead_code)]

use std::fs::{create_dir_all, write};
use std::future::pending;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tempfile::{tempdir, TempDir};
use tokio::net::TcpListener;
use tokio::task::{spawn, JoinHandle};
use tokio::time::timeout;
use uuid::Uuid;

use teryon::client::Client;
use teryon::service::Service;
use teryon::service_file::ServiceFile;
use teryon_core::paths::Folders;
use teryon_core::protocol::{Answer, Event};

/// The development app's address, listed as allowed in the test configuration.
pub const DEVELOPMENT: &str = "http://localhost:4200";

/// A running service in a home folder of its own.
pub struct Scene {
    pub home: TempDir,
    pub file: ServiceFile,
    pub stopped: JoinHandle<()>,
}

pub async fn scene() -> Scene {
    scene_with("").await
}

/// A running service whose `config.toml` also holds `more`.
pub async fn scene_with(more: &str) -> Scene {
    let home = tempdir().unwrap();
    let folders = Folders::inside(home.path());
    create_dir_all(&folders.config).unwrap();
    write(
        folders.config.join("config.toml"),
        format!(
            "[service]\nport = {}\nallowed_origins = [\"{DEVELOPMENT}\"]\n\n\
             [agents]\nstop_grace_seconds = 1\n\n\
             [agents.claude]\nprogram = \"../core/tests/samples/fake-claude.sh\"\n\n\
             [agents.grok]\nprogram = \"../core/tests/samples/fake-grok.sh\"\n\n\
             {more}",
            free_port().await
        ),
    )
    .unwrap();
    let service = Service::start(folders, program())
        .await
        .expect("the service starts");
    let file = service.file().clone();
    let stopped = spawn(service.run(pending()));
    Scene {
        home,
        file,
        stopped,
    }
}

impl Scene {
    pub async fn client(&self) -> Client {
        Client::connect_to(&self.file)
            .await
            .expect("the client connects")
    }

    /// The folder holding `service.json`.
    pub fn data(&self) -> PathBuf {
        Folders::inside(self.home.path()).data
    }
}

/// A port nobody listens on right now, so tests running at once do not meet.
pub async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// Waits for the next event that `wanted` accepts.
pub async fn wait_for(client: &mut Client, wanted: impl Fn(&Event) -> bool) -> Event {
    timeout(Duration::from_secs(10), async {
        loop {
            let event = client.event().await.expect("an event");
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("the event arrived in time")
}

pub fn project_id(answer: Answer) -> Uuid {
    match answer {
        Answer::Project { project } => project.id,
        other => panic!("not a project: {other:?}"),
    }
}

pub fn thread_id(answer: Answer) -> Uuid {
    match answer {
        Answer::Thread { thread } => thread.id,
        other => panic!("not a thread: {other:?}"),
    }
}

/// The `teryon` program cargo built for these tests.
pub fn program() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_teryon"))
}

/// Runs git in a directory and returns what it printed, ignoring the settings of whoever runs the tests.
pub fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Tester")
        .env("GIT_AUTHOR_EMAIL", "tester@example.com")
        .env("GIT_COMMITTER_NAME", "Tester")
        .env("GIT_COMMITTER_EMAIL", "tester@example.com")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A folder that is a repository on `main` with one commit.
pub fn repository() -> TempDir {
    let folder = tempdir().unwrap();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    write(folder.path().join("README.md"), "hello\n").unwrap();
    git(folder.path(), &["add", "-A"]);
    git(folder.path(), &["commit", "-q", "-m", "start"]);
    folder
}
