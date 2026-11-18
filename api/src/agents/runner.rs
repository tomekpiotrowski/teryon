//! Runs agent programs and stores what they report.
//!
//! One program run serves one message. The run reads the program's report
//! line by line, hands each line to the agent's driver and appends the
//! resulting entries to the thread, which is how they reach the screen.
//!
//! The program is started in its own process group. An agent starts further
//! programs of its own, such as test runs, and stopping the agent has to stop
//! those too.

use std::collections::HashMap;
use std::env::current_dir;
use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use sea_orm::{ActiveModelTrait, DatabaseConnection, DbErr, IntoActiveModel, Set};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::sync::watch::{channel, Receiver, Sender};
use tokio::time::{interval, sleep_until, timeout, Instant as TokioInstant, MissedTickBehavior};
use tracing::{error, warn};
use uuid::Uuid;

use crate::agents::worktree::{instruction_for, observed_branch};
use crate::agents::{driver_for, Agent, AgentDriver, EntryDraft, EntryKind, Invocation, Turn};
use crate::settings::AgentSettings;
use crate::workspace::journal::Journal;
use crate::workspace::thread;

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

/// A message for an agent, together with where to run it.
pub struct TurnRequest {
    pub thread: thread::Model,
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
    /// The caller has already marked the thread as running. The thread is
    /// marked as idle again when the program has ended, whatever happened.
    pub fn start(&self, db: DatabaseConnection, settings: AgentSettings, request: TurnRequest) {
        let (stop, stopped) = channel(None);
        let thread = request.thread.clone();
        self.running().insert(thread.id, stop);

        let runner = self.clone();
        tokio::spawn(async move {
            run_turn(&db, &settings, request, stopped).await;
            // Forget the run before the thread accepts messages again, so a
            // new run for the same thread is never forgotten in its place.
            runner.running().remove(&thread.id);
            if let Err(problem) = mark_idle(&db, thread).await {
                error!("could not mark a thread as idle: {problem}");
            }
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

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Sender<Option<Stop>>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs the program for one message and writes everything it reports.
async fn run_turn(
    db: &DatabaseConnection,
    settings: &AgentSettings,
    request: TurnRequest,
    stopped: Receiver<Option<Stop>>,
) {
    let TurnRequest {
        thread,
        directory,
        message,
    } = request;

    let mut journal = match Journal::open(db, &thread).await {
        Ok(journal) => journal,
        Err(problem) => {
            error!("could not open the journal of a thread: {problem}");
            return;
        }
    };
    let Some(agent) = Agent::parse(&thread.agent) else {
        record(
            &mut journal,
            EntryDraft::error("This thread names an agent Teryon does not know."),
        )
        .await;
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
    let child = match spawn(&invocation, &directory) {
        Ok(child) => child,
        Err(problem) => {
            let text = format!("Could not start `{}`: {problem}.", invocation.program);
            record(&mut journal, EntryDraft::error(&text)).await;
            return;
        }
    };

    let grace = Duration::from_secs(settings.stop_grace_seconds);
    let outcome = follow(
        db,
        &thread,
        &directory,
        watch_worktree,
        &mut journal,
        driver,
        child,
        stopped,
        grace,
    )
    .await;

    match outcome.stop {
        Some(Stop::ByUser) => record(&mut journal, EntryDraft::notice("Stopped by you.")).await,
        Some(Stop::Shutdown) => {
            record(
                &mut journal,
                EntryDraft::notice("Interrupted by a restart."),
            )
            .await
        }
        None if !outcome.summarised => {
            let text = unfinished_turn(outcome.status, &outcome.complaints);
            record(&mut journal, EntryDraft::error(&text)).await;
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
async fn follow(
    db: &DatabaseConnection,
    thread: &thread::Model,
    directory: &Path,
    watch_worktree: bool,
    journal: &mut Journal,
    mut driver: Box<dyn AgentDriver>,
    mut child: Child,
    mut stopped: Receiver<Option<Stop>>,
    grace: Duration,
) -> Outcome {
    let started = Instant::now();
    let group = child.id();
    let complaints = tokio::spawn(read_end(child.stderr.take()));
    let mut lines = child
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
                remember_branch(db, thread, directory, &mut known_branch).await;
            }
            line = report.next_line() => {
                let Ok(Some(line)) = line else { break };
                if !conversation_started {
                    conversation_started = true;
                    if let Err(problem) = mark_conversation_started(db, thread.clone()).await {
                        error!("could not mark a conversation as started: {problem}");
                    }
                }
                match driver.interpret(&line) {
                    Ok(drafts) => {
                        for draft in drafts {
                            summarised |= draft.kind == EntryKind::TurnSummary;
                            record(journal, timed(draft, started)).await;
                        }
                    }
                    Err(_) => warn!("skipped a line of an agent's report that was not understood: {line}"),
                }
                // The programs name their commands every turn, some several
                // times. Only a list that differs is stored, so that a turn
                // does not send the same thread to every device again and again.
                if let Some(commands) = driver.take_commands().map(|names| json!(names)) {
                    if commands != known_commands {
                        known_commands = commands.clone();
                        if let Err(problem) = store_commands(db, thread.clone(), commands).await {
                            error!("could not store the commands an agent takes: {problem}");
                        }
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
                        let signal = if reason == Stop::ByUser { Signal::SIGINT } else { Signal::SIGTERM };
                        signal_group(group, signal);
                        kill_at = Some(TokioInstant::now() + grace);
                    }
                    (Ok(()), None) => {}
                    (Err(_), _) => listening = false,
                }
            }
            () = sleep_until(kill_at.unwrap_or_else(TokioInstant::now)), if kill_at.is_some() => {
                kill_at = None;
                signal_group(group, Signal::SIGKILL);
            }
        }
    }

    if watch_worktree {
        remember_branch(db, thread, directory, &mut known_branch).await;
    }

    let status = match timeout(grace, child.wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            signal_group(group, Signal::SIGKILL);
            child.wait().await.ok()
        }
    };
    for draft in driver.finish() {
        record(journal, draft).await;
    }

    Outcome {
        summarised,
        stop,
        status,
        complaints: complaints.await.unwrap_or_default(),
    }
}

/// Starts the program in the project directory, in a process group of its own.
fn spawn(invocation: &Invocation, directory: &Path) -> Result<Child, IoError> {
    let mut command = Command::new(program_path(&invocation.program));
    command
        .args(&invocation.arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    for variable in &invocation.removed_variables {
        command.env_remove(variable);
    }
    command.spawn()
}

/// Makes a program given as a relative path independent of the project directory.
///
/// The program is started inside the project directory, where a relative path
/// from the configuration would point somewhere else. A bare name such as
/// `claude` is left alone and found through `PATH`.
fn program_path(program: &str) -> PathBuf {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() < 2 {
        return path.to_path_buf();
    }
    current_dir().map_or_else(|_| path.to_path_buf(), |directory| directory.join(path))
}

fn signal_group(group: Option<u32>, signal: Signal) {
    let Some(group) = group.and_then(|group| i32::try_from(group).ok()) else {
        return;
    };
    // The group is gone when the program has already ended; nothing to do then.
    let _ = killpg(Pid::from_raw(group), signal);
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

async fn record(journal: &mut Journal, draft: EntryDraft) {
    if let Err(problem) = journal.add(draft).await {
        error!("could not store a thread entry: {problem}");
    }
}

async fn mark_conversation_started(
    db: &DatabaseConnection,
    thread: thread::Model,
) -> Result<(), DbErr> {
    let mut thread = thread.into_active_model();
    thread.conversation_started = Set(true);
    thread.update(db).await.map(|_| ())
}

/// Stores the branch checked out for a thread when it has changed.
///
/// A failure to read the worktrees leaves the stored branch as it was: not
/// being able to ask git is different from the checkout being gone.
async fn remember_branch(
    db: &DatabaseConnection,
    thread: &thread::Model,
    directory: &Path,
    known: &mut Option<String>,
) {
    let found = match observed_branch(directory, thread.id).await {
        Ok(found) => found,
        Err(problem) => {
            warn!("could not read worktrees: {problem}");
            return;
        }
    };
    if found == *known {
        return;
    }
    let mut current = thread.clone().into_active_model();
    current.branch = Set(found.clone());
    match current.update(db).await {
        Ok(_) => *known = found,
        Err(problem) => warn!("could not store a thread's branch: {problem}"),
    }
}

async fn store_commands(
    db: &DatabaseConnection,
    thread: thread::Model,
    commands: Value,
) -> Result<(), DbErr> {
    let mut thread = thread.into_active_model();
    thread.commands = Set(commands);
    thread.update(db).await.map(|_| ())
}

/// Marks a thread as waiting for a message again, and as not yet looked at.
///
/// However the agent ended, the person has something new to see.
async fn mark_idle(db: &DatabaseConnection, thread: thread::Model) -> Result<(), DbErr> {
    let mut thread = thread.into_active_model();
    thread.status = Set(thread::IDLE.to_string());
    thread.unread = Set(true);
    thread.update(db).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use lets_expect::lets_expect;

    use super::*;

    fn exit_code(code: i32) -> Option<ExitStatus> {
        Some(ExitStatus::from_raw(code << 8))
    }

    fn killed() -> Option<ExitStatus> {
        Some(ExitStatus::from_raw(9))
    }

    lets_expect! {
        expect(program_path(program)) {
            when(program = "claude") as a_bare_name to be_left_to_the_search_path {
                equal(PathBuf::from("claude"))
            }
            when(program = "/usr/bin/claude") as an_absolute_path to be_left_alone {
                equal(PathBuf::from("/usr/bin/claude"))
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
