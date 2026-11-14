//! The requests the history screen answers.
//!
//! All three only read, and all are asked again and again while the screen
//! is open, which is how a commit an agent just made shows up.

use axum::extract::{Path, Query, State};
use axum::Json;
use erno::auth::current_user::CurrentUser;
use serde::Deserialize;
use uuid::Uuid;

use crate::files::handlers::open;
use crate::files::FileView;
use crate::history::turns::{turns_of, working_at};
use crate::history::{Changes, History};
use crate::workspace::error::WorkspaceError;
use crate::Teryon;

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// Left out to see the history of whatever the project's directory is on.
    branch: Option<String>,
}

#[derive(Deserialize)]
pub struct ChangesQuery {
    commit: String,
    /// Left out to compare the commit with the one it continues.
    from: Option<String>,
}

#[derive(Deserialize)]
pub struct ChangeQuery {
    commit: String,
    from: Option<String>,
    path: String,
}

/// The commits of one of the project's branches, each with the thread it was made in.
pub async fn list_history(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<History>, WorkspaceError> {
    let (project, repository) = open(&app, user.id, project_id).await?;
    let mut history = repository.history(query.branch.as_deref()).await?;
    let turns = turns_of(&app.db, project.id).await?;
    for commit in &mut history.commits {
        commit.thread = working_at(&turns, commit.time);
    }
    Ok(Json(history))
}

/// The files a commit changed, or what changed between two commits.
pub async fn show_changes(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<Changes>, WorkspaceError> {
    let (_, repository) = open(&app, user.id, project_id).await?;
    Ok(Json(
        repository
            .changes(&query.commit, query.from.as_deref())
            .await?,
    ))
}

/// One file a commit changed, line by line, with its changes marked.
pub async fn show_change(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
    Query(query): Query<ChangeQuery>,
) -> Result<Json<FileView>, WorkspaceError> {
    let (_, repository) = open(&app, user.id, project_id).await?;
    Ok(Json(
        repository
            .change(&query.commit, query.from.as_deref(), &query.path)
            .await?,
    ))
}
