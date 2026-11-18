//! Where projects and threads are kept: plain files, no database.
//!
//! Teryon on your own machine must not need a database server. What it
//! remembers is small and mostly only grows: a list of projects, and for each
//! thread a few facts and the conversation. Plain files are enough, can be
//! read and mended by hand, and deleting them is a complete reset.
//!
//! The list of projects lives in Teryon's data folder. Everything about a
//! project's threads lives inside the project, in `.teryon/threads/`, next to
//! the worktrees agents create in `.teryon/worktrees/`, so removing a project
//! directory removes everything Teryon knew about it.

pub mod projects;
pub mod threads;

use std::fs::{rename, write};
use std::io::Error as IoError;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

use crate::changes::Change;

/// Why something could not be read or stored.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Could not use {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("Could not read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// Another Teryon service has the project open.
    #[error("Another Teryon is using this project.")]
    InUse,
    #[error("There is no such thread.")]
    UnknownThread,
}

impl StoreError {
    fn io(path: &Path) -> impl FnOnce(IoError) -> Self + '_ {
        move |source| Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// A directory agents work in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    /// Where the project is, as its real location.
    pub directory: PathBuf,
    pub created_at: DateTime<Utc>,
}

/// Whether an agent is working in a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting for a message.
    Idle,
    /// An agent program is running for this thread.
    Running,
}

/// One conversation with an agent inside a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Thread {
    pub id: Uuid,
    /// The project the thread belongs to.
    ///
    /// Not trusted from the file: a project removed and added again gets a
    /// new identifier, and its threads follow it. The store sets it on reading.
    #[serde(default)]
    pub project_id: Uuid,
    pub title: String,
    pub agent: String,
    /// The identifier the agent program knows this conversation by.
    pub conversation_id: Uuid,
    /// Whether the agent program has seen the conversation yet.
    ///
    /// The programs take a different option for starting a conversation than
    /// for continuing one, and refuse the wrong one.
    pub conversation_started: bool,
    pub status: Status,
    /// Whether an agent stopped working here since the person last looked.
    pub unread: bool,
    /// The slash commands the agent program last said it takes.
    ///
    /// Empty until the program has run once for this thread.
    pub commands: Vec<String>,
    /// The change the thread's agent is working on, in a copy of the project of its own.
    ///
    /// Nothing until the agent starts one. Threads stored before changes had
    /// names have none either, and their copy is taken over, see
    /// `crate::changes::claim`. Boxed, because it is large next to the rest,
    /// and a thread travels in most messages.
    #[serde(default)]
    pub change: Option<Box<Change>>,
    pub created_at: DateTime<Utc>,
}

/// One line of a thread: a message, something the agent did, or a remark from Teryon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Entry {
    pub id: Uuid,
    pub thread_id: Uuid,
    /// Order within the thread, starting at 1.
    pub position: u64,
    pub kind: String,
    pub body: Value,
    pub created_at: DateTime<Utc>,
}

/// Replaces a file in one step, so nobody ever reads half of it.
///
/// The text goes to a file next to it first, which is then renamed over the
/// old one. A rename within one folder replaces the old file at once on
/// every platform Teryon runs on.
pub fn replace_file(path: &Path, text: &str) -> Result<(), StoreError> {
    let mut fresh = path.as_os_str().to_owned();
    fresh.push(".new");
    let fresh = PathBuf::from(fresh);
    write(&fresh, text).map_err(StoreError::io(&fresh))?;
    rename(&fresh, path).map_err(StoreError::io(path))
}
