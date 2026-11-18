//! Runs agent programs and records what they report.
//!
//! One program run serves one message. The run reads the program's report
//! line by line, hands each line to the agent's driver and gives the
//! resulting entries to a [`Journal`], which stores them and tells whoever is
//! watching. The runner itself knows nothing about where threads are kept.
//!
//! The program is started as the head of a process tree, see
//! [`crate::agents::process`]. An agent starts further programs of its own,
//! such as test runs, and stopping the agent has to stop those too.

use std::collections::HashMap;
use std::env::current_dir;
use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, Command};
use tokio::sync::watch::{channel, Receiver, Sender};
use tokio::time::{interval, sleep_until, timeout, Instant as TokioInstant, MissedTickBehavior};
use tracing::warn;
use uuid::Uuid;

use crate::agents::process::{ProcessTree, Signal};
use crate::agents::worktree::{instruction_for, observed_branch};
use crate::agents::{driver_for, Agent, AgentDriver, EntryDraft, EntryKind, Invocation, Turn};
use crate::settings::AgentSettings;
use crate::store::Thread;

/// How often a running turn looks for the worktree its agent may have created.
const CHECK_WORKTREE_EVERY: Duration = Duration::from_secs(1);

/// How much of a failed program's complaints is shown.
const LONGEST_COMPLAINT: usize = 2000;

/// Why a running agent is being stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// The person using Teryon asked for it.
    ByUser,
    /// Teryon itself is shutting down.
    Shutdown,
}

/// Where a turn's results go.
///
/// Storing can fail; the journal logs that itself, because a turn has no one
/// to tell and must go on either way.
pub trait Journal: Send + Sync + 'static {
    /// Adds an entry to the end of a thread.
    fn add(&self, thread_id: Uuid, draft: EntryDraft);

    /// Changes a fact about a thread.
    fn note(&self, thread_id: Uuid, change: ThreadChange);
}

/// A fact about a thread that a turn learns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadChange {
    /// The agent program has seen the conversation, so later turns continue it.
    ConversationStarted,
    /// The slash commands the program says it takes.
    Commands(Vec<String>),
    /// The branch checked out in the thread's worktree, or nothing once it is gone.
    Branch(Option<String>),
    /// The program has ended, however it ended. The thread takes messages again
    /// and needs the person's attention.
    Finished,
}

/// A message for an agent, together with where to run it.
pub struct TurnRequest {
    pub thread: Thread,
    pub directory: PathBuf,
    pub message: String,
}

/// Keeps track of the agents that are running, so they can be stopped.
#[derive(Clone, Default)]
pub struct Runner {
    running: Arc<Mutex<HashMap<Uuid, Sender<Option<Stop>>>>>,
}

impl Runner {
    /// Starts an agent for one message and returns immediately.
    ///
    /// The caller has already marked the thread as running. The journal hears
    /// [`ThreadChange::Finished`] when the program has ended, whatever happened.
    pub fn start(&self, journal: Arc<dyn Journal>, settings: AgentSettings, request: TurnRequest) {
        let (stop, stopped) = channel(None);
        let thread_id = request.thread.id;
        self.running().insert(thread_id, stop);

        let runner = self.clone();
        tokio::spawn(async move {
            run_turn(journal.as_ref(), &settings, request, stopped).await;
            // Forget the run before the thread accepts messages again, so a
            // new run for the same thread is never forgotten in its place.
            runner.running().remove(&thread_id);
            journal.note(thread_id, ThreadChange::Finished);
        });
    }

    /// Asks the agent of a thread to stop. Does nothing when none is running.
    pub fn stop(&self, thread_id: Uuid) {
        if let Some(stop) = self.running().get(&thread_id) {
            let _ = stop.send(Some(Stop::ByUser));
        }
    }

    /// Stops every agent because Teryon is shutting down.
    pub fn stop_all(&self) {
        for stop in self.running().values() {
            let _ = stop.send(Some(Stop::Shutdown));
        }
    }

    /// Whether any agent is still running.
    pub fn busy(&self) -> bool {
        !self.running().is_empty()
    }

    fn running(&self) -> MutexGuard<'_, HashMap<Uuid, Sender<Option<Stop>>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs the program for one message and records everything it reports.
async fn run_turn(
    journal: &dyn Journal,
    settings: &AgentSettings,
    request: TurnRequest,
    stopped: Receiver<Option<Stop>>,
) {
    let TurnRequest {
        thread,
        directory,
        message,
    } = request;

    let Some(agent) = Agent::parse(&thread.agent) else {
        journal.add(
            thread.id,
            EntryDraft::error("This thread names an agent Teryon does not know."),
        );
        return;
    };

    let driver = driver_for(agent, settings);
    let brief = instruction_for(&directory, thread.id).await;
    let watch_worktree = brief.is_some();
    let invocation = driver.invocation(&Turn {
        message,
        conversation: thread.conversation_id,
        first: !thread.conversation_started,
        brief,
    });
    let tree = match spawn(&invocation, &directory) {
        Ok(tree) => tree,
        Err(problem) => {
            let text = format!("Could not start `{}`: {problem}.", invocation.program);
            journal.add(thread.id, EntryDraft::error(&text));
            return;
        }
    };

    let grace = Duration::from_secs(settings.stop_grace_seconds);
    let outcome = follow(
        journal,
        &thread,
        &directory,
        watch_worktree,
        driver,
        tree,
        stopped,
        grace,
    )
    .await;

    match outcome.stop {
        Some(Stop::ByUser) => journal.add(thread.id, EntryDraft::notice("Stopped by you.")),
        Some(Stop::Shutdown) => {
            journal.add(thread.id, EntryDraft::notice("Interrupted by a restart."))
        }
        None if !outcome.summarised => {
            let text = unfinished_turn(outcome.status, &outcome.complaints);
            journal.add(thread.id, EntryDraft::error(&text));
        }
        None => {}
    }
}

/// What became of a program run.
struct Outcome {
    /// Whether the program reported the end of its turn.
    summarised: bool,
    stop: Option<Stop>,
    status: Option<ExitStatus>,
    /// The end of what the program wrote to its error output.
    complaints: String,
}

/// Reads the program's report until it ends, stopping it when asked to.
#[allow(clippy::too_many_arguments)]
async fn follow(
    journal: &dyn Journal,
    thread: &Thread,
    directory: &Path,
    watch_worktree: bool,
    mut driver: Box<dyn AgentDriver>,
    mut tree: ProcessTree,
    mut stopped: Receiver<Option<Stop>>,
    grace: Duration,
) -> Outcome {
    let started = Instant::now();
    let complaints = tokio::spawn(read_end(tree.child().stderr.take()));
    let mut lines = tree
        .child()
        .stdout
        .take()
        .map(|output| BufReader::new(output).lines());

    let mut summarised = false;
    let mut conversation_started = thread.conversation_started;
    let mut known_commands = thread.commands.clone();
    let mut known_branch = thread.branch.clone();
    let mut stop = None;
    let mut listening = true;
    let mut kill_at: Option<TokioInstant> = None;
    let mut ticks = interval(CHECK_WORKTREE_EVERY);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    while let Some(report) = lines.as_mut() {
        tokio::select! {
            _ = ticks.tick(), if watch_worktree => {
                remember_branch(journal, thread.id, directory, &mut known_branch).await;
            }
            line = report.next_line() => {
                let Ok(Some(line)) = line else { break };
                if !conversation_started {
                    conversation_started = true;
                    journal.note(thread.id, ThreadChange::ConversationStarted);
                }
                match driver.interpret(&line) {
                    Ok(drafts) => {
                        for draft in drafts {
                            summarised |= draft.kind == EntryKind::TurnSummary;
                            journal.add(thread.id, timed(draft, started));
                        }
                    }
                    Err(_) => warn!("skipped a line of an agent's report that was not understood: {line}"),
                }
                // The programs name their commands every turn, some several
                // times. Only a list that differs is stored, so that a turn
                // does not send the same thread to every client again and again.
                if let Some(commands) = driver.take_commands() {
                    if commands != known_commands {
                        known_commands = commands.clone();
                        journal.note(thread.id, ThreadChange::Commands(commands));
                    }
                }
            }
            changed = stopped.changed(), if listening => {
                let reason = *stopped.borrow();
                match (changed, reason) {
                    (Ok(()), Some(reason)) => {
                        listening = false;
                        stop = Some(reason);
                        // Claude Code ends its turn on an interrupt and exits at once
                        // on a termination request, which is what shutdown needs.
                        let signal = if reason == Stop::ByUser { Signal::Interrupt } else { Signal::Terminate };
                        tree.signal(signal);
                        kill_at = Some(TokioInstant::now() + grace);
                    }
                    (Ok(()), None) => {}
                    (Err(_), _) => listening = false,
                }
            }
            () = sleep_until(kill_at.unwrap_or_else(TokioInstant::now)), if kill_at.is_some() => {
                kill_at = None;
                tree.signal(Signal::Kill);
            }
        }
    }

    if watch_worktree {
        remember_branch(journal, thread.id, directory, &mut known_branch).await;
    }

    let status = match timeout(grace, tree.child().wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            tree.signal(Signal::Kill);
            tree.child().wait().await.ok()
        }
    };
    for draft in driver.finish() {
        journal.add(thread.id, draft);
    }

    Outcome {
        summarised,
        stop,
        status,
        complaints: complaints.await.unwrap_or_default(),
    }
}

/// Starts the program in the project directory, at the head of a process tree.
fn spawn(invocation: &Invocation, directory: &Path) -> Result<ProcessTree, IoError> {
    let mut command = Command::new(program_path(&invocation.program));
    command
        .args(&invocation.arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for variable in &invocation.removed_variables {
        command.env_remove(variable);
    }
    ProcessTree::start(command)
}

/// Makes a program given as a relative path independent of the project directory.
///
/// The program is started inside the project directory, where a relative path
/// from the configuration would point somewhere else. A bare name such as
/// `claude` is found through `PATH`; on Windows that includes `claude.cmd`,
/// which is how programs installed with npm arrive there.
fn program_path(program: &str) -> PathBuf {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if path.components().count() < 2 {
        return search_path(program).unwrap_or_else(|| path.to_path_buf());
    }
    current_dir().map_or_else(|_| path.to_path_buf(), |directory| directory.join(path))
}

/// Finds a program on Windows, where the search Teryon relies on elsewhere
/// only looks for `.exe` files. Elsewhere the system finds programs itself.
#[cfg(windows)]
fn search_path(program: &str) -> Option<PathBuf> {
    let folders = std::env::var_os("PATH")?;
    std::env::split_paths(&folders).find_map(|folder| {
        windows_names(program)
            .into_iter()
            .map(|name| folder.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(not(windows))]
fn search_path(_program: &str) -> Option<PathBuf> {
    None
}

/// The file names a program may have on Windows, in the order Windows prefers.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_names(program: &str) -> Vec<String> {
    if Path::new(program).extension().is_some() {
        return vec![program.to_string()];
    }
    ["exe", "cmd", "bat"]
        .iter()
        .map(|extension| format!("{program}.{extension}"))
        .collect()
}

/// Adds how long the turn has taken to its summary.
fn timed(mut draft: EntryDraft, started: Instant) -> EntryDraft {
    if draft.kind == EntryKind::TurnSummary {
        let milliseconds = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        draft.body["duration_ms"] = json!(milliseconds);
    }
    draft
}

/// Explains a program that ended without reporting the end of its turn.
fn unfinished_turn(status: Option<ExitStatus>, complaints: &str) -> String {
    let complaints = complaints.trim();
    if !complaints.is_empty() {
        return complaints.to_string();
    }
    match status.and_then(|status| status.code()) {
        Some(code) => format!("The agent ended without finishing its turn (exit code {code})."),
        None => "The agent ended without finishing its turn.".to_string(),
    }
}

/// Reads everything a program writes to its error output and keeps the end.
async fn read_end(output: Option<ChildStderr>) -> String {
    let mut text = String::new();
    if let Some(mut output) = output {
        let mut bytes = Vec::new();
        let _ = output.read_to_end(&mut bytes).await;
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    let length = text.chars().count();
    if length > LONGEST_COMPLAINT {
        text = text.chars().skip(length - LONGEST_COMPLAINT).collect();
    }
    text
}

/// Notes the branch checked out for a thread when it has changed.
///
/// A failure to read the worktrees leaves the branch as it was: not being
/// able to ask git is different from the checkout being gone.
async fn remember_branch(
    journal: &dyn Journal,
    thread_id: Uuid,
    directory: &Path,
    known: &mut Option<String>,
) {
    let found = match observed_branch(directory, thread_id).await {
        Ok(found) => found,
        Err(problem) => {
            warn!("could not read worktrees: {problem}");
            return;
        }
    };
    if found != *known {
        *known = found.clone();
        journal.note(thread_id, ThreadChange::Branch(found));
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    use lets_expect::lets_expect;

    use super::*;

    #[cfg(unix)]
    fn exit_code(code: i32) -> Option<ExitStatus> {
        Some(ExitStatus::from_raw(code << 8))
    }

    #[cfg(windows)]
    fn exit_code(code: i32) -> Option<ExitStatus> {
        Some(ExitStatus::from_raw(code as u32))
    }

    /// A program ended by a signal, which has no exit code. Windows has no such thing.
    #[cfg(unix)]
    fn killed() -> Option<ExitStatus> {
        Some(ExitStatus::from_raw(9))
    }

    #[cfg(windows)]
    fn killed() -> Option<ExitStatus> {
        None
    }

    #[cfg(unix)]
    const ABSOLUTE: &str = "/usr/bin/claude";
    #[cfg(windows)]
    const ABSOLUTE: &str = r"C:\Programs\claude.exe";

    lets_expect! {
        expect(windows_names(program)) {
            when(program = "claude") as a_bare_name to try_every_kind_of_program {
                equal(vec!["claude.exe".to_string(), "claude.cmd".to_string(), "claude.bat".to_string()])
            }
            when(program = "claude.cmd") as a_name_with_its_extension to be_taken_as_it_is {
                equal(vec!["claude.cmd".to_string()])
            }
        }

        expect(program_path(program)) {
            when(program = "a-program-nobody-has") as a_bare_name_not_found to be_left_to_the_system {
                equal(PathBuf::from("a-program-nobody-has"))
            }
            when(program = ABSOLUTE) as an_absolute_path to be_left_alone {
                equal(PathBuf::from(ABSOLUTE))
            }
            when(program = "tests/samples/fake-claude.sh") as a_relative_path to start_from_where_teryon_runs {
                equal(current_dir().unwrap().join("tests/samples/fake-claude.sh"))
            }
        }

        expect(unfinished_turn(status, complaints)) {
            // requirement: a program that ends with a failure adds an error entry with its message
            when(status = exit_code(3), complaints = "  not signed in\n") as the_program_complained to show_its_message {
                equal("not signed in".to_string())
            }
            when(status = exit_code(3), complaints = "") as the_program_was_silent to name_the_exit_code {
                equal("The agent ended without finishing its turn (exit code 3).".to_string())
            }
            when(status = killed(), complaints = " ") as the_program_was_killed to say_it_did_not_finish {
                equal("The agent ended without finishing its turn.".to_string())
            }
            when(status = None, complaints = "") as nothing_is_known to say_it_did_not_finish {
                equal("The agent ended without finishing its turn.".to_string())
            }
        }

        expect(timed(draft, Instant::now()).body["duration_ms"].is_u64()) {
            when(draft = EntryDraft::turn_summary(None, None)) as a_turn_summary to carry_a_duration { be_true }
            when(draft = EntryDraft::notice("Hello")) as another_entry to be_left_alone { be_false }
        }
    }
}
