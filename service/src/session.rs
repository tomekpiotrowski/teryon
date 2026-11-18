//! One client's conversation with the service, whatever carries it.
//!
//! A session answers requests through the workspace, and decides which
//! changes the client hears about: every change to projects and threads, and
//! the new entries of the threads it follows. Today every session comes over
//! a local connection; later a Teryon server will open sessions on a
//! person's behalf, and those are told apart by [`Origin`] because they may
//! do less.

use std::collections::HashSet;

use uuid::Uuid;

use teryon_core::protocol::{Answer, Event, Request};
use teryon_core::workspace::Workspace;

/// Where a session comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A program on this machine that showed the service's token.
    Local,
    /// A browser or phone, through a Teryon server.
    ///
    /// It may not add or remove projects, or a person signed in to the server
    /// could make any folder of this machine a project from anywhere.
    Relayed,
}

/// What a session did besides answering.
#[derive(Debug, PartialEq, Eq)]
pub enum Aftermath {
    Nothing,
    /// The client asked the service to stop.
    ShutDown,
}

pub struct Session {
    workspace: Workspace,
    origin: Origin,
    following: HashSet<Uuid>,
}

impl Session {
    pub fn new(workspace: Workspace, origin: Origin) -> Self {
        Self {
            workspace,
            origin,
            following: HashSet::new(),
        }
    }

    /// Answers a request. The error is a message meant for the person.
    pub async fn answer(&mut self, request: Request) -> (Result<Answer, String>, Aftermath) {
        let workspace = &self.workspace;
        let local = self.origin == Origin::Local;
        let answer = match request {
            Request::AddProject { .. } | Request::RemoveProject { .. } | Request::Shutdown
                if !local =>
            {
                Err("This can only be done on the machine itself.".to_string())
            }
            Request::Shutdown => return (Ok(Answer::Nothing), Aftermath::ShutDown),
            Request::AddProject { name, directory } => workspace
                .add_project(&name, &directory)
                .map(|project| Answer::Project { project })
                .map_err(|problem| problem.to_string()),
            Request::RemoveProject { project_id } => workspace
                .remove_project(project_id)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::StartThread {
                project_id,
                agent,
                title,
            } => workspace
                .start_thread(project_id, &agent, &title)
                .map(|thread| Answer::Thread { thread })
                .map_err(|problem| problem.to_string()),
            Request::SendMessage { thread_id, text } => workspace
                .send_message(thread_id, &text)
                .map(|entry| Answer::Entry { entry })
                .map_err(|problem| problem.to_string()),
            Request::Stop { thread_id } => workspace
                .stop(thread_id)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::MarkSeen { thread_id } => workspace
                .mark_seen(thread_id)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::FollowThread { thread_id } => {
                // Followed before reading, so no entry added in between is missed.
                // One may then arrive twice; clients keep entries by identifier.
                self.following.insert(thread_id);
                let entries = self.workspace.entries(thread_id);
                if entries.is_err() {
                    self.following.remove(&thread_id);
                }
                entries
                    .map(|entries| Answer::Entries { entries })
                    .map_err(|problem| problem.to_string())
            }
            Request::UnfollowThread { thread_id } => {
                self.following.remove(&thread_id);
                Ok(Answer::Nothing)
            }
            Request::Files {
                project_id,
                branch,
                commit,
            } => workspace
                .files(project_id, branch.as_deref(), commit.as_deref())
                .await
                .map(|listing| Answer::Listing { listing })
                .map_err(|problem| problem.to_string()),
            Request::File {
                project_id,
                branch,
                commit,
                path,
            } => workspace
                .file(project_id, branch.as_deref(), commit.as_deref(), &path)
                .await
                .map(|file| Answer::File { file })
                .map_err(|problem| problem.to_string()),
            Request::History { project_id, branch } => workspace
                .history(project_id, branch.as_deref())
                .await
                .map(|history| Answer::History { history })
                .map_err(|problem| problem.to_string()),
            Request::Changes {
                project_id,
                commit,
                from,
            } => workspace
                .changes(project_id, &commit, from.as_deref())
                .await
                .map(|changes| Answer::Changes { changes })
                .map_err(|problem| problem.to_string()),
            Request::Change {
                project_id,
                commit,
                from,
                path,
            } => workspace
                .change(project_id, &commit, from.as_deref(), &path)
                .await
                .map(|file| Answer::File { file })
                .map_err(|problem| problem.to_string()),
        };
        (answer, Aftermath::Nothing)
    }

    /// Whether the client hears about a change.
    pub fn wants(&self, event: &Event) -> bool {
        match event {
            Event::EntryAdded { entry } => self.following.contains(&entry.thread_id),
            _ => true,
        }
    }
}
