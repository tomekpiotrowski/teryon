//! The history screen: a branch's commits, and what each of them changed.
//!
//! Agents commit their work, and the person using Teryon wants to follow
//! what was done, by whom, and in which order, without opening a terminal.
//! This answers three questions about a project's git repository: which
//! commits a branch has, which files a commit changed, and what one of those
//! files looks like next to the version before.
//!
//! Like the file browser, see `crate::files`, nothing here is stored. Every
//! answer is read from the repository at the moment it is asked for, and
//! nothing here ever writes to it.

pub mod handlers;
pub mod log;
pub mod reader;
pub mod turns;

use serde::Serialize;
use uuid::Uuid;

use crate::files::Change;

/// The commits of a branch, newest first, with the branches there are to choose from.
#[derive(Debug, Serialize)]
pub struct History {
    pub branches: Vec<String>,
    /// The branch shown. Nothing when the project's directory is not on a branch.
    pub branch: Option<String>,
    /// The main branch, when the branch shown is another one.
    ///
    /// Nothing when the branch shown is the main branch itself, or when
    /// there is no main branch.
    pub base: Option<String>,
    /// How many commits the branch shown has that the main branch has not.
    pub ahead: usize,
    /// Whether the branch has older commits than the ones listed.
    pub more: bool,
    pub commits: Vec<Commit>,
}

/// One commit of a branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Commit {
    /// The commit's full name.
    pub hash: String,
    /// The commits this one was made on top of, the one it continues first.
    pub parents: Vec<String>,
    /// The first line of the message.
    pub subject: String,
    /// The name of the person the commit is recorded under.
    pub author: String,
    pub email: String,
    /// When the commit came to be, in seconds since 1970.
    ///
    /// This is what git calls the committer date. Unlike the author date it
    /// moves when a commit is made again by a rebase or a squash, which is
    /// what tells the commit apart from the work that was done before.
    pub time: i64,
    /// Whether the main branch has this commit too.
    pub on_base: bool,
    /// The thread whose agent was working when the commit was made, see `turns`.
    pub thread: Option<Uuid>,
    /// The files the commit changed, each from the top of the repository.
    pub files: Vec<String>,
}

/// What a commit changed, or what changed between two commits.
#[derive(Debug, Serialize)]
pub struct Changes {
    /// The commit looked at, by its full name.
    pub commit: String,
    /// The commit it is compared with. Nothing for a commit without a parent.
    pub from: Option<String>,
    /// The commit's message after its first line.
    pub body: String,
    pub files: Vec<ChangedFile>,
}

/// One file a commit changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChangedFile {
    pub path: String,
    pub change: Change,
    /// How many lines were added and removed. Nothing for a file that is not text.
    pub added: Option<u32>,
    pub removed: Option<u32>,
}
