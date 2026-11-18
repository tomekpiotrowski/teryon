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
//! outside the lock.

use std::collections::{HashMap, HashSet};
use std::env::{join_paths, split_paths, var_os};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::process::Command;
use tokio::sync::broadcast::{channel, Receiver, Sender};
use tokio::time::{sleep, Instant};
use tracing::{error, warn};
use uuid::Uuid;

use crate::agents::runner::{Journal, Pending, Runner, ThreadChange, TurnRequest};
use crate::agents::{Agent, EntryDraft, EntryKind};
use crate::changes::claim::{claimant, Candidate};
use crate::changes::create::{create, take_over};
use crate::changes::open::{command_line, file_manager, CommandLine};
use crate::changes::status::{read_state, Look};
use crate::changes::{copies_in, Change, ChangeError, ChangeState, FoundCopy};
use crate::directory::existing_directory;
use crate::files::repository::{Place, Repository};
use crate::files::{FileView, FilesError, Listing};
use crate::history::turns::{turns_in, working_at, Turn};
use crate::history::{Changes, History};
use crate::protocol::{Event, Opener, Opens, ProjectState, ProjectView};
use crate::settings::{AgentSettings, OpenSettings};
use crate::store::projects::ProjectList;
use crate::store::threads::ThreadStore;
use crate::store::{Entry, Project, Status, StoreError, Thread};

/// The longest title taken from a first message.
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
}

/// What a workspace needs besides its data folder.
#[derive(Clone, Debug, Default)]
pub struct Setup {
    pub agents: AgentSettings,
    pub open: OpenSettings,
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
}

impl State {
    fn view(&self, project: &Project) -> ProjectView {
        let state = self
            .threads
            .get(&project.id)
            .map_or(ProjectState::Missing, Threads::state);
        ProjectView::new(project, state)
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
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn announce(&self, event: Event) {
        // Nobody listening is fine; the change is stored either way.
        let _ = self.changes.send(event);
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
        for project in projects.projects() {
            let mut opened = Threads::open(project);
            if let Threads::Ready(store) = &mut opened {
                release_interrupted(store);
            }
            threads.insert(project.id, opened);
        }
        let (changes, _) = channel(BACKLOG);
        Ok(Self {
            inner: Arc::new(Inner {
                setup,
                state: Mutex::new(State { projects, threads }),
                runner: Runner::default(),
                changes,
                noticed: Mutex::new(HashMap::new()),
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
        }
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
        drop(state);

        self.inner.announce(Event::ProjectRemoved { project_id });
        Ok(())
    }

    pub fn start_thread(
        &self,
        project_id: Uuid,
        agent: &str,
        title: &str,
    ) -> Result<Thread, WorkspaceError> {
        let agent = Agent::parse(agent).ok_or_else(|| {
            WorkspaceError::Invalid("Choose Claude or Grok as the agent.".to_string())
        })?;
        let mut state = self.inner.state();
        state.project(project_id)?;
        let thread = state.ready(project_id)?.save(Thread {
            id: Uuid::new_v4(),
            project_id,
            title: title.trim().to_string(),
            agent: agent.name().to_string(),
            conversation_id: Uuid::new_v4(),
            conversation_started: false,
            status: Status::Idle,
            unread: false,
            commands: Vec::new(),
            change: None,
            created_at: Utc::now(),
        })?;
        drop(state);

        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        Ok(thread)
    }

    /// Stores a message and starts the thread's agent on it.
    ///
    /// Checking that the thread is idle and marking it as running happen
    /// under one lock, so two messages sent at the same moment cannot both
    /// start an agent.
    pub fn send_message(&self, thread_id: Uuid, text: &str) -> Result<Entry, WorkspaceError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(WorkspaceError::Invalid(
                "Write a message first.".to_string(),
            ));
        }

        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        if thread.status == Status::Running {
            return Err(WorkspaceError::Busy(
                "The agent is still working on the previous message.",
            ));
        }
        let draft = EntryDraft::user_message(text);
        let entry = store.append(thread_id, draft.kind.name(), draft.body)?;
        let thread = store.change(thread_id, |thread| {
            thread.status = Status::Running;
            if thread.title.is_empty() {
                thread.title = title_from(text);
            }
        })?;
        let directory = state.project(thread.project_id)?.directory.clone();
        drop(state);

        self.inner.announce(Event::EntryAdded {
            entry: entry.clone(),
        });
        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        self.inner.runner.start(
            Arc::new(WorkspaceJournal(self.inner.clone())),
            self.inner.setup.agents.clone(),
            TurnRequest {
                thread,
                directory,
                message: text.to_string(),
                environment: self
                    .inner
                    .setup
                    .tools
                    .as_ref()
                    .map(AgentTools::environment)
                    .unwrap_or_default(),
            },
        );
        Ok(entry)
    }

    /// Asks a thread's agent to stop. Does nothing when it is not running.
    pub fn stop(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        self.inner.state().holding(thread_id)?;
        self.inner.runner.stop(thread_id);
        Ok(())
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

    /// Starts a change for a thread: a copy of the project on a new branch, named by the agent.
    ///
    /// Only the agent running the thread's current turn may, which it proves
    /// with the turn's identifier from its environment. That keeps a command
    /// copied to a terminal, or run after the turn, from touching the thread.
    /// Returns where the copy is.
    pub async fn start_change(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        name: &str,
        from: Option<&str>,
    ) -> Result<PathBuf, WorkspaceError> {
        let (directory, project_id) = {
            let mut state = self.inner.state();
            let (_, thread) = state.holding(thread_id)?;
            if self.inner.runner.turn_of(thread_id) != Some(turn) {
                return Err(WorkspaceError::Invalid(
                    "`teryon worktree create` is for agents working in Teryon, during their turn."
                        .to_string(),
                ));
            }
            if let Some(change) = &thread.change {
                return Err(WorkspaceError::Invalid(format!(
                    "This thread already has a worktree at `{}`. Work there.",
                    change.path.display()
                )));
            }
            (
                state.project(thread.project_id)?.directory.clone(),
                thread.project_id,
            )
        };

        // Noticed before it exists, so a look in between cannot take it for one
        // the agent made by hand and give it away with a guessed starting point.
        self.inner.notice(project_id, name);
        let change = create(&directory, name, from).await?;
        let path = change.path.clone();
        self.inner.adopt(thread_id, change)?;
        self.inner.look_at(thread_id, Look::Quick).await;
        Ok(path)
    }

    /// Reads again where a thread's change stands, closely. For when a person opens the thread.
    pub async fn refresh_change(&self, thread_id: Uuid) {
        self.inner.look_at(thread_id, Look::Thorough).await;
    }

    /// Opens a thread's copy in the file manager, or in the editor or terminal set up for it.
    pub fn open_copy(&self, thread_id: Uuid, with: Opener) -> Result<(), WorkspaceError> {
        let (_, thread) = self.inner.state().holding(thread_id)?;
        let change = thread
            .change
            .ok_or(WorkspaceError::Busy("This thread has no changes to open."))?;
        if !change.path.is_dir() {
            return Err(WorkspaceError::Busy(
                "This thread's copy was removed outside Teryon.",
            ));
        }
        let open = &self.inner.setup.open;
        let line = match with {
            Opener::Folder => Some(file_manager(&change.path)),
            Opener::Editor => open
                .editor
                .as_deref()
                .and_then(|editor| command_line(editor, &change.path)),
            Opener::Terminal => open
                .terminal
                .as_deref()
                .and_then(|terminal| command_line(terminal, &change.path)),
        }
        .ok_or(WorkspaceError::Busy(
            "Nothing is set up to open it with. Add it under [open] in Teryon's config.toml.",
        ))?;
        start_detached(&line, &change.path)
    }

    /// A thread's conversation so far.
    pub fn entries(&self, thread_id: Uuid) -> Result<Vec<Entry>, WorkspaceError> {
        let mut state = self.inner.state();
        let (store, _) = state.holding(thread_id)?;
        Ok(store.entries(thread_id)?)
    }

    /// Every file of a branch or a commit. Without either, the project's directory as it is.
    pub async fn files(
        &self,
        project_id: Uuid,
        branch: Option<&str>,
        commit: Option<&str>,
    ) -> Result<Listing, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        Ok(repository.listing(Place::named(branch, commit)).await?)
    }

    pub async fn file(
        &self,
        project_id: Uuid,
        branch: Option<&str>,
        commit: Option<&str>,
        path: &str,
    ) -> Result<FileView, WorkspaceError> {
        let repository = self.repository(project_id).await?;
        Ok(repository.file(Place::named(branch, commit), path).await?)
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
            commit.thread = working_at(&turns, commit.time);
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
                        ThreadChange::ConversationStarted => thread.conversation_started = true,
                        ThreadChange::Commands(commands) => thread.commands = commands,
                        ThreadChange::Finished => {
                            thread.status = Status::Idle;
                            thread.unread = true;
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
}

impl Inner {
    /// Gives a thread a change, unless it got one meanwhile, and tells everyone.
    fn adopt(&self, thread_id: Uuid, change: Change) -> Result<(), WorkspaceError> {
        let mut state = self.state();
        let (store, thread) = state.holding(thread_id)?;
        if thread.change.is_some() {
            return Ok(());
        }
        let thread = store.change(thread_id, |thread| thread.change = Some(Box::new(change)))?;
        drop(state);
        self.announce(Event::ThreadChanged { thread });
        Ok(())
    }

    /// Reads where a thread's change stands and stores it when it differs.
    ///
    /// A thread without a change takes over a copy its agent made by hand,
    /// when [`claimant`] says it is this thread's. Problems are logged: this
    /// runs on its own, with nobody to tell, and the next look tries again.
    async fn look_at(&self, thread_id: Uuid, look: Look) {
        let found = {
            let mut state = self.state();
            let Ok((store, thread)) = state.holding(thread_id) else {
                return;
            };
            let candidates: Vec<Candidate> = store
                .threads()
                .iter()
                .filter(|other| other.change.is_none())
                .map(|other| Candidate {
                    thread_id: other.id,
                    running: other.status == Status::Running,
                })
                .collect();
            let taken: Vec<String> = store
                .threads()
                .iter()
                .filter_map(|other| other.change.as_ref().map(|change| change.name.clone()))
                .collect();
            let Ok(project) = state.project(thread.project_id) else {
                return;
            };
            (project.directory.clone(), thread, candidates, taken)
        };
        let (directory, thread, candidates, taken) = found;

        let change = match thread.change {
            Some(change) => *change,
            None => {
                self.claim_copies(thread.project_id, &directory, &candidates, &taken)
                    .await;
                let claimed = self
                    .state()
                    .holding(thread_id)
                    .ok()
                    .and_then(|(_, thread)| thread.change);
                match claimed {
                    Some(change) => *change,
                    None => return,
                }
            }
        };
        let (state, branch) = match read_state(&directory, &change, look).await {
            Ok(read) => read,
            Err(problem) => {
                warn!(
                    "could not read where the change {} stands: {problem}",
                    change.name
                );
                return;
            }
        };
        let state = keep_known_fit(change.state.as_ref(), state, look);
        if change.state.as_ref() == Some(&state) && change.branch == branch {
            return;
        }

        let mut guarded = self.state();
        let Ok((store, current)) = guarded.holding(thread_id) else {
            return;
        };
        // Started, or taken over, since this look began: that look is the one to keep.
        if current
            .change
            .as_ref()
            .is_some_and(|known| known.name != change.name)
        {
            return;
        }
        let updated = Change {
            state: Some(state),
            branch,
            ..change
        };
        match store.change(thread_id, |thread| thread.change = Some(Box::new(updated))) {
            Ok(thread) => {
                drop(guarded);
                self.announce(Event::ThreadChanged { thread });
            }
            Err(problem) => error!("could not store where a change stands: {problem}"),
        }
    }

    /// Gives the copies in `.teryon/worktrees/` that no thread has to the threads they belong to, see [`claimant`].
    ///
    /// The first look at a project after Teryon starts only notes the copies
    /// already there: none of them is new, so only those named after a
    /// thread are given to it.
    async fn claim_copies(
        &self,
        project_id: Uuid,
        directory: &Path,
        candidates: &[Candidate],
        taken: &[String],
    ) {
        let copies = match copies_in(directory).await {
            Ok(copies) => copies,
            Err(problem) => {
                warn!(
                    "could not list the copies of {}: {problem}",
                    directory.display()
                );
                return;
            }
        };
        let owned: Vec<(Uuid, FoundCopy)> = {
            let mut noticed = self.noticed();
            let project = noticed.entry(project_id).or_default();
            let first_look = !project.looked;
            project.looked = true;
            copies
                .into_iter()
                .filter(|copy| !taken.contains(&copy.name))
                .filter_map(|copy| {
                    let new = project.names.insert(copy.name.clone()) && !first_look;
                    claimant(&copy.name, new, candidates).map(|owner| (owner, copy))
                })
                .collect()
        };
        for (owner, copy) in owned {
            let adopted = take_over(directory, &copy)
                .await
                .map_err(WorkspaceError::from)
                .and_then(|change| self.adopt(owner, change));
            if let Err(problem) = adopted {
                warn!(
                    "could not give the copy {} to its thread: {problem}",
                    copy.name
                );
            }
        }
    }

    /// Notes that a copy is not new, so it is never given to a thread by [`claimant`]'s guess.
    fn notice(&self, project_id: Uuid, name: &str) {
        self.noticed()
            .entry(project_id)
            .or_default()
            .names
            .insert(name.to_string());
    }

    fn noticed(&self) -> MutexGuard<'_, HashMap<Uuid, Noticed>> {
        self.noticed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The copies Teryon has noticed in one project since it started.
#[derive(Default)]
struct Noticed {
    /// Whether the project's copies have been listed at all yet.
    looked: bool,
    names: HashSet<String>,
}

/// What a quick look says, with whether the change applies kept from the last thorough look.
///
/// A quick look does not check that, and saying "unknown" every second of a
/// turn would make the status line flicker.
fn keep_known_fit(known: Option<&ChangeState>, mut state: ChangeState, look: Look) -> ChangeState {
    if look == Look::Quick && state.copy_present && state.base_present {
        if let Some(known) = known {
            state.fit = known.fit;
        }
    }
    state
}

/// Starts a program in `directory` and leaves it running on its own.
fn start_detached(line: &CommandLine, directory: &Path) -> Result<(), WorkspaceError> {
    Command::new(&line.program)
        .args(&line.arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|problem| {
            WorkspaceError::Invalid(format!("Could not start `{}`: {problem}.", line.program))
        })
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
                })
            });
        if let Err(problem) = released {
            error!("could not release an interrupted thread: {problem}");
        }
    }
}

/// The start of a message's first line, short enough for a list.
fn title_from(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
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
