//! Projects, threads and their entries.

pub mod directory;
pub mod error;
pub mod handlers;
pub mod journal;
pub mod policy;
pub mod project;
pub mod recovery;
pub mod thread;
pub mod thread_entry;

use axum::routing::{delete, get, post};
use axum::{Extension, Router};
use erno::sync::delta::sync_delta;

use crate::agents::runner::Runner;
use crate::settings::Settings;
use crate::workspace::handlers::{
    create_project, create_thread, delete_project, send_message, stop_thread,
};
use crate::Teryon;

async fn health() -> &'static str {
    "OK"
}

/// Teryon's own routes, mounted by erno under `/api`.
///
/// The runner is created here because this is the one place erno hands the
/// application its state. Agents are stopped when erno announces shutdown.
pub fn router(app: Teryon) -> Router {
    let runner = Runner::default();
    stop_agents_on_shutdown(&app, runner.clone());

    Router::new()
        .route("/health", get(health))
        .route("/projects", post(create_project))
        .route(
            "/projects/sync",
            get(sync_delta::<project::Entity, Settings>),
        )
        .route("/projects/{id}", delete(delete_project))
        .route("/projects/{id}/threads", post(create_thread))
        .route("/threads/sync", get(sync_delta::<thread::Entity, Settings>))
        .route("/threads/{id}/messages", post(send_message))
        .route("/threads/{id}/stop", post(stop_thread))
        .route(
            "/thread-entries/sync",
            get(sync_delta::<thread_entry::Entity, Settings>),
        )
        .layer(Extension(runner))
        .with_state(app)
}

fn stop_agents_on_shutdown(app: &Teryon, runner: Runner) {
    let mut shutdown = app.shutdown.clone();
    tokio::spawn(async move {
        shutdown.recv().await;
        runner.stop_all();
    });
}
