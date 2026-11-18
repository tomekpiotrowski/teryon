//! Shared request-spec helpers. See the Erno Testing guide.
//!
//! Each test file compiles this module for itself and uses only part of it.
#![allow(dead_code)]

use std::fs::{create_dir_all, read_to_string, write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use axum_test::TestResponse;
pub use erno::tests::{bearer, no_fixtures, setup_test, verified_user, TestUtils};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde_json::{json, Value};
use tempfile::{tempdir_in, TempDir};
use tokio::time::sleep;
use uuid::Uuid;

use teryon::boot_config;
use teryon::workspace::{project, thread, thread_entry};
use teryon::{Migrator, Settings};

/// The one folder the test configuration allows projects in.
const PROJECTS: &str = "target/test-projects";

pub async fn setup() -> TestUtils {
    setup_test::<Migrator, Settings>(boot_config(), no_fixtures).await
}

/// A fresh directory that may become a project.
pub fn project_folder() -> TempDir {
    create_dir_all(PROJECTS).expect("the folder for test projects");
    tempdir_in(PROJECTS).expect("a project folder")
}

/// Runs git in a directory and returns what it printed, without the line ending.
///
/// The repositories made for tests ignore the settings of whoever runs them.
pub fn git(directory: &Path, arguments: &[&str]) -> String {
    run_git(directory, None, arguments)
}

/// Runs git as if it were the given moment, in seconds since 1970.
///
/// A commit made this way carries that moment as the time it was made.
pub fn git_at(directory: &Path, time: i64, arguments: &[&str]) -> String {
    run_git(directory, Some(time), arguments)
}

fn run_git(directory: &Path, time: Option<i64>, arguments: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Tester")
        .env("GIT_AUTHOR_EMAIL", "tester@example.com")
        .env("GIT_COMMITTER_NAME", "Tester")
        .env("GIT_COMMITTER_EMAIL", "tester@example.com");
    if let Some(time) = time {
        command
            .env("GIT_AUTHOR_DATE", format!("{time} +0000"))
            .env("GIT_COMMITTER_DATE", format!("{time} +0000"));
    }
    let output = command.output().expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Writes a file in a directory, making the folders on the way.
pub fn put(directory: &Path, path: &str, content: impl AsRef<[u8]>) {
    let file = directory.join(path);
    create_dir_all(file.parent().expect("a folder")).expect("the folder is made");
    write(file, content).expect("the file is written");
}

/// Commits everything in a directory as it is.
pub fn commit(directory: &Path) {
    git(directory, &["add", "-A"]);
    git(directory, &["commit", "-q", "-m", "change"]);
}

/// How the fake agent program was started in a project folder, one line per run.
pub fn invocations(folder: &TempDir) -> Vec<String> {
    read_to_string(folder.path().join(".fake-agent-invocations"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// A text field of a JSON answer, or nothing when it is missing.
pub fn text_field(response: &TestResponse, field: &str) -> String {
    response.json::<Value>()[field]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// A running Teryon with one signed-in user.
pub struct Scene {
    pub t: TestUtils,
    pub token: String,
}

pub async fn scene() -> Scene {
    let t = setup().await;
    let user = verified_user(&t.db, "owner@example.com", "a long password").await;
    let token = bearer(&t, &user);
    Scene { t, token }
}

impl Scene {
    /// Signs in a second person, to check what others may see and do.
    pub async fn someone_else(&self) -> String {
        let user = verified_user(&self.t.db, "other@example.com", "a long password").await;
        bearer(&self.t, &user)
    }

    pub async fn post(&self, path: &str, body: Value) -> TestResponse {
        self.post_as(&self.token, path, body).await
    }

    pub async fn post_as(&self, token: &str, path: &str, body: Value) -> TestResponse {
        self.t
            .server
            .post(path)
            .add_header("Authorization", token.to_string())
            .json(&body)
            .await
    }

    /// Asks for something, with the given `name=value` pairs after the address.
    pub async fn get(&self, path: &str, query: &[(&str, &str)]) -> TestResponse {
        self.get_as(&self.token, path, query).await
    }

    pub async fn get_as(&self, token: &str, path: &str, query: &[(&str, &str)]) -> TestResponse {
        let mut request = self
            .t
            .server
            .get(path)
            .add_header("Authorization", token.to_string());
        for (name, value) in query {
            request = request.add_query_param(name, value);
        }
        request.await
    }

    pub async fn delete(&self, path: &str) -> TestResponse {
        self.delete_as(&self.token, path).await
    }

    pub async fn delete_as(&self, token: &str, path: &str) -> TestResponse {
        self.t
            .server
            .delete(path)
            .add_header("Authorization", token.to_string())
            .await
    }

    /// What synchronisation hands a user for one table.
    pub async fn synchronised(&self, token: &str, table: &str) -> Vec<Value> {
        let response = self
            .t
            .server
            .get(&format!("/api/{table}/sync"))
            .add_header("Authorization", token.to_string())
            .await;
        response.json::<Value>()["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub async fn project(&self, folder: &TempDir) -> project::Model {
        self.post(
            "/api/projects",
            json!({ "name": "Sample", "directory": folder.path() }),
        )
        .await
        .json()
    }

    pub async fn thread(&self, project: &project::Model, agent: &str) -> thread::Model {
        self.post(
            &format!("/api/projects/{}/threads", project.id),
            json!({ "agent": agent }),
        )
        .await
        .json()
    }

    pub async fn say(&self, thread: &thread::Model, text: &str) -> TestResponse {
        self.post(
            &format!("/api/threads/{}/messages", thread.id),
            json!({ "text": text }),
        )
        .await
    }

    pub async fn stop(&self, thread: &thread::Model) -> TestResponse {
        self.post(&format!("/api/threads/{}/stop", thread.id), json!({}))
            .await
    }

    pub async fn see(&self, thread: &thread::Model) -> TestResponse {
        self.post(&format!("/api/threads/{}/seen", thread.id), json!({}))
            .await
    }

    pub async fn thread_now(&self, thread: &thread::Model) -> thread::Model {
        thread::Entity::find_by_id(thread.id)
            .one(&self.t.db)
            .await
            .expect("a database answer")
            .expect("the thread")
    }

    pub async fn mark_running(&self, thread: &thread::Model) {
        let mut thread = thread.clone().into_active_model();
        thread.status = Set(thread::RUNNING.to_string());
        thread
            .update(&self.t.db)
            .await
            .expect("the thread is updated");
    }

    /// Waits for the agent of a thread to end. Fails when it takes too long.
    pub async fn wait_until_idle(&self, thread: &thread::Model) {
        for _ in 0..200 {
            if self.thread_now(thread).await.status == thread::IDLE {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!("the agent did not end in time");
    }

    /// Gives a running agent's first lines time to be read.
    ///
    /// For checking that a line changed nothing, where there is no change to wait for.
    pub async fn let_the_report_be_read(&self) {
        sleep(Duration::from_millis(400)).await;
    }

    /// Waits for the agent of a thread to report its first line.
    ///
    /// Stopping an agent before it has got going proves little, so tests that
    /// stop one wait for this first.
    pub async fn wait_until_reporting(&self, thread: &thread::Model) {
        for _ in 0..200 {
            if self.thread_now(thread).await.conversation_started {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!("the agent did not report anything in time");
    }

    pub async fn entries(&self, thread_id: Uuid) -> Vec<thread_entry::Model> {
        thread_entry::Entity::find()
            .filter(thread_entry::Column::ThreadId.eq(thread_id))
            .order_by_asc(thread_entry::Column::Position)
            .all(&self.t.db)
            .await
            .expect("a database answer")
    }

    /// The kinds of a thread's entries, in order.
    pub async fn kinds(&self, thread: &thread::Model) -> Vec<String> {
        self.entries(thread.id)
            .await
            .into_iter()
            .map(|entry| entry.kind)
            .collect()
    }

    /// The text of the entry at a position, counted from 1.
    pub async fn text_at(&self, thread: &thread::Model, position: usize) -> String {
        self.entries(thread.id).await[position - 1].body["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}
