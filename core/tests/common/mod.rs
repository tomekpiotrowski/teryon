//! Shared helpers for the core library's tests that run agents and read repositories.
//!
//! The agent programs are replaced by `tests/samples/fake-agent.sh`, which
//! replays recordings of the real programs and misbehaves on request. Nothing
//! here reaches a model provider. Each test gets its own data folder and its
//! own project folders, so tests never touch a person's real projects.
//!
//! Each test file compiles this module for itself and uses only part of it.
#![allow(dead_code)]

use std::fs::{create_dir_all, read_to_string, write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tempfile::{tempdir, TempDir};
use tokio::time::sleep;
use uuid::Uuid;

use teryon_core::protocol::{Event, ProjectView};
use teryon_core::settings::{AgentSettings, ClaudeSettings, GrokSettings};
use teryon_core::store::{Entry, Status, Thread};
use teryon_core::workspace::{Setup, Workspace};

/// Settings that start the fake agents, and give a stopped agent one second.
pub fn fake_agents() -> AgentSettings {
    AgentSettings {
        stop_grace_seconds: 1,
        claude: ClaudeSettings {
            program: "tests/samples/fake-claude.sh".to_string(),
            use_subscription: true,
        },
        grok: GrokSettings {
            program: "tests/samples/fake-grok.sh".to_string(),
        },
    }
}

/// Settings that start a given program in place of Claude.
pub fn settings_with(program: &str) -> AgentSettings {
    AgentSettings {
        claude: ClaudeSettings {
            program: program.to_string(),
            use_subscription: true,
        },
        ..fake_agents()
    }
}

/// What a workspace in tests is opened with: the given agents, nothing to open copies with, and no `teryon` program.
pub fn setup(settings: AgentSettings) -> Setup {
    Setup {
        agents: settings,
        ..Setup::default()
    }
}

/// A fresh directory that may become a project.
pub fn project_folder() -> TempDir {
    tempdir().expect("a project folder")
}

/// Runs git in a directory and returns what it printed, without the line ending.
///
/// The repositories made for tests ignore the settings of whoever runs them.
pub fn git(directory: &Path, arguments: &[&str]) -> String {
    run_git(directory, None, arguments)
}

/// Runs git as if it were the given moment, in seconds since 1970.
///
/// A commit made this way carries that moment as the time it was made.
pub fn git_at(directory: &Path, time: i64, arguments: &[&str]) -> String {
    run_git(directory, Some(time), arguments)
}

fn run_git(directory: &Path, time: Option<i64>, arguments: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Tester")
        .env("GIT_AUTHOR_EMAIL", "tester@example.com")
        .env("GIT_COMMITTER_NAME", "Tester")
        .env("GIT_COMMITTER_EMAIL", "tester@example.com");
    if let Some(time) = time {
        command
            .env("GIT_AUTHOR_DATE", format!("{time} +0000"))
            .env("GIT_COMMITTER_DATE", format!("{time} +0000"));
    }
    let output = command.output().expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Makes a folder a repository on `main` with one commit.
pub fn repository(folder: &TempDir) {
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), "README.md", "hello\n");
    commit(folder.path());
}

/// Writes a file in a directory, making the folders on the way.
pub fn put(directory: &Path, path: &str, content: impl AsRef<[u8]>) {
    let file = directory.join(path);
    create_dir_all(file.parent().expect("a folder")).expect("the folder is made");
    write(file, content).expect("the file is written");
}

/// Commits everything in a directory as it is.
pub fn commit(directory: &Path) {
    git(directory, &["add", "-A"]);
    git(directory, &["commit", "-q", "-m", "change"]);
}

/// How the fake agent program was started in a project folder, one line per run.
pub fn invocations(folder: &TempDir) -> Vec<String> {
    read_to_string(folder.path().join(".fake-agent-invocations"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

pub fn kinds(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// An open workspace in a data folder of its own.
pub struct Scene {
    pub home: TempDir,
    pub setup: Setup,
    pub workspace: Workspace,
}

impl Scene {
    pub fn new() -> Self {
        Self::with(fake_agents())
    }

    pub fn with(settings: AgentSettings) -> Self {
        Self::with_setup(setup(settings))
    }

    pub fn with_setup(setup: Setup) -> Self {
        let home = tempdir().expect("a data folder");
        let workspace = Workspace::open(home.path(), setup.clone()).expect("the workspace opens");
        Self {
            home,
            setup,
            workspace,
        }
    }

    /// Closes the workspace, changes files while it is closed, and opens it again, as after a restart.
    pub fn restart(self, meanwhile: impl FnOnce(&Path)) -> Self {
        let Self {
            home,
            setup,
            workspace,
        } = self;
        drop(workspace);
        meanwhile(home.path());
        let workspace =
            Workspace::open(home.path(), setup.clone()).expect("the workspace opens again");
        Self {
            home,
            setup,
            workspace,
        }
    }

    pub fn project(&self, folder: &TempDir) -> ProjectView {
        self.workspace
            .add_project("Sample", &folder.path().to_string_lossy())
            .expect("the project is added")
    }

    pub fn thread(&self, project: &ProjectView, agent: &str) -> Thread {
        self.workspace
            .start_thread(project.id, agent, "")
            .expect("the thread is started")
    }

    pub fn say(&self, thread: &Thread, text: &str) -> Result<Entry, String> {
        self.workspace
            .send_message(thread.id, text)
            .map_err(|problem| problem.to_string())
    }

    pub fn projects(&self) -> Vec<ProjectView> {
        match self.workspace.snapshot() {
            Event::Snapshot { projects, .. } => projects,
            _ => Vec::new(),
        }
    }

    pub fn threads(&self) -> Vec<Thread> {
        match self.workspace.snapshot() {
            Event::Snapshot { threads, .. } => threads,
            _ => Vec::new(),
        }
    }

    pub fn thread_now(&self, thread: &Thread) -> Thread {
        self.threads()
            .into_iter()
            .find(|candidate| candidate.id == thread.id)
            .expect("the thread is still there")
    }

    pub fn entries(&self, thread: &Thread) -> Vec<Entry> {
        self.workspace.entries(thread.id).expect("the entries")
    }

    /// The kinds of a thread's entries, in order.
    pub fn kinds(&self, thread: &Thread) -> Vec<String> {
        self.entries(thread)
            .into_iter()
            .map(|entry| entry.kind)
            .collect()
    }

    /// The text of the entry at a position, counted from 1.
    pub fn text_at(&self, thread: &Thread, position: usize) -> String {
        self.entries(thread)[position - 1].body["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Waits for the agent of a thread to end. Fails when it takes too long.
    pub async fn wait_until_idle(&self, thread: &Thread) {
        for _ in 0..200 {
            if self.thread_now(thread).status == Status::Idle {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!("the agent did not end in time");
    }

    /// Waits until a thread is as `wanted` says, and returns it. Fails when it takes too long.
    pub async fn wait_for(&self, thread: &Thread, wanted: impl Fn(&Thread) -> bool) -> Thread {
        for _ in 0..200 {
            let current = self.thread_now(thread);
            if wanted(&current) {
                return current;
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "the thread did not get there in time: {:?}",
            self.thread_now(thread)
        );
    }

    /// Waits for the agent of a thread to report its first line.
    ///
    /// Stopping an agent before it has got going proves little, so tests that
    /// stop one wait for this first.
    pub async fn wait_until_reporting(&self, thread: &Thread) {
        for _ in 0..200 {
            if self.thread_now(thread).conversation_started {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!("the agent did not report anything in time");
    }
}

/// The file holding a thread's facts, inside its project.
pub fn thread_file(folder: &TempDir, thread: &Thread) -> PathBuf {
    folder
        .path()
        .join(".teryon/threads")
        .join(thread.id.to_string())
        .join("thread.json")
}

/// Marks a thread as running in its file, as if Teryon had stopped in the middle of a turn.
pub fn mark_running(folder: &TempDir, thread: &Thread) {
    let path = thread_file(folder, thread);
    let text = read_to_string(&path).expect("the thread file");
    write(&path, text.replace("\"idle\"", "\"running\"")).expect("the thread file is written");
}

pub fn unknown() -> Uuid {
    Uuid::new_v4()
}

/// What a file the fake agent writes in a project folder holds, once it is there. Fails when it takes too long.
pub async fn written_by_agent(folder: &TempDir, name: &str) -> String {
    for _ in 0..200 {
        if let Ok(text) = read_to_string(folder.path().join(name)) {
            if text.ends_with('\n') {
                return text.trim().to_string();
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the fake agent did not write {name} in time");
}

/// The turn the fake agent noted when told to "remember the turn".
pub async fn noted_turn(folder: &TempDir) -> Uuid {
    let noted = written_by_agent(folder, ".fake-agent-turn").await;
    let turn = noted.split(' ').nth(1).expect("the turn is noted");
    Uuid::parse_str(turn).expect("the turn is an identifier")
}
