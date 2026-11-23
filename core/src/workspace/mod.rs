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
mod stages;
#[cfg(test)]
mod starting_tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env::{join_paths, split_paths, var_os};
use std::ffi::OsString;
use std::fs::{create_dir_all, read_to_string, remove_file, rename, write};
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::id;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
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
use crate::agents::staging::{planning, reviewing_plan, staging};
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
use crate::plans::plan_path;
use crate::protocol::{Event, Opens, ProjectState, ProjectView};
use crate::settings::{
    revise, AgentSettings, Configuration, GitHubSettings, OpenSettings, Settings, StageSettings,
    WorkflowDefinition, WorkflowSettings,
};
use crate::store::projects::ProjectList;
use crate::store::threads::ThreadStore;
use crate::store::{Entry, Project, Reviewer, Status, StoreError, Thread, Verdict};
use crate::usage::{Reading, UsageBook, UsageError, UsageLine, UsageReport};
use crate::workflow::picking::{Pick, PickError};
use crate::workflow::{moved as stage_after, Happening, Stage, Workflows};
use crate::workspace::changes::Noticed;
use crate::workspace::stages::{Mover, Starter, TurnStart};

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
    pub workflow: WorkflowSettings,
    /// Who works in each stage, see [`crate::workflow::picking`].
    pub stages: StageSettings,
    pub workflows: Workflows,
    /// Whether `config.toml` names its own workflows. False means the built-in pair.
    pub own_workflows: bool,
    /// The folder `config.toml` is read from and saved to. Nothing in tests that never save.
    pub config_folder: Option<PathBuf>,
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
    /// Replaced when the settings screen saves. A guard is dropped before any other lock or an await.
    setup: Mutex<Setup>,
    /// Held for the whole of one save of the settings file. See [`Inner::saving`].
    saving: Mutex<()>,
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
    /// How each running turn started, to decide when it ends whether another agent takes over.
    turns: Mutex<HashMap<Uuid, TurnStart>>,
    /// Who the next new thread starts with while the person has not picked, see [`Inner::refresh_starting`].
    starting: Mutex<Option<Result<Pick, PickError>>>,
    /// Whether Teryon is shutting down, so no agent starts by itself any more.
    stopping: AtomicBool,
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

    /// The settings the next decision uses.
    ///
    /// Drop the guard before locking anything else, and before waiting.
    /// A turn already running keeps the copy it was given; this lock is only
    /// for the next one. Other paths lock the threads first and then this,
    /// so holding this and then waiting for the threads would stall both.
    fn setup(&self) -> MutexGuard<'_, Setup> {
        self.setup.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One save at a time, from reading the file through applying it.
    ///
    /// Two saves must not pass each other. If they did, one could write, the
    /// other could write and be applied, and the first could then apply the
    /// copy it had already read. The file would hold the later choices and
    /// the service would run the earlier ones until the next start.
    ///
    /// Take this before [`Inner::setup`]. Nothing else locks it, so that
    /// order does not reverse. Drop it before locking the threads or waiting.
    fn saving(&self) -> MutexGuard<'_, ()> {
        self.saving.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn usage(&self) -> MutexGuard<'_, UsageBook> {
        self.usage.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps what a turn told about an agent's limits, tells clients, and re-checks who new threads start with.
    ///
    /// Failing here must not fail the turn, which has ended by now, so problems are only logged.
    fn note_limits(&self, agent: Agent, reading: Reading, hit_limit: bool, now: DateTime<Utc>) {
        let noted = self.usage().note(agent, reading, hit_limit, now);
        match noted {
            Ok(Some(limits)) => {
                self.announce(Event::LimitsChanged { limits });
                self.refresh_starting();
            }
            Ok(None) => {}
            Err(problem) => error!("could not keep an agent's limits: {problem}"),
        }
    }

    /// Take this first, and before any other lock.
    fn starting(&self) -> MutexGuard<'_, Option<Result<Pick, PickError>>> {
        self.starting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn turns(&self) -> MutexGuard<'_, HashMap<Uuid, TurnStart>> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
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
                setup: Mutex::new(setup),
                saving: Mutex::new(()),
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
                turns: Mutex::new(HashMap::new()),
                starting: Mutex::new(None),
                stopping: AtomicBool::new(false),
            }),
        })
    }

    /// Every project and thread as they are now.
    pub fn snapshot(&self) -> Event {
        let starting = self.inner.refresh_starting();
        let (opens, workflows) = {
            let setup = self.inner.setup();
            (opens_of(&setup.open), setup.workflows.clone())
        };
        let state = self.inner.state();
        Event::Snapshot {
            projects: state
                .projects
                .projects()
                .iter()
                .map(|project| state.view(project))
                .collect(),
            threads: state.all_threads(),
            opens,
            models: self.inner.models().clone(),
            workflows,
            starting,
        }
    }

    /// The choices the settings screen shows.
    ///
    /// What the next turn, the next hand-over and the next copy-open will
    /// use, including a change saved a moment ago. A turn already running
    /// keeps the program, model and instructions it was given.
    pub fn configuration(&self) -> Configuration {
        Configuration::from_settings(&view_of(&self.inner.setup()))
    }

    /// Writes the settings screen's choices and starts using them.
    ///
    /// The file is replaced only after the same rules as startup accept the
    /// choices, and only the parts the screen owns. A refusal leaves the file
    /// and the running service as they were. Nothing is restarted: an agent
    /// at work finishes as it is, and the next decision reads what was saved.
    ///
    /// Reading, writing and applying are one step. A later save waits, then
    /// wins, and the file and the running service stay on that same save.
    pub fn save_configuration(
        &self,
        draft: &Configuration,
    ) -> Result<Configuration, WorkspaceError> {
        // Held until this function returns, so the write and the apply are one save.
        let _saving = self.inner.saving();
        let folder = self
            .inner
            .setup()
            .config_folder
            .clone()
            .ok_or(WorkspaceError::Invalid(
                "These settings are not stored in a file.".to_string(),
            ))?;
        let path = folder.join("config.toml");
        let current = match read_to_string(&path) {
            Ok(text) => text,
            Err(problem) if problem.kind() == ErrorKind::NotFound => String::new(),
            Err(problem) => {
                warn!("could not read the settings: {problem}");
                return Err(WorkspaceError::Invalid(
                    "Could not read the settings.".to_string(),
                ));
            }
        };
        let text = revise(&current, draft).map_err(WorkspaceError::Invalid)?;
        if let Err(problem) = write_settings(&folder, &text) {
            warn!("could not save the settings: {problem}");
            return Err(WorkspaceError::Invalid(
                "Could not save the settings.".to_string(),
            ));
        }
        let settings = Settings::read(&folder).map_err(|problem| {
            warn!("could not read the settings just saved: {problem}");
            WorkspaceError::Invalid("Could not read the settings.".to_string())
        })?;
        // Applied under the choice's lock, so a new thread never mixes the old
        // workflow with a choice made for the new one, or the other way round.
        let mut held = self.inner.starting();
        let (opens, workflows) = {
            let mut setup = self.inner.setup();
            apply_saved(&mut setup, &settings);
            (opens_of(&setup.open), setup.workflows.clone())
        };
        self.inner
            .announce(Event::SettingsChanged { opens, workflows });
        self.inner.update_starting(&mut held, false);
        drop(held);
        Ok(Configuration::from_settings(&settings))
    }

    /// Asks the agent programs that list their own models for them, unless `config.toml` names them.
    ///
    /// Asking takes a moment and can fail, so it happens after the service
    /// has started. Each list arrives independently; failures are logged and
    /// leave that agent with its default. Explicit settings are never replaced.
    pub async fn list_models(&self) {
        let (codex, grok_program, grok_named) = {
            let setup = self.inner.setup();
            (
                setup.agents.codex.clone(),
                setup.agents.grok.program.clone(),
                setup.agents.grok.models.is_some(),
            )
        };
        let codex_list = async {
            if codex.models.is_some() {
                return;
            }
            match list_codex_models(&codex).await {
                Ok(names) => self.inner.publish_models(|models| models.codex = names),
                Err(problem) => warn!("Codex offers only its default model: {problem}"),
            }
        };
        let grok_list = async {
            if grok_named {
                return;
            }
            match list_grok_models(&grok_program).await {
                Ok(names) => self.inner.publish_models(|models| models.grok = names),
                Err(problem) => warn!("Grok offers only its default model: {problem}"),
            }
        };
        join!(codex_list, grok_list);
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

    ///
    /// Without an agent, the workflow's first stage chooses one, as `config.toml`
    /// says, and Claude on its default model when it says nothing. An agent
    /// the person picked always wins.
    pub fn start_thread(
        &self,
        project_id: Uuid,
        agent: Option<&str>,
        model: Option<&str>,
        title: &str,
    ) -> Result<Thread, WorkspaceError> {
        // Held from picking until the thread is saved, so two new threads never start on the same choice.
        let mut held = match agent {
            None => Some(self.inner.starting()),
            Some(_) => None,
        };
        let workflow = self.inner.setup().workflows.of("").clone();
        let first = workflow.first();
        let (agent, model, picked) = match (agent, held.as_mut()) {
            (Some(agent), _) => {
                let agent = Agent::parse(agent).ok_or_else(|| {
                    WorkspaceError::Invalid(
                        "Choose Claude, Codex or Grok as the agent.".to_string(),
                    )
                })?;
                (agent, checked_model(model)?, false)
            }
            (None, Some(held)) => {
                self.inner.update_starting(held, false);
                match held.clone() {
                    Some(Ok(pick)) => (pick.agent, pick.model, true),
                    Some(Err(problem)) => return Err(WorkspaceError::Invalid(problem.to_string())),
                    None => (Agent::Claude, None, false),
                }
            }
            (None, None) => unreachable!("a thread without an agent holds the choice"),
        };
        let title = title.trim();
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
            workflow: workflow.name.clone(),
            stage: first,
            stage_since: Some(Utc::now()),
            plan: None,
            rounds: 0,
            held: false,
            picks: match picked {
                true => [(first, agent.name().to_string())].into(),
                false => Default::default(),
            },
            created_at: Utc::now(),
        })?;
        drop(state);

        if let (true, Some(held)) = (picked, held.as_mut()) {
            self.inner.update_starting(held, true);
        }
        drop(held);
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
        self.send(thread_id, text, to, Starter::Person)
    }

    /// Sends a message, from the person or from Teryon carrying the thread on to its next stage.
    ///
    /// A message from the person clears what holds Teryon back from starting
    /// agents by itself: the count of rounds without them, and a stop. A
    /// thread whose change is done starts its workflow over, since the
    /// message can start a new change.
    fn send(
        &self,
        thread_id: Uuid,
        text: &str,
        to: Voice,
        starter: Starter,
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
        // What the thread says before the message: a stage it moved to, and who takes that stage.
        let mut thread = thread;
        let mut early: Vec<Entry> = Vec::new();
        let mut remembered: Option<(Stage, Agent)> = None;
        match (&starter, to) {
            (Starter::Person, Voice::Author) => {
                // A thread starting over from Done gets its first stage's agent, as a new thread does.
                let workflow = self.inner.workflow_of(&thread);
                let next = stage_after(&workflow, thread.stage, Happening::PersonWrote);
                let pick = match next.and_then(|stage| {
                    self.inner
                        .stage_pick(Some(&thread), stage, None, None)
                        .map(|pick| (stage, pick))
                }) {
                    Some((stage, Ok(pick))) => Some((stage, pick)),
                    Some((_, Err(problem))) => {
                        return Err(WorkspaceError::Invalid(problem.to_string()))
                    }
                    None => None,
                };
                if let Some((entry, after)) = self.inner.happen(
                    store,
                    &thread,
                    Happening::PersonWrote,
                    Mover::Person("you wrote again".to_string()),
                )? {
                    early.push(entry);
                    thread = after;
                }
                if let Some((stage, pick)) = pick {
                    let (entries, after) =
                        self.inner.hand_over(store, &thread, stage, &pick, true)?;
                    early.extend(entries);
                    thread = after;
                    remembered = Some((stage, pick.agent));
                }
            }
            (Starter::Teryon { stage, pick }, _) => {
                let (entries, after) =
                    self.inner
                        .hand_over(store, &thread, *stage, pick, to == Voice::Author)?;
                early.extend(entries);
                thread = after;
                remembered = Some((*stage, pick.agent));
            }
            (Starter::Person, Voice::Reviewer) => {}
        }
        let message = handover_message(store, &thread, to, text)?;
        let mut draft = EntryDraft::user_message(text).by(to);
        if to == Voice::Author {
            // Which agent a message went to tells the history whose work followed, see `history::turns`.
            draft.body["agent"] = thread.agent.clone().into();
        }
        if let Starter::Teryon { .. } = starter {
            draft.body["automatic"] = true.into();
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
            match starter {
                Starter::Person => {
                    thread.asking = false;
                    thread.rounds = 0;
                    thread.held = false;
                }
                Starter::Teryon { .. } => thread.rounds += 1,
            }
            if let Some((stage, agent)) = remembered {
                thread.picks.insert(stage, agent.name().to_string());
            }
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

        for early in early {
            self.inner.announce(Event::EntryAdded { entry: early });
        }
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
        self.review(thread_id, agent, model, Starter::Person)
    }

    /// Starts a review, asked for by the person or by Teryon when the thread reached its review stage.
    fn review(
        &self,
        thread_id: Uuid,
        agent: Agent,
        model: Option<String>,
        starter: Starter,
    ) -> Result<Entry, WorkspaceError> {
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
        let taking = match &starter {
            Starter::Teryon { stage, pick } => {
                self.inner.hand_over(store, &thread, *stage, pick, false)?.0
            }
            Starter::Person => Vec::new(),
        };
        let entry = store.append(thread_id, notice.kind.name(), body)?;
        let moved = match starter {
            Starter::Person => self.inner.happen(
                store,
                &thread,
                Happening::ReviewAsked,
                Mover::Person("you asked for a review".to_string()),
            )?,
            Starter::Teryon { .. } => None,
        };
        let thread = store.change(thread_id, |thread| {
            thread.status = Status::Running;
            if let Starter::Teryon { stage, .. } = &starter {
                thread.rounds += 1;
                thread.picks.insert(*stage, agent.name().to_string());
            }
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

        for taking in taking {
            self.inner.announce(Event::EntryAdded { entry: taking });
        }
        self.inner.announce(Event::EntryAdded {
            entry: entry.clone(),
        });
        if let Some((stage_entry, _)) = moved {
            self.inner
                .announce(Event::EntryAdded { entry: stage_entry });
        }
        self.start_turn(thread, Voice::Reviewer, directory, &message);
        Ok(entry)
    }

    /// Tells everyone a thread is now running, and starts one of its agents.
    ///
    /// The reviewer runs inside the change's copy, so that a file it writes
    /// where it stands lands where the check after its turn sees it, not in
    /// the project folder the person may be working in.
    fn start_turn(&self, thread: Thread, voice: Voice, directory: PathBuf, message: &str) {
        // The plan is in the project, wherever the agent runs.
        let plan = plan_path(&directory, thread.id);
        let directory = match (voice, thread.change.as_deref()) {
            (Voice::Reviewer, Some(change)) => change.path.clone(),
            _ => directory,
        };
        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        self.inner
            .turns()
            .insert(thread.id, TurnStart::of(voice, thread.stage));
        let (agents, environment, workflows, workflow) = {
            let setup = self.inner.setup();
            (
                setup.agents.clone(),
                setup
                    .tools
                    .as_ref()
                    .map(AgentTools::environment)
                    .unwrap_or_default(),
                setup.workflows.clone(),
                setup.workflows.of(&thread.workflow).clone(),
            )
        };
        let staging = match voice {
            Voice::Author => {
                let choosing = thread.stage == workflow.first()
                    && thread.change.is_none()
                    && thread.finished.is_none();
                let stage = staging(&workflows, &workflow, thread.stage, choosing);
                Some(
                    match planning(&plan, &workflow, thread.stage, thread.plan.is_some()) {
                        Some(planned) => format!("{stage} {planned}"),
                        None => stage,
                    },
                )
            }
            Voice::Reviewer => thread.plan.is_some().then(|| reviewing_plan(&plan)),
        };
        self.inner.runner.start(
            Arc::new(WorkspaceJournal(self.inner.clone())),
            agents,
            TurnRequest {
                thread,
                voice,
                directory,
                message: message.to_string(),
                environment,
                staging,
            },
        );
    }

    /// Asks a thread's agent to stop. Does nothing when it is not running.
    ///
    /// A stopped thread also starts nothing by itself until the person writes
    /// again: they stopped it for a reason, and the next stage's agent
    /// starting a moment later would undo that.
    pub fn stop(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        let held = match thread.status == Status::Running && !thread.held {
            true => Some(store.change(thread_id, |thread| thread.held = true)?),
            false => None,
        };
        drop(state);
        if let Some(thread) = held {
            self.inner.announce(Event::ThreadChanged { thread });
        }
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
                // The person's choice is kept for the stage, until the thread moves on.
                thread.picks.insert(thread.stage, agent.name().to_string());
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
        self.inner.stopping.store(true, Ordering::SeqCst);
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
        if let ThreadChange::RanOut(agent) = change {
            if let Some(start) = self.0.turns().get_mut(&thread_id) {
                start.ran_out = Some(agent);
            }
            return;
        }
        let finished = change == ThreadChange::Finished;
        // The plan is read before the thread shows as done, so its line comes before the person is called.
        let authored = self
            .0
            .turns()
            .get(&thread_id)
            .is_some_and(|start| start.voice == Voice::Author);
        if finished && authored {
            self.0.look_at_plan(thread_id);
        }
        let reviewed = match &change {
            ThreadChange::Reviewed(findings) => Some(*findings),
            _ => None,
        };
        let mut state = self.0.state();
        let changed = state
            .holding(thread_id)
            .map_err(|problem| problem.to_string())
            .and_then(|(store, _)| {
                let thread = store
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
                        ThreadChange::RanOut(_) => {}
                    })
                    .map_err(|problem| problem.to_string())?;
                let moved = match reviewed {
                    Some(findings) => self
                        .0
                        .happen(
                            store,
                            &thread,
                            Happening::Reviewed(findings),
                            Mover::Teryon(findings_text(findings)),
                        )
                        .map_err(|problem| problem.to_string())?,
                    None => None,
                };
                Ok((thread, moved))
            });
        drop(state);
        match changed {
            Ok((thread, moved)) => {
                self.0.announce(Event::ThreadChanged { thread });
                self.0.announce_moved(moved);
            }
            Err(problem) => error!("could not store a change to a thread: {problem}"),
        }
        if finished {
            let start = self.0.turns().remove(&thread_id);
            if let Some(start) = start {
                Workspace {
                    inner: self.0.clone(),
                }
                .carry_on(thread_id, start);
            }
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
        let book = self.0.usage();
        let recorded = line.map(|line| book.record(&line).map(|()| line));
        drop(book);
        match recorded {
            Some(Ok(line)) => self.0.announce(Event::UsageRecorded { line }),
            Some(Err(problem)) => error!("could not add a turn to the usage log: {problem}"),
            None => {}
        }
        self.0
            .note_limits(usage.agent, usage.reading, usage.hit_limit, now);
    }
}

/// Whether the menu can offer the editor and the terminal.
fn opens_of(open: &OpenSettings) -> Opens {
    Opens {
        editor: open.editor.is_some(),
        terminal: open.terminal.is_some(),
    }
}

/// The settings the screen owns, as `Configuration` reads them.
///
/// Program paths, the port and the model lists stay out: the screen neither
/// shows nor saves them, and a later read must still see the live strengths.
fn view_of(setup: &Setup) -> Settings {
    let mut settings = Settings {
        workflow: setup.workflow.clone(),
        stages: setup.stages.clone(),
        open: setup.open.clone(),
        workflows: setup
            .own_workflows
            .then(|| workflow_definitions(&setup.workflows)),
        ..Settings::default()
    };
    settings.agents.claude.strong = setup.agents.claude.strong.clone();
    settings.agents.claude.medium = setup.agents.claude.medium.clone();
    settings.agents.claude.fast = setup.agents.claude.fast.clone();
    settings.agents.codex.strong = setup.agents.codex.strong.clone();
    settings.agents.codex.medium = setup.agents.codex.medium.clone();
    settings.agents.codex.fast = setup.agents.codex.fast.clone();
    settings.agents.grok.strong = setup.agents.grok.strong.clone();
    settings.agents.grok.medium = setup.agents.grok.medium.clone();
    settings.agents.grok.fast = setup.agents.grok.fast.clone();
    settings
}

/// Keeps a saved file in what the next decision reads.
///
/// Only the strengths move on the agents. Their programs, model lists, homes
/// and subscription switches were not on the screen, so a save must not
/// wipe a path the service started with.
fn apply_saved(setup: &mut Setup, settings: &Settings) {
    setup.agents.claude.strong = settings.agents.claude.strong.clone();
    setup.agents.claude.medium = settings.agents.claude.medium.clone();
    setup.agents.claude.fast = settings.agents.claude.fast.clone();
    setup.agents.codex.strong = settings.agents.codex.strong.clone();
    setup.agents.codex.medium = settings.agents.codex.medium.clone();
    setup.agents.codex.fast = settings.agents.codex.fast.clone();
    setup.agents.grok.strong = settings.agents.grok.strong.clone();
    setup.agents.grok.medium = settings.agents.grok.medium.clone();
    setup.agents.grok.fast = settings.agents.grok.fast.clone();
    setup.open = settings.open.clone();
    setup.workflow = settings.workflow.clone();
    setup.stages = settings.stages.clone();
    setup.workflows = Workflows::from_settings(settings);
    setup.own_workflows = settings.workflows.is_some();
}

/// The workflows the screen is showing, named for the settings file.
fn workflow_definitions(workflows: &Workflows) -> BTreeMap<String, WorkflowDefinition> {
    workflows
        .workflows
        .iter()
        .map(|workflow| {
            (
                workflow.name.clone(),
                WorkflowDefinition {
                    description: workflow.description.clone(),
                    stages: workflow.stages.clone(),
                },
            )
        })
        .collect()
}

/// Replaces `config.toml` only once the new text is fully on disk.
///
/// A crash mid-write would otherwise leave a half file, and the service
/// would refuse to start. Each save uses its own temporary file beside
/// `config.toml` and renames it over the real one, which replaces the whole
/// file or leaves the old one. A shared name would let one save truncate
/// the file another is about to rename, and that save would then be told
/// it failed even though its text had landed.
fn write_settings(folder: &Path, text: &str) -> Result<(), IoError> {
    create_dir_all(folder)?;
    let temporary = folder.join(writing_name());
    if let Err(problem) = write(&temporary, text) {
        let _ = remove_file(&temporary);
        return Err(problem);
    }
    if let Err(problem) = rename(&temporary, folder.join("config.toml")) {
        let _ = remove_file(&temporary);
        return Err(problem);
    }
    Ok(())
}

/// A temporary name for one save, never `config.toml.writing` itself.
///
/// The counter is per process. The save lock already keeps two saves in
/// this process apart; the name still has to differ so a file left behind
/// under the old shared name is not reused.
fn writing_name() -> String {
    static WRITING: AtomicU64 = AtomicU64::new(0);
    format!(
        "config.toml.{}-{}.writing",
        id(),
        WRITING.fetch_add(1, Ordering::Relaxed)
    )
}

/// "3 findings", for the line that moves a thread after a review.
fn findings_text(findings: Option<u32>) -> String {
    match findings {
        Some(0) => "no findings".to_string(),
        Some(1) => "1 finding".to_string(),
        Some(count) => format!("{count} findings"),
        None => "a review without a count".to_string(),
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
