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

use crate::changes::{Change, Finish};

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
    /// Whether the title was chosen, by the person or by the agent.
    ///
    /// Until then the title is the start of the first message, which often
    /// tells threads apart poorly, and the agent is asked to name the thread.
    #[serde(default)]
    pub named: bool,
    pub agent: String,
    /// The model the agent runs on. Nothing leaves it to the agent program's own settings.
    ///
    /// Can change between messages; each turn's summary says which model did that turn.
    #[serde(default)]
    pub model: Option<String>,
    /// The identifier the agent program gave this conversation.
    ///
    /// Nothing until the program has started it: the program makes the
    /// identifier up and reports it, and the next message continues it.
    #[serde(default)]
    pub conversation: Option<String>,
    /// Whether the agent took the thread over from another agent and has not yet been told what was said.
    ///
    /// Its next message carries the thread so far, see
    /// `crate::agents::handover`. Cleared once its program names the new
    /// conversation, which then holds it.
    #[serde(default)]
    pub handover: bool,
    pub status: Status,
    /// Whether an agent stopped working here since the person last looked.
    pub unread: bool,
    /// Whether the thread's agent asked the person questions they haven't answered yet.
    ///
    /// Kept on the thread, so the thread list can show who waits for an
    /// answer without reading every thread's entries. Any message from the
    /// person clears it: they answered, or chose to say something else.
    #[serde(default)]
    pub asking: bool,
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
    /// How the thread's last change ended. Nothing while one is under way, or before the first.
    #[serde(default)]
    pub finished: Option<Finish>,
    /// The second agent, reviewing the change. Nothing until a review is asked for.
    #[serde(default)]
    pub reviewer: Option<Reviewer>,
    pub created_at: DateTime<Utc>,
}

/// The agent reviewing a thread's change, in a conversation of its own.
///
/// It never sees the author's conversation, only the change and what the
/// person passes on, which keeps its judgement its own. It stays with the
/// thread, so a second review remembers what the first one found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Reviewer {
    pub agent: String,
    /// The model the reviewer runs on. Nothing leaves it to the agent program's own settings.
    #[serde(default)]
    pub model: Option<String>,
    /// The identifier the agent program gave the reviewer's conversation. Nothing until it has started it.
    #[serde(default)]
    pub conversation: Option<String>,
    /// Whether the reviewer took over from another reviewer and has not yet been told what was said.
    #[serde(default)]
    pub handover: bool,
    /// Whether the turn running in the thread is the reviewer's.
    pub reviewing: bool,
    /// What the last review found. Nothing until one has ended.
    pub verdict: Option<Verdict>,
}

/// What a review found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Verdict {
    /// How many problems the reviewer counted. Nothing when it did not say.
    pub findings: Option<u32>,
    /// Whether the author has worked on the change since, so the review may no longer fit it.
    pub answered: bool,
    /// Whether the review ended before the reviewer finished it, so it may be missing findings.
    #[serde(default)]
    pub cut: bool,
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
