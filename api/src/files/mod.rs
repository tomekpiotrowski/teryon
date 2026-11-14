//! The file browser: a project's files, and what changed in them.
//!
//! Agents change files, and the person using Teryon wants to see what they
//! did without opening a terminal. This reads a project's git repository and
//! answers two questions: which files are there on a branch, and what does
//! one of them look like next to the version it started from.
//!
//! Nothing here is stored. Files change under an agent's hands, so every
//! answer is read from the repository at the moment it is asked for.

pub mod diff;
pub mod git;
pub mod handlers;
pub mod repository;

use serde::Serialize;
use thiserror::Error;

use crate::files::diff::Line;
use crate::files::git::GitError;

/// Why the file browser could not answer.
#[derive(Debug, Error)]
pub enum FilesError {
    #[error("The file browser needs a git repository, and this project's directory is not one.")]
    NotARepository,
    #[error("There is no such branch.")]
    UnknownBranch,
    #[error("There is no such file.")]
    UnknownFile,
    #[error(transparent)]
    Git(#[from] GitError),
}

/// What happened to a file since the version it is compared with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Modified,
    Deleted,
}

/// One file of a branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    /// Where the file is, from the top of the repository, with `/` between folders.
    pub path: String,
    /// Nothing for a file that did not change.
    pub change: Option<Change>,
}

/// Every file of a branch, together with the branches there are to choose from.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub branches: Vec<String>,
    /// The branch shown. Nothing when the project's directory is not on a branch.
    pub branch: Option<String>,
    /// The branch the changes are counted from.
    ///
    /// Nothing when the branch shown is that branch itself, or when there is
    /// no such branch. Then the only changes are the ones not committed yet.
    pub base: Option<String>,
    pub files: Vec<FileEntry>,
}

/// What a file holds.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    Text {
        lines: Vec<Line>,
    },
    /// Not text, so there are no lines to show.
    Binary,
    /// Text perhaps, but more than the screen should be asked to show.
    TooLarge,
}

/// One file, ready to show.
#[derive(Debug, Serialize)]
pub struct FileView {
    pub path: String,
    pub content: Content,
}
