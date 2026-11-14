//! The requests the file browser answers.
//!
//! Both only read, and both are asked again and again while the screen is
//! open, which is how changes made by an agent show up.

use axum::extract::{Path, Query, State};
use axum::Json;
use erno::auth::current_user::CurrentUser;
use serde::Deserialize;
use uuid::Uuid;

use crate::files::repository::{Place, Repository};
use crate::files::{FileView, Listing};
use crate::workspace::error::WorkspaceError;
use crate::workspace::handlers::owned_project;
use crate::workspace::project;
use crate::Teryon;

#[derive(Deserialize)]
pub struct ListingQuery {
    /// Left out, together with the commit, to see the project's directory as it is.
    branch: Option<String>,
    /// A commit to show in place of a branch.
    commit: Option<String>,
}

#[derive(Deserialize)]
pub struct FileQuery {
    branch: Option<String>,
    commit: Option<String>,
    path: String,
}

/// Every file of one of the project's branches or commits, and the branches to choose from.
pub async fn list_files(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
    Query(query): Query<ListingQuery>,
) -> Result<Json<Listing>, WorkspaceError> {
    let (_, repository) = open(&app, user.id, project_id).await?;
    let place = Place::named(query.branch.as_deref(), query.commit.as_deref());
    Ok(Json(repository.listing(place).await?))
}

/// One file of a branch or a commit, line by line, with its changes marked.
pub async fn show_file(
    State(app): State<Teryon>,
    CurrentUser { user, .. }: CurrentUser,
    Path(project_id): Path<Uuid>,
    Query(query): Query<FileQuery>,
) -> Result<Json<FileView>, WorkspaceError> {
    let (_, repository) = open(&app, user.id, project_id).await?;
    let place = Place::named(query.branch.as_deref(), query.commit.as_deref());
    Ok(Json(repository.file(place, &query.path).await?))
}

/// One of the signed-in person's projects, and its repository.
pub(crate) async fn open(
    app: &Teryon,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<(project::Model, Repository), WorkspaceError> {
    let project = owned_project(&app.db, user_id, project_id).await?;
    let repository = Repository::open(
        project.directory.as_ref(),
        &app.config.extra.agents.allowed_folders,
    )
    .await?;
    Ok((project, repository))
}
