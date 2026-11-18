//! A thread's change: the work its agent does in a copy of the project of its own.
//!
//! Several agents can work in one project at once without touching each
//! other or the person's own folder, because each changes files in its own
//! copy. Underneath, a copy is a git worktree in `.teryon/worktrees/<name>`,
//! on a branch with the same name. The agent picks the name when it starts
//! changing files, through `teryon worktree create`, and Teryon creates the
//! worktree and remembers which thread it belongs to.
//!
//! Nothing a person reads about a change uses git's words. They see the
//! thread's copy, the change and its name, how many files changed, and
//! whether it would apply to the project or overlaps with newer work in it.
//! The words exist only on screen; here, and for agents, it is all git.
//!
//! What git says about a change is read again whenever it can have changed,
//! see [`status`], and stored on the thread, so every screen sees the same.

pub mod claim;
pub mod create;
pub mod instruction;
pub mod name;
pub mod open;
pub mod status;

use std::fs::canonicalize;
use std::io::Error as IoError;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::changes::name::BadName;
use crate::files::git::{checkouts, Git, GitError};

/// Why a change could not be started. The text is meant for the agent, so it may use git's words.
#[derive(Debug, Error)]
pub enum ChangeError {
    #[error(transparent)]
    BadName(#[from] BadName),
    #[error("There is already a worktree or a branch called `{name}`. Pick another name, such as `{suggestion}`.")]
    NameTaken { name: String, suggestion: String },
    #[error("There is no branch called `{0}` to start from.")]
    UnknownBranch(String),
    #[error("This project is not a git repository, so there is nowhere to create a worktree.")]
    NotARepository,
    #[error("This repository has no commits yet. Make a first commit before creating a worktree.")]
    NoCommits,
    #[error("Could not use the worktree's files: {0}")]
    Files(#[from] IoError),
    #[error(transparent)]
    Git(#[from] GitError),
}

/// The work a thread's agent is doing in its own copy of the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Change {
    /// The worktree folder's name in `.teryon/worktrees/`, also its branch.
    pub name: String,
    /// The worktree's full path.
    #[ts(type = "string")]
    pub path: PathBuf,
    /// The branch the change started from, and is applied to. Nothing when
    /// the project was not on a branch when the change started.
    pub base: Option<String>,
    /// The commit the change started from.
    pub start: String,
    /// The branch checked out in the worktree now, read from git. Usually `name`.
    pub branch: Option<String>,
    /// Nothing until first read.
    pub state: Option<ChangeState>,
}

/// Where a change stands, as last read from git.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChangeState {
    /// Whether the worktree is still there.
    pub copy_present: bool,
    /// Whether the branch the change started from is still there.
    pub base_present: bool,
    /// Files differing between the worktree as it is now and where the change
    /// started, whether the agent committed them or not.
    pub files_changed: u32,
    pub fit: Fit,
    /// The agent left the worktree in the middle of a rebase or a merge, or
    /// switched it to another branch. Something only the agent can sort out.
    pub unfinished: bool,
}

impl ChangeState {
    /// The state of a change whose worktree has gone.
    pub fn gone() -> Self {
        Self {
            copy_present: false,
            base_present: false,
            files_changed: 0,
            fit: Fit::Unknown,
            unfinished: false,
        }
    }
}

/// Whether a change would apply to the project as it is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// It merges without conflicts.
    Applies,
    /// It conflicts with commits added to its base since it started.
    Overlaps,
    /// Not checked yet, or it cannot be: the base is gone, or git is too old.
    Unknown,
}

/// The folder all copies go in, inside the project.
pub fn copies_folder(project: &Path) -> PathBuf {
    project.join(".teryon").join("worktrees")
}

/// A worktree in the copies folder: its folder's name, its path, and the branch checked out there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundCopy {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// Every worktree of the project that sits directly in its copies folder.
///
/// Worktrees elsewhere are the person's own business and are never claimed.
pub async fn copies_in(project: &Path) -> Result<Vec<FoundCopy>, GitError> {
    let Ok(folder) = canonicalize(copies_folder(project)) else {
        return Ok(Vec::new());
    };
    let listed = Git::at(project)
        .output(&["worktree", "list", "--porcelain", "-z"])
        .await?;
    Ok(checkouts(&listed)
        .into_iter()
        .filter_map(|checkout| {
            let path = canonicalize(&checkout.directory).ok()?;
            if path.parent() != Some(folder.as_path()) {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(FoundCopy {
                name,
                path,
                branch: checkout.branch,
            })
        })
        .collect())
}
