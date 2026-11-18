//! The requests that change the workspace.
//!
//! Reading does not happen here: the application receives projects, threads
//! and entries through erno's synchronisation.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use erno::auth::current_user::CurrentUser;
use erno::sync::syncable::Syncable;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, Set, Statement,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::agents::runner::{Runner, TurnRequest};
use crate::agents::{Agent, EntryDraft};
use crate::workspace::directory::resolve_directory;
use crate::workspace::error::WorkspaceError;
use crate::workspace::journal::Journal;
use crate::workspace::{project, thread, thread_entry};
use crate::Teryon;

/// The longest title taken from a first message.
const LONGEST_TITLE: usize = 60;

#[derive(Deserialize)]
pub struct NewProject {
    name: String,
    directory: String,
}

#[derive(Deserialize)]
pub struct NewThread {
    agent: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
pub struct NewMessage {
    text: String,
}

pub async fn create_project(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Json(new): Json<NewProject>,
) -> Result<(StatusCode, Json<project::Model>), WorkspaceError> {
    let name = new.name.trim();
    if name.is_empty() {
        return Err(WorkspaceError::Invalid(
            "A project needs a name.".to_string(),
        ));
    }
    let directory = resolve_directory(&app.config.extra.agents.allowed_folders, &new.directory)
        .map_err(|problem| WorkspaceError::Invalid(problem.to_string()))?;

    let project = project::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user.id),
        name: Set(name.to_string()),
        directory: Set(directory.to_string_lossy().into_owned()),
        created_at: Set(Utc::now().naive_utc()),
        ..Default::default()
    }
    .insert(&app.db)
    .await?;

    Ok((StatusCode::CREATED, Json(project)))
}

/// Removes a project from Teryon. Its directory is left untouched.
///
/// The threads and entries go with it, as rows marked deleted, so that the
/// application learns about the removal.
pub async fn delete_project(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
) -> Result<StatusCode, WorkspaceError> {
    let project = owned_project(&app.db, user.id, project_id).await?;

    let running = thread::Entity::find()
        .filter(thread::Column::ProjectId.eq(project.id))
        .filter(thread::Column::DeletedAt.is_null())
        .filter(thread::Column::Status.eq(thread::RUNNING))
        .count(&app.db)
        .await?;
    if running > 0 {
        return Err(WorkspaceError::Busy(
            "An agent is still running in this project.",
        ));
    }

    for statement in [
        "UPDATE thread_entries SET deleted_at = NOW() WHERE deleted_at IS NULL \
         AND thread_id IN (SELECT id FROM threads WHERE project_id = $1)",
        "UPDATE threads SET deleted_at = NOW() WHERE deleted_at IS NULL AND project_id = $1",
    ] {
        app.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                statement,
                [project.id.into()],
            ))
            .await?;
    }
    project::Entity::soft_delete_by_id(project.id)
        .exec(&app.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_thread(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
    Json(new): Json<NewThread>,
) -> Result<(StatusCode, Json<thread::Model>), WorkspaceError> {
    let project = owned_project(&app.db, user.id, project_id).await?;
    let agent = Agent::parse(&new.agent).ok_or_else(|| {
        WorkspaceError::Invalid("Choose Claude or Grok as the agent.".to_string())
    })?;

    let thread = thread::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user.id),
        project_id: Set(project.id),
        title: Set(new.title.trim().to_string()),
        agent: Set(agent.name().to_string()),
        conversation_id: Set(Uuid::new_v4()),
        conversation_started: Set(false),
        status: Set(thread::IDLE.to_string()),
        unread: Set(false),
        commands: Set(json!([])),
        created_at: Set(Utc::now().naive_utc()),
        ..Default::default()
    }
    .insert(&app.db)
    .await?;

    Ok((StatusCode::CREATED, Json(thread)))
}

/// Stores a message and starts the thread's agent on it.
///
/// The thread is claimed with a single update that only succeeds while it is
/// idle, so two messages sent at the same moment cannot both start an agent.
pub async fn send_message(
    State(app): State<Teryon>,
    Extension(runner): Extension<Runner>,
    CurrentUser { user, .. }: CurrentUser,
    Path(thread_id): Path<Uuid>,
    Json(new): Json<NewMessage>,
) -> Result<(StatusCode, Json<thread_entry::Model>), WorkspaceError> {
    let thread = owned_thread(&app.db, user.id, thread_id).await?;
    let project = owned_project(&app.db, user.id, thread.project_id).await?;
    let text = new.text.trim();
    if text.is_empty() {
        return Err(WorkspaceError::Invalid(
            "Write a message first.".to_string(),
        ));
    }

    let claimed = thread::Entity::update_many()
        .col_expr(thread::Column::Status, Expr::value(thread::RUNNING))
        .filter(thread::Column::Id.eq(thread.id))
        .filter(thread::Column::Status.eq(thread::IDLE))
        .exec(&app.db)
        .await?;
    if claimed.rows_affected != 1 {
        return Err(WorkspaceError::Busy(
            "The agent is still working on the previous message.",
        ));
    }

    let entry = match store_message(&app.db, &thread, text).await {
        Ok(entry) => entry,
        Err(problem) => {
            release(&app.db, thread).await;
            return Err(problem);
        }
    };

    // Read the thread again: the runner needs it as it is now, claimed and titled.
    let thread = owned_thread(&app.db, user.id, thread_id).await?;
    runner.start(
        app.db.clone(),
        app.config.extra.agents.clone(),
        TurnRequest {
            thread,
            directory: project.directory.into(),
            message: text.to_string(),
        },
    );

    Ok((StatusCode::ACCEPTED, Json(entry)))
}

pub async fn stop_thread(
    State(app): State<Teryon>,
    Extension(runner): Extension<Runner>,
    CurrentUser { user, .. }: CurrentUser,
    Path(thread_id): Path<Uuid>,
) -> Result<StatusCode, WorkspaceError> {
    let thread = owned_thread(&app.db, user.id, thread_id).await?;
    runner.stop(thread.id);
    Ok(StatusCode::NO_CONTENT)
}

/// Notes that the person has looked at a thread.
///
/// A thread is marked as unread when its agent stops. The application calls
/// this when the thread is on screen, which clears the mark on every device.
/// A thread that is not marked is left alone, so that looking at it again
/// does not send a change to every device.
pub async fn mark_thread_seen(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(thread_id): Path<Uuid>,
) -> Result<StatusCode, WorkspaceError> {
    let thread = owned_thread(&app.db, user.id, thread_id).await?;
    if thread.unread {
        let mut thread = thread.into_active_model();
        thread.unread = Set(false);
        thread.update(&app.db).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Appends the user's message and names an untitled thread after it.
async fn store_message(
    db: &DatabaseConnection,
    thread: &thread::Model,
    text: &str,
) -> Result<thread_entry::Model, WorkspaceError> {
    let entry = Journal::open(db, thread)
        .await?
        .add(EntryDraft::user_message(text))
        .await?;

    if thread.title.is_empty() {
        let mut titled = thread.clone().into_active_model();
        titled.title = Set(title_from(text));
        titled.update(db).await?;
    }
    Ok(entry)
}

/// Gives a claimed thread back when its agent could not be started.
async fn release(db: &DatabaseConnection, thread: thread::Model) {
    let mut thread = thread.into_active_model();
    thread.status = Set(thread::IDLE.to_string());
    if let Err(problem) = thread.update(db).await {
        tracing::error!("could not release a thread: {problem}");
    }
}

/// The start of a message's first line, short enough for the side list.
fn title_from(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= LONGEST_TITLE {
        line.to_string()
    } else {
        let start: String = line.chars().take(LONGEST_TITLE).collect();
        format!("{}…", start.trim_end())
    }
}

pub(crate) async fn owned_project(
    db: &DatabaseConnection,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<project::Model, WorkspaceError> {
    project::Entity::find_by_id(project_id)
        .filter(project::Column::UserId.eq(user_id))
        .filter(project::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(WorkspaceError::NotFound)
}

async fn owned_thread(
    db: &DatabaseConnection,
    user_id: Uuid,
    thread_id: Uuid,
) -> Result<thread::Model, WorkspaceError> {
    thread::Entity::find_by_id(thread_id)
        .filter(thread::Column::UserId.eq(user_id))
        .filter(thread::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(WorkspaceError::NotFound)
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        // requirement: a thread's title is the start of its first message, unless one was given
        expect(title_from(&text)) {
            when(text = "Fix the login bug".to_string()) as a_short_message to be_the_title {
                equal("Fix the login bug".to_string())
            }
            when(text = "First line\nSecond line".to_string()) as several_lines to use_the_first {
                equal("First line".to_string())
            }
            when(text = "é".repeat(60)) as exactly_the_longest_title to be_kept_whole {
                equal("é".repeat(60))
            }
            when(text = "é".repeat(61)) as a_longer_line to be_shortened {
                equal(format!("{}…", "é".repeat(60)))
            }
            when(text = format!("{} tail", "a".repeat(59))) as a_cut_at_a_space to drop_the_space {
                equal(format!("{}…", "a".repeat(59)))
            }
        }
    }
}
