//! Shared request-spec helpers. See the Erno Testing guide.
//!
//! Each test file compiles this module for itself and uses only part of it.
#![allow(dead_code)]

use std::fs::{create_dir_all, read_to_string};
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
