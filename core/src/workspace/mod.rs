//! Everything a person can do with their projects and threads, and the rules for it.
//!
//! The service hands every request here, whoever sent it and however. This is
//! where a message is refused while the agent is still working, where a
//! project with a running agent cannot be removed, and where every change is
//! announced to whoever is listening.
//!
//! Projects are read from the data folder when the workspace opens, and each
//! project's threads from its own `.teryon/` folder. A project whose directory
//! has gone, or that another Teryon has open, is still listed with that state,
//! so it can be removed.
//!
//! The stores are behind one lock. Every change is a few small file writes,
//! so holding it briefly is cheaper than anything finer would be. Reading a
//! repository for the file browser, the history or a thread's change happens
//! outside the lock. Changes have a file of their own, see [`changes`].

mod changes;

use std::collections::{HashMap, HashSet};
use std::env::{join_paths, split_paths, var_os};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::join;
use tokio::sync::broadcast::{channel, Receiver, Sender};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, Instant};
use tracing::{error, warn};
use uuid::Uuid;

use crate::agents::asking::{Questions, SHAPE};
use crate::agents::handover::handed_over;
use crate::agents::models::{
    checked_model, list_codex_models, list_grok_models, ModelError, ModelLists,
};
use crate::agents::runner::{Journal, Pending, Runner, ThreadChange, TurnRequest, TurnUsage};
use crate::agents::{Agent, EntryDraft, EntryKind, Voice};
use crate::changes::finish::FinishError;
use crate::changes::github::on_github;
use crate::changes::reviewing::{REVIEW_AGAIN, REVIEW_FIRST};
use crate::changes::status::Look;
use crate::changes::ChangeError;
use crate::directory::existing_directory;
use crate::files::repository::{Place, Repository};
use crate::files::{FileView, FilesError, Listing};
use crate::history::turns::{turns_in, working_at, Turn};
use crate::history::{Changes, History};
use crate::protocol::{Event, Opens, ProjectState, ProjectView};
use crate::settings::{AgentSettings, GitHubSettings, OpenSettings};
use crate::store::projects::ProjectList;
use crate::store::threads::ThreadStore;
use crate::store::{Entry, Project, Reviewer, Status, StoreError, Thread, Verdict};
use crate::usage::{UsageBook, UsageError, UsageLine, UsageReport};
use crate::workspace::changes::Noticed;

/// The longest title a list shows whole.
const LONGEST_TITLE: usize = 60;

/// How many changes a slow listener may fall behind before it must start over.
const BACKLOG: usize = 1024;

/// Why a request was refused. The text is meant for the person.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// What was sent cannot be accepted; the text says why.
    #[error("{0}")]
    Invalid(String),
    #[error("There is no such project.")]
    UnknownProject,
    #[error("There is no such thread.")]
    UnknownThread,
    /// The request is fine but cannot be done right now; the text says why.
    #[error("{0}")]
    Busy(&'static str),
    #[error("Something went wrong while saving. The service's log has the details.")]
    Store(#[from] StoreError),
    /// The project's repository could not be read; the text says why.
    #[error("{0}")]
    Files(#[from] FilesError),
    /// A change could not be started; the text, meant for the agent, says why.
    #[error("{0}")]
    Change(#[from] ChangeError),
    /// A change could not be finished; the text says why.
    #[error("{0}")]
    Finish(#[from] FinishError),
    /// A model name was refused; the text says why.
    #[error("{0}")]
    Model(#[from] ModelError),
    #[error("Could not read the usage log. The service's log has the details.")]
    Usage(#[from] UsageError),
}

/// What a workspace needs besides its data folder.
#[derive(Clone, Debug, Default)]
pub struct Setup {
    pub agents: AgentSettings,
    pub open: OpenSettings,
    pub github: GitHubSettings,
    /// What agents need to call `teryon` themselves. Nothing where there is
    /// no such program, as in the core library's own tests.
    pub tools: Option<AgentTools>,
}

/// The `teryon` program an agent calls, and the data folder it finds the service through.
#[derive(Clone, Debug)]
pub struct AgentTools {
    pub program: PathBuf,
    pub data: PathBuf,
}

impl AgentTools {
    /// The variables that let an agent's shell find `teryon`, and `teryon` find this service.
    ///
    /// The program's folder goes first on `PATH`, so the agent reaches the
    /// `teryon` that is running, never another one it may find.
    fn environment(&self) -> Vec<(String, OsString)> {
        let mut environment = vec![(
            "TERYON_DATA".to_string(),
            self.data.clone().into_os_string(),
        )];
        if let Some(folder) = self.program.parent() {
            let mut folders = vec![folder.to_path_buf()];
            folders.extend(
                var_os("PATH")
                    .map(|path| split_paths(&path).collect::<Vec<_>>())
                    .unwrap_or_default(),
            );
            if let Ok(path) = join_paths(folders) {
                environment.push(("PATH".to_string(), path));
            }
        }
        environment
    }
}

/// A project's threads, or why they cannot be reached.
enum Threads {
    Ready(ThreadStore),
    Missing,
    InUse,
}

impl Threads {
    fn open(project: &Project) -> Self {
        if !project.directory.is_dir() {
            return Self::Missing;
        }
        match ThreadStore::open(project.id, &project.directory) {
            Ok(store) => Self::Ready(store),
            Err(StoreError::InUse) => Self::InUse,
            Err(problem) => {
                error!(
                    "could not open the threads of {}: {problem}",
                    project.directory.display()
                );
                Self::Missing
            }
        }
    }

    fn state(&self) -> ProjectState {
        match self {
            Self::Ready(_) => ProjectState::Ready,
            Self::Missing => ProjectState::Missing,
            Self::InUse => ProjectState::InUse,
        }
    }
}

struct State {
    projects: ProjectList,
    threads: HashMap<Uuid, Threads>,
    /// The projects whose `origin` is on GitHub, read when each was opened.
    on_github: HashSet<Uuid>,
}

impl State {
    fn view(&self, project: &Project) -> ProjectView {
        let state = self
            .threads
            .get(&project.id)
            .map_or(ProjectState::Missing, Threads::state);
        ProjectView::new(project, state, self.on_github.contains(&project.id))
    }

    fn project(&self, id: Uuid) -> Result<&Project, WorkspaceError> {
        self.projects
            .projects()
            .iter()
            .find(|project| project.id == id)
            .ok_or(WorkspaceError::UnknownProject)
    }

    /// The store of a project that can be worked in.
    fn ready(&mut self, project_id: Uuid) -> Result<&mut ThreadStore, WorkspaceError> {
        match self.threads.get_mut(&project_id) {
            Some(Threads::Ready(store)) => Ok(store),
            Some(Threads::InUse) => Err(WorkspaceError::Busy(
                "Another Teryon is using this project.",
            )),
            Some(Threads::Missing) => {
                Err(WorkspaceError::Busy("This project's directory is gone."))
            }
            None => Err(WorkspaceError::UnknownProject),
        }
    }

    /// The store holding a thread, and the thread.
    fn holding(&mut self, thread_id: Uuid) -> Result<(&mut ThreadStore, Thread), WorkspaceError> {
        self.threads
            .values_mut()
            .find_map(|threads| match threads {
                Threads::Ready(store) => store
                    .thread(thread_id)
                    .cloned()
                    .map(|thread| (store, thread)),
                _ => None,
            })
            .ok_or(WorkspaceError::UnknownThread)
    }

    fn all_threads(&self) -> Vec<Thread> {
        self.threads
            .values()
            .filter_map(|threads| match threads {
                Threads::Ready(store) => Some(store.threads()),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

struct Inner {
    setup: Setup,
    state: Mutex<State>,
    runner: Runner,
    changes: Sender<Event>,
    /// The copies each project had when Teryon looked, see [`Inner::claim_copies`].
    noticed: Mutex<HashMap<Uuid, Noticed>>,
    /// The threads whose change is being applied, caught up or discarded.
    finishing_threads: Mutex<HashSet<Uuid>>,
    /// One lock per project, held while one of its changes is finished.
    project_locks: Mutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>,
    /// The models each agent offers besides its default.
    models: Mutex<ModelLists>,
    /// The agents' limits and the log of turns, in the data folder.
    usage: Mutex<UsageBook>,
}

impl Inner {
    /// Refuses to start an agent in a thread where one is working, or whose change is being finished.
    fn check_free(&self, thread: &Thread) -> Result<(), WorkspaceError> {
        if thread.status == Status::Running {
            return Err(WorkspaceError::Busy(
                "The agent is still working on the previous message.",
            ));
        }
        if self.finishing_threads().contains(&thread.id) {
            return Err(WorkspaceError::Busy(
                "Teryon is finishing this thread's changes. Send the message once it is done.",
            ));
        }
        Ok(())
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn usage(&self) -> MutexGuard<'_, UsageBook> {
        self.usage.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn announce(&self, event: Event) {
        // Nobody listening is fine; the change is stored either way.
        let _ = self.changes.send(event);
    }

    fn models(&self) -> MutexGuard<'_, ModelLists> {
        self.models.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces one agent's models and tells listeners the lists as they now stand.
    ///
    /// Codex and Grok are asked together. The lists stay locked until the
    /// notice is sent, so the slower answer includes the faster one's names
    /// instead of publishing a copy from before that change and wiping it.
    fn publish_models(&self, change: impl FnOnce(&mut ModelLists)) {
        let mut models = self.models();
        change(&mut models);
        self.announce(Event::ModelsChanged {
            models: models.clone(),
        });
    }
}

/// A person's projects and threads on this machine.
#[derive(Clone)]
pub struct Workspace {
    inner: Arc<Inner>,
}

impl Workspace {
    /// Opens the projects listed in `data_folder`.
    ///
    /// No agent can be running when the workspace opens, so any thread still
    /// marked as running was cut short when Teryon last stopped. It is marked
    /// idle and unread, with a notice saying why.
    pub fn open(data_folder: &Path, setup: Setup) -> Result<Self, WorkspaceError> {
        let projects = ProjectList::open(data_folder)?;
        let mut threads = HashMap::new();
        let mut github = HashSet::new();
        for project in projects.projects() {
            if on_github(&project.directory) {
                github.insert(project.id);
            }
            let mut opened = Threads::open(project);
            if let Threads::Ready(store) = &mut opened {
                release_interrupted(store);
            }
            threads.insert(project.id, opened);
        }
        let (changes, _) = channel(BACKLOG);
        let models = Mutex::new(ModelLists::from_settings(&setup.agents));
        Ok(Self {
            inner: Arc::new(Inner {
                models,
                setup,
                state: Mutex::new(State {
                    projects,
                    threads,
                    on_github: github,
                }),
                runner: Runner::default(),
                changes,
                noticed: Mutex::new(HashMap::new()),
                finishing_threads: Mutex::new(HashSet::new()),
                project_locks: Mutex::new(HashMap::new()),
                usage: Mutex::new(UsageBook::open(data_folder)),
            }),
        })
    }

    /// Every project and thread as they are now.
    pub fn snapshot(&self) -> Event {
        let state = self.inner.state();
        Event::Snapshot {
            projects: state
                .projects
                .projects()
                .iter()
                .map(|project| state.view(project))
                .collect(),
            threads: state.all_threads(),
            opens: Opens {
                editor: self.inner.setup.open.editor.is_some(),
                terminal: self.inner.setup.open.terminal.is_some(),
            },
            models: self.inner.models().clone(),
        }
    }

    /// Asks the agent programs that list their own models for them, unless `config.toml` names them.
    ///
    /// Asking takes a moment and can fail, so it happens after the service
    /// has started. Each list arrives independently; failures are logged and
    /// leave that agent with its default. Explicit settings are never replaced.
    pub async fn list_models(&self) {
        let codex = async {
            let settings = &self.inner.setup.agents.codex;
            if settings.models.is_some() {
                return;
            }
            match list_codex_models(settings).await {
                Ok(names) => self.inner.publish_models(|models| models.codex = names),
                Err(problem) => warn!("Codex offers only its default model: {problem}"),
            }
        };
        let grok = async {
            let settings = &self.inner.setup.agents.grok;
            if settings.models.is_some() {
                return;
            }
            match list_grok_models(&settings.program).await {
                Ok(names) => self.inner.publish_models(|models| models.grok = names),
                Err(problem) => warn!("Grok offers only its default model: {problem}"),
            }
        };
        join!(codex, grok);
    }

    /// Every change from now on.
    ///
    /// A listener that falls more than a thousand changes behind misses some
    /// and is told so by the channel; it should take a fresh snapshot then.
    pub fn subscribe(&self) -> Receiver<Event> {
        self.inner.changes.subscribe()
    }

    pub fn add_project(&self, name: &str, directory: &str) -> Result<ProjectView, WorkspaceError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(WorkspaceError::Invalid(
                "A project needs a name.".to_string(),
            ));
        }
        let directory = existing_directory(directory)
            .map_err(|problem| WorkspaceError::Invalid(problem.to_string()))?;

        let mut state = self.inner.state();
        if state
            .projects
            .projects()
            .iter()
            .any(|project| project.directory == directory)
        {
            return Err(WorkspaceError::Invalid(
                "This directory is already a project.".to_string(),
            ));
        }
        let project = Project {
            id: Uuid::new_v4(),
            name: name.to_string(),
            directory,
            created_at: Utc::now(),
        };
        if on_github(&project.directory) {
            state.on_github.insert(project.id);
        }
        let mut opened = Threads::open(&project);
        if let Threads::Ready(store) = &mut opened {
            release_interrupted(store);
        }
        let threads = match &opened {
            Threads::Ready(store) => store.threads(),
            _ => Vec::new(),
        };
        state.projects.add(project.clone())?;
        state.threads.insert(project.id, opened);
        let view = state.view(&project);
        drop(state);

        self.inner.announce(Event::ProjectAdded {
            project: view.clone(),
        });
        for thread in threads {
            self.inner.announce(Event::ThreadChanged { thread });
        }
        Ok(view)
    }

    /// Forgets a project. Its directory, threads and worktrees stay where they are.
    pub fn remove_project(&self, project_id: Uuid) -> Result<(), WorkspaceError> {
        let mut state = self.inner.state();
        state.project(project_id)?;
        if let Some(Threads::Ready(store)) = state.threads.get(&project_id) {
            if store
                .threads()
                .iter()
                .any(|thread| thread.status == Status::Running)
            {
                return Err(WorkspaceError::Busy(
                    "An agent is still running in this project.",
                ));
            }
        }
        state.projects.remove(project_id)?;
        state.threads.remove(&project_id);
        state.on_github.remove(&project_id);
        drop(state);

        self.inner.announce(Event::ProjectRemoved { project_id });
        Ok(())
    }

    pub fn start_thread(
        &self,
        project_id: Uuid,
        agent: &str,
        model: Option<&str>,
        title: &str,
    ) -> Result<Thread, WorkspaceError> {
        let agent = Agent::parse(agent).ok_or_else(|| {
            WorkspaceError::Invalid("Choose Claude, Codex or Grok as the agent.".to_string())
        })?;
        let title = title.trim();
        let model = checked_model(model)?;
        let mut state = self.inner.state();
        state.project(project_id)?;
        let thread = state.ready(project_id)?.save(Thread {
            id: Uuid::new_v4(),
            project_id,
            title: title.to_string(),
            named: !title.is_empty(),
            agent: agent.name().to_string(),
            model,
            conversation: None,
            handover: false,
            status: Status::Idle,
            unread: false,
            asking: false,
            commands: Vec::new(),
            change: None,
            finished: None,
            reviewer: None,
            created_at: Utc::now(),
        })?;
        drop(state);

        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        Ok(thread)
    }

    /// Gives a thread the title its agent chose, in place of the start of the first message.
    ///
    /// A person tells threads apart in a list by their titles, and the start
    /// of a message is often a greeting or context rather than the point.
    /// The agent has read the whole message, so it can say in a few words
    /// what the thread is about.
    ///
    /// Only the agent running the thread's current turn may, which it proves
    /// with the turn's identifier from its environment, as for a change.
    pub fn name_thread(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        title: &str,
    ) -> Result<(), WorkspaceError> {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return Err(WorkspaceError::Invalid(
                "Give the thread a title of a few words.".to_string(),
            ));
        }
        let mut state = self.inner.state();
        let (store, _) = state.holding(thread_id)?;
        if self.inner.runner.turn_of(thread_id) != Some(turn) {
            return Err(WorkspaceError::Invalid(
                "`teryon thread title` is for agents working in Teryon, during their turn."
                    .to_string(),
            ));
        }
        let thread = store.change(thread_id, |thread| {
            thread.title = shortened(&title);
            thread.named = true;
        })?;
        drop(state);

        self.inner.announce(Event::ThreadChanged { thread });
        Ok(())
    }

    /// Records questions the thread's agent asks the person, and marks the thread as waiting for answers.
    ///
    /// Agents run without a screen, so on their own they guess every unclear
    /// choice. Asking lets them hand a design choice to the person instead.
    /// The agent then ends its turn, and the person's next message carries
    /// the answers.
    ///
    /// Only the thread's own agent may ask, during its current turn: a
    /// reviewer reports to the author, not to the person. The questions are
    /// read here rather than in `teryon thread ask`, so every refusal is
    /// logged where someone looking into a quiet agent will find it. Asking
    /// twice in one turn adds a second entry; the app shows only the newer.
    pub fn ask_questions(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        asked: &str,
    ) -> Result<(), WorkspaceError> {
        let asking = self.take_questions(thread_id, turn, asked);
        if let Err(problem) = &asking {
            warn!(%thread_id, %problem, "refused the questions an agent asked");
        }
        let (entry, thread) = asking?;
        self.inner.announce(Event::EntryAdded { entry });
        self.inner.announce(Event::ThreadChanged { thread });
        Ok(())
    }

    fn take_questions(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        asked: &str,
    ) -> Result<(Entry, Thread), WorkspaceError> {
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        if self.inner.runner.turn_of(thread_id) != Some(turn) {
            return Err(WorkspaceError::Invalid(
                "`teryon thread ask` is for agents working in Teryon, during their turn."
                    .to_string(),
            ));
        }
        if thread
            .reviewer
            .as_ref()
            .is_some_and(|reviewer| reviewer.reviewing)
        {
            return Err(WorkspaceError::Invalid(
                "`teryon thread ask` is for the thread's own agent; a reviewer reports its findings instead."
                    .to_string(),
            ));
        }
        let questions = Questions::read(asked)
            .map_err(|problem| WorkspaceError::Invalid(format!("{problem}\n\n{SHAPE}")))?;
        let draft = EntryDraft::questions(turn, &questions);
        let entry = store.append(thread_id, draft.kind.name(), draft.body)?;
        let thread = store.change(thread_id, |thread| thread.asking = true)?;
        Ok((entry, thread))
    }

    /// Sends a message to a thread's agent, or to its reviewer, and starts that agent on it.
    ///
    /// Checking that the thread is idle and marking it as running happen
    /// under one lock, so two messages sent at the same moment cannot both
    /// start an agent. Only one of a thread's two agents works at a time,
    /// so their lines never mix. A message to the author means it may change
    /// the code again, so the last review may no longer fit.
    pub fn send_message(
        &self,
        thread_id: Uuid,
        text: &str,
        to: Voice,
    ) -> Result<Entry, WorkspaceError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(WorkspaceError::Invalid(
                "Write a message first.".to_string(),
            ));
        }

        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        self.inner.check_free(&thread)?;
        if to == Voice::Reviewer && (thread.reviewer.is_none() || thread.change.is_none()) {
            return Err(WorkspaceError::Invalid(
                "Nobody reviews this thread yet. Ask for a review first.".to_string(),
            ));
        }
        let message = handover_message(store, &thread, to, text)?;
        let mut draft = EntryDraft::user_message(text).by(to);
        if to == Voice::Author {
            // Which agent a message went to tells the history whose work followed, see `history::turns`.
            draft.body["agent"] = thread.agent.clone().into();
        }
        if let Some(reviewer) = thread
            .reviewer
            .as_ref()
            .and_then(|reviewer| Agent::parse(&reviewer.agent))
        {
            draft = draft.naming(reviewer);
        }
        let entry = store.append(thread_id, draft.kind.name(), draft.body)?;
        let thread = store.change(thread_id, |thread| {
            thread.status = Status::Running;
            thread.asking = false;
            if thread.title.is_empty() {
                thread.title = title_from(text);
            }
            if let Some(reviewer) = &mut thread.reviewer {
                match to {
                    Voice::Author => {
                        if let Some(verdict) = &mut reviewer.verdict {
                            verdict.answered = true;
                        }
                    }
                    Voice::Reviewer => reviewer.reviewing = true,
                }
            }
        })?;
        let directory = state.project(thread.project_id)?.directory.clone();
        drop(state);

        self.inner.announce(Event::EntryAdded {
            entry: entry.clone(),
        });
        self.start_turn(thread, to, directory, &message);
        Ok(entry)
    }

    /// Asks a second agent to review a thread's change, in the same thread.
    ///
    /// The reviewer is another agent program than the author, so a second
    /// model reads the work. It reviews in a conversation of its own, which
    /// the next review by the same agent continues; a review by a different
    /// agent starts afresh. Answers with the line that marks the review's
    /// start in the thread. The model is the one this review runs on; a
    /// reviewer asked again may be asked on another model than before.
    pub fn review_change(
        &self,
        thread_id: Uuid,
        reviewer: &str,
        model: Option<&str>,
    ) -> Result<Entry, WorkspaceError> {
        let agent = Agent::parse(reviewer).ok_or_else(|| {
            WorkspaceError::Invalid("Choose Claude, Codex or Grok as the reviewer.".to_string())
        })?;
        let model = checked_model(model)?;
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        self.inner.check_free(&thread)?;
        let change = thread.change.as_deref().ok_or(FinishError::NoChange)?;
        if let Some(changed) = &change.state {
            if !changed.copy_present {
                return Err(WorkspaceError::Invalid(
                    "The copy of this thread's changes is gone, so there is nothing to review."
                        .to_string(),
                ));
            }
            if changed.files_changed == 0 {
                return Err(WorkspaceError::Invalid(
                    "No files have changed, so there is nothing to review.".to_string(),
                ));
            }
        }
        if thread.agent == agent.name() {
            return Err(WorkspaceError::Invalid(format!(
                "{} wrote this change, so another agent has to review it.",
                agent.label()
            )));
        }
        let same_reviewer = thread
            .reviewer
            .as_ref()
            .is_some_and(|reviewer| reviewer.agent == agent.name());
        // Only a finished review the same reviewer made, and the author has worked on since, is checked for fixes.
        let again = same_reviewer
            && thread
                .reviewer
                .as_ref()
                .and_then(|reviewer| reviewer.verdict)
                .is_some_and(|verdict| verdict.answered && !verdict.cut);
        let message = if again { REVIEW_AGAIN } else { REVIEW_FIRST };
        let message = match same_reviewer {
            true => handover_message(store, &thread, Voice::Reviewer, message)?,
            false => message.to_string(),
        };
        let notice = EntryDraft::notice(&format!("{} reviews the change.", agent.label()))
            .by(Voice::Reviewer)
            .naming(agent);
        let mut body = notice.body;
        body["starts_review"] = true.into();
        let entry = store.append(thread_id, notice.kind.name(), body)?;
        let thread = store.change(thread_id, |thread| {
            thread.status = Status::Running;
            if !same_reviewer {
                thread.reviewer = Some(Reviewer {
                    agent: agent.name().to_string(),
                    model: None,
                    conversation: None,
                    handover: false,
                    reviewing: false,
                    verdict: None,
                });
            }
            if let Some(reviewer) = &mut thread.reviewer {
                reviewer.model = model;
                reviewer.reviewing = true;
                reviewer.verdict = None;
            }
        })?;
        let directory = state.project(thread.project_id)?.directory.clone();
        drop(state);

        self.inner.announce(Event::EntryAdded {
            entry: entry.clone(),
        });
        self.start_turn(thread, Voice::Reviewer, directory, &message);
        Ok(entry)
    }

    /// Tells everyone a thread is now running, and starts one of its agents.
    ///
    /// The reviewer runs inside the change's copy, so that a file it writes
    /// where it stands lands where the check after its turn sees it, not in
    /// the project folder the person may be working in.
    fn start_turn(&self, thread: Thread, voice: Voice, directory: PathBuf, message: &str) {
        let directory = match (voice, thread.change.as_deref()) {
            (Voice::Reviewer, Some(change)) => change.path.clone(),
            _ => directory,
        };
        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        self.inner.runner.start(
            Arc::new(WorkspaceJournal(self.inner.clone())),
            self.inner.setup.agents.clone(),
            TurnRequest {
                thread,
                voice,
                directory,
                message: message.to_string(),
                environment: self
                    .inner
                    .setup
                    .tools
                    .as_ref()
                    .map(AgentTools::environment)
                    .unwrap_or_default(),
            },
        );
    }

    /// Asks a thread's agent to stop. Does nothing when it is not running.
    pub fn stop(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        self.inner.state().holding(thread_id)?;
        self.inner.runner.stop(thread_id);
        Ok(())
    }

    /// Switches a thread's agent, or its reviewer, to another agent or model, from the next message on.
    ///
    /// When an agent cannot go on, for example because its subscription ran
    /// out, the person hands the thread to another one instead of starting
    /// over. A conversation cannot move to another program, so a new agent
    /// starts its own, and its first message carries what was said so far,
    /// see `crate::agents::handover`. Its slash commands are learned anew.
    ///
    /// A new model for the same agent is allowed while it works: the turn
    /// under way keeps the model it started with. A new agent has to wait for
    /// the turn to end, since the turn reports the conversation it belongs
    /// to. The author and the reviewer stay different agents, so a second
    /// model reads the work.
    pub fn change_agent(
        &self,
        thread_id: Uuid,
        to: Voice,
        agent: &str,
        model: Option<&str>,
    ) -> Result<Thread, WorkspaceError> {
        let agent = Agent::parse(agent)
            .ok_or_else(|| WorkspaceError::Invalid("Choose Claude, Codex or Grok.".to_string()))?;
        let model = checked_model(model)?;
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        let (current, other) = match to {
            Voice::Author => (
                Some(thread.agent.as_str()),
                thread
                    .reviewer
                    .as_ref()
                    .map(|reviewer| reviewer.agent.as_str()),
            ),
            Voice::Reviewer => (
                thread
                    .reviewer
                    .as_ref()
                    .map(|reviewer| reviewer.agent.as_str()),
                Some(thread.agent.as_str()),
            ),
        };
        let Some(current) = current.and_then(Agent::parse) else {
            return Err(WorkspaceError::Invalid(
                "Nobody reviews this thread yet. Ask for a review first.".to_string(),
            ));
        };
        let switching = current != agent;
        if switching {
            self.inner.check_free(&thread)?;
            if other == Some(agent.name()) {
                let text = match to {
                    Voice::Author => format!(
                        "{} reviews this thread, so it cannot also write it. Choose another reviewer first.",
                        agent.label()
                    ),
                    Voice::Reviewer => format!(
                        "{} wrote this change, so another agent has to review it.",
                        agent.label()
                    ),
                };
                return Err(WorkspaceError::Invalid(text));
            }
        }
        let notice = switching.then(|| {
            EntryDraft::notice(&format!(
                "{} takes over from {}.",
                agent.label(),
                current.label()
            ))
            .by(to)
            .naming(agent)
        });
        let entry = notice
            .map(|notice| {
                let mut body = notice.body;
                // Written while nobody works, so it ends no turn, see `history::turns`.
                body["takes_over"] = true.into();
                store.append(thread_id, notice.kind.name(), body)
            })
            .transpose()?;
        let thread = store.change(thread_id, |thread| match to {
            Voice::Author => {
                thread.model = model;
                if switching {
                    thread.agent = agent.name().to_string();
                    thread.conversation = None;
                    thread.handover = true;
                    thread.commands = Vec::new();
                }
            }
            Voice::Reviewer => {
                if let Some(reviewer) = &mut thread.reviewer {
                    reviewer.model = model;
                    if switching {
                        reviewer.agent = agent.name().to_string();
                        reviewer.conversation = None;
                        reviewer.handover = true;
                    }
                }
            }
        })?;
        drop(state);
        if let Some(entry) = entry {
            self.inner.announce(Event::EntryAdded { entry });
        }
        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        Ok(thread)
    }

    /// Notes that the person has looked at a thread.
    ///
    /// A thread that is not marked is left alone, so that looking at it again
    /// does not send a change to every client.
    pub fn mark_seen(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        if !thread.unread {
            return Ok(());
        }
        let thread = store.change(thread_id, |thread| thread.unread = false)?;
        drop(state);
        self.inner.announce(Event::ThreadChanged { thread });
        Ok(())
    }

    /// A thread's conversation so far.
    pub fn entries(&self, thread_id: Uuid) -> Result<Vec<Entry>, WorkspaceError> {
        let mut state = self.inner.state();
        let (store, _) = state.holding(thread_id)?;
        Ok(store.entries(thread_id)?)
    }

    /// Every file of a thread's copy, a branch or a commit. Without any, the project's directory as it is.
    pub async fn files(
        &self,
        project_id: Uuid,
        thread_id: Option<Uuid>,
        branch: Option<&str>,
        commit: Option<&str>,
    ) -> Result<Listing, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        let copy = self.copy_of(project_id, thread_id)?;
        Ok(repository
            .listing(Place::named(copy.as_deref(), branch, commit))
            .await?)
    }

    pub async fn file(
        &self,
        project_id: Uuid,
        thread_id: Option<Uuid>,
        branch: Option<&str>,
        commit: Option<&str>,
        path: &str,
    ) -> Result<FileView, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        let copy = self.copy_of(project_id, thread_id)?;
        Ok(repository
            .file(Place::named(copy.as_deref(), branch, commit), path)
            .await?)
    }

    /// Where a thread's copy is. Nothing when no thread is named.
    ///
    /// The thread has to be one of the project's, so a request cannot read
    /// another project's copy as if it were this one's.
    fn copy_of(
        &self,
        project_id: Uuid,
        thread_id: Option<Uuid>,
    ) -> Result<Option<PathBuf>, WorkspaceError> {
        let Some(thread_id) = thread_id else {
            return Ok(None);
        };
        let mut state = self.inner.state();
        let (_, thread) = state.holding(thread_id)?;
        if thread.project_id != project_id {
            return Err(WorkspaceError::UnknownThread);
        }
        let change = thread.change.as_deref().ok_or(FinishError::NoChange)?;
        Ok(Some(change.path.clone()))
    }

    /// The commits of a branch, each with the thread whose agent was working when it was made.
    pub async fn history(
        &self,
        project_id: Uuid,
        branch: Option<&str>,
    ) -> Result<History, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        let mut history = repository.history(branch).await?;
        let turns = self.turns(project_id)?;
        for commit in &mut history.commits {
            let turn = working_at(&turns, commit.time);
            commit.thread = turn.map(|turn| turn.thread_id);
            commit.agent = turn.map(|turn| turn.agent.clone());
        }
        Ok(history)
    }

    pub async fn changes(
        &self,
        project_id: Uuid,
        commit: &str,
        from: Option<&str>,
    ) -> Result<Changes, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        Ok(repository.changes(commit, from).await?)
    }

    pub async fn change(
        &self,
        project_id: Uuid,
        commit: &str,
        from: Option<&str>,
        path: &str,
    ) -> Result<FileView, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        Ok(repository.change(commit, from, path).await?)
    }

    /// Every agent's latest limits and the turns of the last 14 days.
    pub fn usage(&self) -> Result<UsageReport, WorkspaceError> {
        let report = self.inner.usage().report(Utc::now());
        report.map_err(|problem| {
            error!("could not read the usage: {problem}");
            WorkspaceError::from(problem)
        })
    }

    /// Stops every agent and waits up to `patience` for them to end.
    pub async fn shut_down(&self, patience: Duration) {
        self.inner.runner.stop_all();
        let deadline = Instant::now() + patience;
        while self.inner.runner.busy() && Instant::now() < deadline {
            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn repository(&self, project_id: Uuid) -> Result<Repository, WorkspaceError> {
        let directory: PathBuf = self.inner.state().project(project_id)?.directory.clone();
        Ok(Repository::open(&directory).await?)
    }

    /// When agents were working in a project, read from its threads' conversations.
    fn turns(&self, project_id: Uuid) -> Result<Vec<Turn>, WorkspaceError> {
        let state = self.inner.state();
        let Some(Threads::Ready(store)) = state.threads.get(&project_id) else {
            return Ok(Vec::new());
        };
        let mut turns = Vec::new();
        for thread in store.threads() {
            let entries = store.entries(thread.id)?;
            turns.extend(turns_in(
                thread.id,
                &thread.agent,
                thread.status == Status::Running,
                &entries,
            ));
        }
        Ok(turns)
    }
}

/// Where a running agent's results go: into the thread's store, and out to every listener.
struct WorkspaceJournal(Arc<Inner>);

impl Journal for WorkspaceJournal {
    fn add(&self, thread_id: Uuid, draft: EntryDraft) {
        let mut state = self.0.state();
        let entry = state
            .holding(thread_id)
            .map_err(|problem| problem.to_string())
            .and_then(|(store, _)| {
                store
                    .append(thread_id, draft.kind.name(), draft.body)
                    .map_err(|problem| problem.to_string())
            });
        drop(state);
        match entry {
            Ok(entry) => self.0.announce(Event::EntryAdded { entry }),
            Err(problem) => error!("could not store a thread entry: {problem}"),
        }
    }

    fn note(&self, thread_id: Uuid, change: ThreadChange) {
        let mut state = self.0.state();
        let thread = state
            .holding(thread_id)
            .map_err(|problem| problem.to_string())
            .and_then(|(store, _)| {
                store
                    .change(thread_id, |thread| match change {
                        ThreadChange::Conversation(Voice::Author, named) => {
                            thread.conversation = Some(named);
                            thread.handover = false;
                        }
                        ThreadChange::Conversation(Voice::Reviewer, named) => {
                            if let Some(reviewer) = &mut thread.reviewer {
                                reviewer.conversation = Some(named);
                                reviewer.handover = false;
                            }
                        }
                        ThreadChange::Commands(commands) => thread.commands = commands,
                        // A reply without a count answers a question, and leaves the last review's count as it was.
                        ThreadChange::Reviewed(findings) => {
                            if let Some(reviewer) = &mut thread.reviewer {
                                if findings.is_some() || reviewer.verdict.is_none() {
                                    reviewer.verdict = Some(Verdict {
                                        findings,
                                        answered: false,
                                        cut: false,
                                    });
                                }
                            }
                        }
                        // Only a review cut short counts; a question cut short leaves the last review as it was.
                        ThreadChange::ReviewCut => {
                            if let Some(reviewer) = &mut thread.reviewer {
                                if reviewer.verdict.is_none() {
                                    reviewer.verdict = Some(Verdict {
                                        findings: None,
                                        answered: false,
                                        cut: true,
                                    });
                                }
                            }
                        }
                        ThreadChange::Finished => {
                            thread.status = Status::Idle;
                            thread.unread = true;
                            if let Some(reviewer) = &mut thread.reviewer {
                                reviewer.reviewing = false;
                            }
                        }
                    })
                    .map_err(|problem| problem.to_string())
            });
        drop(state);
        match thread {
            Ok(thread) => self.0.announce(Event::ThreadChanged { thread }),
            Err(problem) => error!("could not store a change to a thread: {problem}"),
        }
    }

    fn look(&self, thread_id: Uuid, look: Look) -> Pending<'_> {
        Box::pin(self.0.look_at(thread_id, look))
    }

    /// Writes the turn to the usage log and keeps the agent's limits, telling every listener.
    ///
    /// Failing here must not fail the turn, which has ended by now, so problems are only logged.
    fn usage(&self, thread_id: Uuid, usage: TurnUsage) {
        let now = Utc::now();
        let line = usage.tokens.and_then(|tokens| {
            let state = self.0.state();
            let project_id = state
                .all_threads()
                .into_iter()
                .find(|thread| thread.id == thread_id)?
                .project_id;
            let project = state.project(project_id).ok()?;
            Some(UsageLine {
                at: now,
                agent: usage.agent.name().to_string(),
                model: usage.model.clone(),
                project_id,
                project_name: project.name.clone(),
                thread_id,
                read: tokens.read,
                written: tokens.written,
            })
        });
        let mut book = self.0.usage();
        let recorded = line.map(|line| book.record(&line).map(|()| line));
        let noted = book.note(usage.agent, usage.reading, usage.hit_limit, now);
        drop(book);
        match recorded {
            Some(Ok(line)) => self.0.announce(Event::UsageRecorded { line }),
            Some(Err(problem)) => error!("could not add a turn to the usage log: {problem}"),
            None => {}
        }
        match noted {
            Ok(Some(limits)) => self.0.announce(Event::LimitsChanged { limits }),
            Ok(None) => {}
            Err(problem) => error!("could not keep an agent's limits: {problem}"),
        }
    }
}

/// Marks every thread left as running as idle and unread, and says why in the thread.
fn release_interrupted(store: &mut ThreadStore) {
    for thread in store.threads() {
        if thread.status != Status::Running {
            continue;
        }
        let notice = EntryDraft::notice("Interrupted by a restart.");
        let released = store
            .append(thread.id, EntryKind::Notice.name(), notice.body)
            .and_then(|_| {
                store.change(thread.id, |thread| {
                    thread.status = Status::Idle;
                    thread.unread = true;
                    if let Some(reviewer) = &mut thread.reviewer {
                        if reviewer.reviewing && reviewer.verdict.is_none() {
                            reviewer.verdict = Some(Verdict {
                                findings: None,
                                answered: false,
                                cut: true,
                            });
                        }
                        reviewer.reviewing = false;
                    }
                })
            });
        if let Err(problem) = released {
            error!("could not release an interrupted thread: {problem}");
        }
    }
}

/// The start of a message's first line, short enough for a list.
/// The message for a thread's agent or reviewer: with the thread so far when it has just taken over from another agent.
fn handover_message(
    store: &ThreadStore,
    thread: &Thread,
    voice: Voice,
    message: &str,
) -> Result<String, WorkspaceError> {
    let handover = match voice {
        Voice::Author => thread.handover,
        Voice::Reviewer => thread
            .reviewer
            .as_ref()
            .is_some_and(|reviewer| reviewer.handover),
    };
    if !handover {
        return Ok(message.to_string());
    }
    Ok(handed_over(&store.entries(thread.id)?, voice, message))
}

fn title_from(text: &str) -> String {
    shortened(text.lines().next().unwrap_or_default().trim())
}

/// A line cut to the longest title, marked where it was cut.
fn shortened(line: &str) -> String {
    if line.chars().count() <= LONGEST_TITLE {
        line.to_string()
    } else {
        let start: String = line.chars().take(LONGEST_TITLE).collect();
        format!("{}…", start.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        // requirement: a thread's title is the start of its first message, unless one was given
        expect(title_from(&text)) {
            when(text = "Fix the login bug".to_string()) as a_short_message to be_the_title {
                equal("Fix the login bug".to_string())
            }
            when(text = "First line\nSecond line".to_string()) as several_lines to use_the_first {
                equal("First line".to_string())
            }
            when(text = "é".repeat(60)) as exactly_the_longest_title to be_kept_whole {
                equal("é".repeat(60))
            }
            when(text = "é".repeat(61)) as a_longer_line to be_shortened {
                equal(format!("{}…", "é".repeat(60)))
            }
            when(text = format!("{} tail", "a".repeat(59))) as a_cut_at_a_space to drop_the_space {
                equal(format!("{}…", "a".repeat(59)))
            }
        }
    }
}
