//! Why a workspace request was refused.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use sea_orm::DbErr;
use serde_json::json;
use thiserror::Error;
use tracing::error;

use crate::files::FilesError;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// What was sent cannot be accepted; the text says why.
    #[error("{0}")]
    Invalid(String),
    /// Nothing with that identifier belongs to the signed-in user.
    #[error("Not found.")]
    NotFound,
    /// The request is fine but cannot be done right now; the text says why.
    #[error("{0}")]
    Busy(&'static str),
    #[error("Something went wrong.")]
    Database(#[from] DbErr),
    /// The file browser could not answer; the text says why.
    #[error("{0}")]
    Files(#[from] FilesError),
}

impl IntoResponse for WorkspaceError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Busy(_) => StatusCode::CONFLICT,
            Self::Database(problem) => {
                error!("a workspace request failed: {problem}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Files(FilesError::NotARepository) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Files(FilesError::UnknownBranch | FilesError::UnknownFile) => {
                StatusCode::NOT_FOUND
            }
            Self::Files(FilesError::Git(problem)) => {
                error!("the file browser could not read a repository: {problem}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "The repository could not be read." })),
                )
                    .into_response();
            }
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
