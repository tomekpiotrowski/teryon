//! One client's conversation with the service, whatever carries it.
//!
//! A session answers requests through the workspace, and decides which
//! changes the client hears about: every change to projects and threads, and
//! the new entries of the threads it follows. Today every session comes over
//! a local connection; later a Teryon server will open sessions on a
//! person's behalf, and those are told apart by [`Origin`] because they may
//! do less.

use std::collections::HashSet;

use tokio::spawn;
use tracing::{info, warn};
use uuid::Uuid;

use teryon_core::changes::finish::CaughtUp;
use teryon_core::directory::browse;
use teryon_core::protocol::{Answer, Event, Request};
use teryon_core::workspace::{Actor, Workspace};

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
    actor: Actor,
    following: HashSet<Uuid>,
}

impl Session {
    pub fn new(workspace: Workspace, origin: Origin, actor: Actor) -> Self {
        Self {
            workspace,
            origin,
            actor,
            following: HashSet::new(),
        }
    }

    /// Answers a request. The error is a message meant for the person.
    pub async fn answer(&mut self, request: Request) -> (Result<Answer, String>, Aftermath) {
        let workspace = &self.workspace;
        let local = self.origin == Origin::Local;
        let actor = self.actor;
        let audit = match actor {
            Actor::Thread(acting) => audit_info(&request).map(|(op, target)| (acting, op, target)),
            Actor::Person => None,
        };
        if let Some((acting, operation, target)) = audit {
            if target == Some(acting) && refuses_own_thread(&request) {
                let message = "An agent cannot act on its own thread with this command. Use `teryon thread title`, `teryon thread stage`, `teryon thread ask`, or `teryon thread workflow` for your own thread.".to_string();
                warn!(%acting, target = %acting, operation, "refused request from an agent about its own thread");
                return (Err(message), Aftermath::Nothing);
            }
            info!(%acting, target = ?target, operation, "agent requested thread operation");
        }
        let answer = match request {
            Request::AddProject { .. }
            | Request::RemoveProject { .. }
            | Request::Directories { .. }
            | Request::OpenCopy { .. }
            | Request::OpenReview { .. }
            | Request::Configuration
            | Request::SaveConfiguration { .. }
            | Request::Shutdown
                if !local =>
            {
                Err("This can only be done on the machine itself.".to_string())
            }
            Request::Shutdown => return (Ok(Answer::Nothing), Aftermath::ShutDown),
            Request::AddProject { name, directory } => workspace
                .add_project(&name, &directory)
                .map(|project| Answer::Project { project })
                .map_err(|problem| problem.to_string()),
            Request::Directories { path } => browse(&path)
                .map(|listing| Answer::Directories { listing })
                .map_err(|problem| problem.to_string()),
            Request::RemoveProject { project_id } => workspace
                .remove_project(project_id)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::StartThread {
                project_id,
                agent,
                model,
                effort,
                title,
                workflow,
            } => workspace
                .start_thread_on(
                    project_id,
                    agent.as_deref(),
                    model.as_deref(),
                    effort,
                    &title,
                    workflow.as_deref(),
                )
                .map(|thread| Answer::Thread {
                    thread: Box::new(thread),
                })
                .map_err(|problem| problem.to_string()),
            Request::SendMessage {
                thread_id,
                text,
                to,
            } => workspace
                .send_message_by(thread_id, &text, to, actor)
                .map(|entry| Answer::Entry { entry })
                .map_err(|problem| problem.to_string()),
            Request::ReviewChange {
                thread_id,
                agent,
                model,
                effort,
            } => workspace
                .review_change_by(thread_id, &agent, model.as_deref(), effort, actor)
                .map(|entry| Answer::Entry { entry })
                .map_err(|problem| problem.to_string()),
            Request::ChangeAgent {
                thread_id,
                to,
                agent,
                model,
                effort,
            } => workspace
                .change_agent_by(thread_id, to, &agent, model.as_deref(), effort, actor)
                .map(|thread| Answer::Thread {
                    thread: Box::new(thread),
                })
                .map_err(|problem| problem.to_string()),
            Request::Stop { thread_id } => workspace
                .stop_by(thread_id, actor)
                .map(|stopped| Answer::Stopped { stopped })
                .map_err(|problem| problem.to_string()),
            Request::MarkSeen { thread_id } => workspace
                .mark_seen(thread_id)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::PutAwayQuiet { project_id } => workspace
                .put_away_quiet(project_id)
                .map(|count| Answer::PutAway { count })
                .map_err(|problem| problem.to_string()),
            Request::AcknowledgeDone { seen } => workspace
                .acknowledge_done(&seen)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::StartChange {
                thread_id,
                turn,
                name,
                from,
            } => workspace
                .start_change(thread_id, turn, &name, from.as_deref())
                .await
                .map(|path| Answer::Path {
                    path: path.to_string_lossy().into_owned(),
                })
                .map_err(|problem| problem.to_string()),
            Request::NameThread {
                thread_id,
                turn,
                title,
            } => workspace
                .name_thread(thread_id, turn, &title)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::ReportStage {
                thread_id,
                turn,
                stage,
            } => workspace
                .report_stage(thread_id, turn, &stage)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::ChooseWorkflow {
                thread_id,
                turn,
                workflow,
            } => workspace
                .choose_workflow_by(thread_id, turn, &workflow, actor)
                .map(|thread| Answer::Thread {
                    thread: Box::new(thread),
                })
                .map_err(|problem| problem.to_string()),
            Request::AskQuestions {
                thread_id,
                turn,
                asked,
            } => workspace
                .ask_questions(thread_id, turn, &asked)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::DraftDescription { thread_id } => workspace
                .draft_description(thread_id)
                .await
                .map(|text| Answer::Draft { text })
                .map_err(|problem| problem.to_string()),
            Request::ApplyChange {
                thread_id,
                description,
            } => workspace
                .apply_change_by(thread_id, &description, actor)
                .await
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::CatchUp { thread_id } => workspace
                .catch_up_by(thread_id, actor)
                .await
                .map(|caught_up| Answer::CaughtUp {
                    overlaps: caught_up == CaughtUp::Overlaps,
                })
                .map_err(|problem| problem.to_string()),
            Request::DiscardChange { thread_id } => workspace
                .discard_change_by(thread_id, actor)
                .await
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::SendForReview {
                thread_id,
                description,
            } => workspace
                .send_for_review_by(thread_id, &description, actor)
                .await
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::OpenReview { thread_id } => workspace
                .open_review(thread_id)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::OpenCopy { thread_id, with } => workspace
                .open_copy(thread_id, with)
                .map(|()| Answer::Nothing)
                .map_err(|problem| problem.to_string()),
            Request::FollowThread { thread_id } => {
                // Followed before reading, so no entry added in between is missed.
                // One may then arrive twice; clients keep entries by identifier.
                self.following.insert(thread_id);
                let entries = self.workspace.entries(thread_id);
                if entries.is_err() {
                    self.following.remove(&thread_id);
                } else {
                    // Someone is looking: anything done to the copy outside
                    // Teryon shows up now. The answer does not wait for it.
                    let workspace = self.workspace.clone();
                    spawn(async move { workspace.refresh_change(thread_id).await });
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
                thread_id,
                branch,
                commit,
            } => workspace
                .files(project_id, thread_id, branch.as_deref(), commit.as_deref())
                .await
                .map(|listing| Answer::Listing { listing })
                .map_err(|problem| problem.to_string()),
            Request::File {
                project_id,
                thread_id,
                branch,
                commit,
                path,
            } => workspace
                .file(
                    project_id,
                    thread_id,
                    branch.as_deref(),
                    commit.as_deref(),
                    &path,
                )
                .await
                .map(|file| Answer::File { file })
                .map_err(|problem| problem.to_string()),
            Request::History { project_id, branch } => workspace
                .history(project_id, branch.as_deref())
                .await
                .map(|history| Answer::History { history })
                .map_err(|problem| problem.to_string()),
            Request::Plan { thread_id } => workspace
                .plan(thread_id)
                .map(|text| Answer::Plan { text })
                .map_err(|problem| problem.to_string()),
            Request::Usage => workspace
                .usage()
                .map(|report| Answer::Usage { report })
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
            Request::Configuration => Ok(Answer::Configuration {
                configuration: Box::new(workspace.configuration()),
            }),
            Request::SaveConfiguration { configuration } => workspace
                .save_configuration(&configuration)
                .map(|configuration| Answer::Configuration {
                    configuration: Box::new(configuration),
                })
                .map_err(|problem| problem.to_string()),
        };
        if let (Some((acting, operation, target)), Err(problem)) = (audit, &answer) {
            warn!(%acting, target = ?target, operation, %problem, "refused request from another thread");
        }
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

/// The operation name and target thread for logging cross-thread requests from an agent.
///
/// Own-thread turn commands (`StartChange`, `NameThread`, `ReportStage`,
/// `AskQuestions`, and `ChooseWorkflow` with a turn) are left out so they are
/// neither logged as cross-thread actions nor blocked as self-targeting.
fn audit_info(request: &Request) -> Option<(&'static str, Option<Uuid>)> {
    match request {
        Request::StartThread { .. } => Some(("start_thread", None)),
        Request::SendMessage { thread_id, .. } => Some(("send_message", Some(*thread_id))),
        Request::ReviewChange { thread_id, .. } => Some(("review_change", Some(*thread_id))),
        Request::ChangeAgent { thread_id, .. } => Some(("change_agent", Some(*thread_id))),
        Request::Stop { thread_id } => Some(("stop", Some(*thread_id))),
        Request::MarkSeen { thread_id } => Some(("mark_seen", Some(*thread_id))),
        Request::PutAwayQuiet { .. } => Some(("put_away_quiet", None)),
        Request::ChooseWorkflow {
            thread_id,
            turn: None,
            ..
        } => Some(("choose_workflow", Some(*thread_id))),
        Request::DraftDescription { thread_id } => Some(("draft_description", Some(*thread_id))),
        Request::ApplyChange { thread_id, .. } => Some(("apply_change", Some(*thread_id))),
        Request::CatchUp { thread_id } => Some(("catch_up", Some(*thread_id))),
        Request::DiscardChange { thread_id } => Some(("discard_change", Some(*thread_id))),
        Request::SendForReview { thread_id, .. } => Some(("send_for_review", Some(*thread_id))),
        Request::FollowThread { thread_id } => Some(("follow_thread", Some(*thread_id))),
        Request::Plan { thread_id } => Some(("plan", Some(*thread_id))),
        _ => None,
    }
}

/// Whether an agent acting for a thread must be refused from running `request` on that same thread.
fn refuses_own_thread(request: &Request) -> bool {
    matches!(
        request,
        Request::SendMessage { .. }
            | Request::ReviewChange { .. }
            | Request::ChangeAgent { .. }
            | Request::Stop { .. }
            | Request::MarkSeen { .. }
            | Request::ChooseWorkflow { turn: None, .. }
            | Request::DraftDescription { .. }
            | Request::ApplyChange { .. }
            | Request::CatchUp { .. }
            | Request::DiscardChange { .. }
            | Request::SendForReview { .. }
    )
}
