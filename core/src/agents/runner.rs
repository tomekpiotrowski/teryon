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
use std::ffi::OsString;
use std::fs::canonicalize;
use std::future::Future;
use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{from_value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, Command};
use tokio::sync::watch::{channel, Receiver, Sender};
use tokio::task::spawn_blocking;
use tokio::time::{interval, sleep_until, timeout, Instant as TokioInstant, MissedTickBehavior};
use tracing::warn;
use uuid::Uuid;

use crate::agents::asking::with_asking;
use crate::agents::naming::with_naming;
use crate::agents::process::{ProcessTree, Signal};
use crate::agents::{
    driver_for, Agent, AgentDriver, EntryDraft, EntryKind, Invocation, Tokens, Turn, Voice,
};
use crate::changes::instruction::instruction_for;
use crate::changes::reviewing::{findings_in, fingerprint, reviewing};
use crate::changes::status::Look;
use crate::files::git::{first_line, Git};
use crate::settings::AgentSettings;
use crate::store::Thread;
use crate::usage::grok::{grok_home_here, grok_reading};
use crate::usage::limits::{codex_home_here, codex_reading, gemini_used_up};
use crate::usage::{says_limit_hit, Limits, Reading};

/// How often a running turn has its thread's change looked at.
const LOOK_AT_CHANGE_EVERY: Duration = Duration::from_secs(1);

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

/// Work that finishes later, handed back by a [`Journal`].
pub type Pending<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Where a turn's results go.
///
/// Storing can fail; the journal logs that itself, because a turn has no one
/// to tell and must go on either way.
pub trait Journal: Send + Sync + 'static {
    /// Adds an entry to the end of a thread.
    fn add(&self, thread_id: Uuid, draft: EntryDraft);

    /// Changes a fact about a thread.
    fn note(&self, thread_id: Uuid, change: ThreadChange);

    /// Reads where the thread's change stands, or finds the copy its agent made by hand.
    ///
    /// Knowing that takes the whole project's threads, which the runner does not have.
    fn look(&self, thread_id: Uuid, look: Look) -> Pending<'_>;

    /// Records what a turn used, and what the agent's program said about its limits.
    fn usage(&self, thread_id: Uuid, usage: TurnUsage);
}

/// What one turn used of its agent's subscription.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnUsage {
    pub agent: Agent,
    /// Tokens read and written. Nothing when the program never reported the
    /// end of its turn: such a turn is left out of the usage log.
    pub tokens: Option<Tokens>,
    /// What the program said about its limits, during the turn or after it.
    pub reading: Reading,
    /// Whether the turn ended on a notice that a usage limit was hit.
    pub hit_limit: bool,
    /// The model the turn ran on, when known. Nothing means the program's own default.
    pub model: Option<String>,
}

/// A fact about a thread that a turn learns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadChange {
    /// The agent program named the conversation of one of the thread's agents, so later turns continue it.
    Conversation(Voice, String),
    /// The slash commands the program says it takes.
    Commands(Vec<String>),
    /// The reviewer finished a turn, having counted this many problems, or not said.
    Reviewed(Option<u32>),
    /// The reviewer's turn ended before it finished: stopped, failed, or cut short by the program.
    ReviewCut,
    /// The agent's program said a usage limit ended the turn, so another provider may carry on.
    RanOut(Agent),
    /// The program has ended, however it ended. The thread takes messages again
    /// and needs the person's attention.
    Finished,
}

/// A message for an agent, together with where to run it.
pub struct TurnRequest {
    pub thread: Thread,
    /// Whose turn it is: the thread's agent's, or its reviewer's.
    pub voice: Voice,
    pub directory: PathBuf,
    pub message: String,
    /// Environment variables for the program, on top of Teryon's own.
    pub environment: Vec<(String, OsString)>,
    /// What the agent is told about the thread's stage and plan, see [`crate::agents::staging`].
    pub staging: Option<String>,
}

/// An agent program that is running.
struct Running {
    stop: Sender<Option<Stop>>,
    /// Identifies this turn to Teryon when the agent calls `teryon` itself.
    turn: Uuid,
}

/// Keeps track of the agents that are running, so they can be stopped.
#[derive(Clone, Default)]
pub struct Runner {
    running: Arc<Mutex<HashMap<Uuid, Running>>>,
}

impl Runner {
    /// Starts an agent for one message and returns immediately.
    ///
    /// The caller has already marked the thread as running. The journal hears
    /// [`ThreadChange::Finished`] when the program has ended, whatever happened.
    pub fn start(&self, journal: Arc<dyn Journal>, settings: AgentSettings, request: TurnRequest) {
        let (stop, stopped) = channel(None);
        let thread_id = request.thread.id;
        let turn = Uuid::new_v4();
        self.running().insert(thread_id, Running { stop, turn });

        let runner = self.clone();
        tokio::spawn(async move {
            let late = run_turn(journal.as_ref(), &settings, request, turn, stopped).await;
            // Forget the run before the thread accepts messages again, so a
            // new run for the same thread is never forgotten in its place.
            runner.running().remove(&thread_id);
            journal.note(thread_id, ThreadChange::Finished);
            if let Some(late) = late {
                let reading = late.reading.await;
                journal.usage(
                    thread_id,
                    TurnUsage {
                        agent: late.agent,
                        tokens: None,
                        reading,
                        hit_limit: false,
                        model: None,
                    },
                );
            }
        });
    }

    /// Asks the agent of a thread to stop. Does nothing when none is running.
    pub fn stop(&self, thread_id: Uuid) {
        if let Some(running) = self.running().get(&thread_id) {
            let _ = running.stop.send(Some(Stop::ByUser));
        }
    }

    /// The turn a thread's agent is running now, if it is running.
    pub fn turn_of(&self, thread_id: Uuid) -> Option<Uuid> {
        self.running().get(&thread_id).map(|running| running.turn)
    }

    /// Stops every agent because Teryon is shutting down.
    pub fn stop_all(&self) {
        for running in self.running().values() {
            let _ = running.stop.send(Some(Stop::Shutdown));
        }
    }

    /// Whether any agent is still running.
    pub fn busy(&self) -> bool {
        !self.running().is_empty()
    }

    fn running(&self) -> MutexGuard<'_, HashMap<Uuid, Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An agent's limits, read once its thread has finished.
///
/// Grok's are asked of its servers, and a slow server must not keep a
/// finished thread busy, nor a stopped one from stopping.
struct LateReading {
    agent: Agent,
    reading: Pin<Box<dyn Future<Output = Reading> + Send>>,
}

/// Runs the program for one message and records everything it reports.
///
/// Returns the reading of the agent's limits that is left for after the thread has finished, if any.
async fn run_turn(
    journal: &dyn Journal,
    settings: &AgentSettings,
    request: TurnRequest,
    turn: Uuid,
    stopped: Receiver<Option<Stop>>,
) -> Option<LateReading> {
    let TurnRequest {
        thread,
        voice,
        directory,
        message,
        mut environment,
        staging,
    } = request;
    environment.push(("TERYON_THREAD".to_string(), thread.id.to_string().into()));
    environment.push(("TERYON_TURN".to_string(), turn.to_string().into()));

    let Some(speaker) = Speaker::of(&thread, voice) else {
        journal.add(
            thread.id,
            EntryDraft::error("This thread names an agent Teryon does not know.").by(voice),
        );
        review_cut(journal, &thread, voice);
        return None;
    };

    let mut driver = driver_for(speaker.agent, settings);
    let change = thread.change.as_deref();
    let brief = match voice {
        Voice::Author => instruction_for(&directory, change).await,
        Voice::Reviewer => change.map(reviewing),
    };
    let watch_change = brief.is_some();
    let brief = match voice {
        Voice::Author => with_naming(thread.named, Some(with_asking(joined(brief, staging)))),
        Voice::Reviewer => joined(brief, staging),
    };
    let before = match (voice, change) {
        (Voice::Reviewer, Some(change)) => fingerprint(change).await,
        _ => None,
    };
    let writable_outside = history_outside(&directory).await.into_iter().collect();
    let invocation = driver.invocation(&Turn {
        message,
        conversation: speaker.conversation.clone(),
        brief,
        read_only: voice == Voice::Reviewer,
        directory: directory.clone(),
        writable_outside,
        model: speaker.model.clone(),
    });
    let tree = match spawn(&invocation, &directory, &environment) {
        Ok(tree) => tree,
        Err(problem) => {
            let text = format!("Could not start `{}`: {problem}.", invocation.program);
            journal.add(
                thread.id,
                EntryDraft::error(&text).by(voice).naming(speaker.agent),
            );
            review_cut(journal, &thread, voice);
            return None;
        }
    };

    let grace = Duration::from_secs(settings.stop_grace_seconds);
    let outcome = follow(
        journal,
        &thread,
        &speaker,
        watch_change,
        driver,
        tree,
        stopped,
        grace,
    )
    .await;

    if let (Some(before), Some(change)) = (before, change) {
        if fingerprint(change)
            .await
            .is_some_and(|after| after != before)
        {
            let text = "The reviewer changed files, though it was told not to. Look at the changes before applying them.";
            journal.add(
                thread.id,
                EntryDraft::error(text).by(voice).naming(speaker.agent),
            );
        }
    }

    match outcome.stop {
        Some(Stop::ByUser) => journal.add(
            thread.id,
            EntryDraft::notice("Stopped by you.")
                .by(voice)
                .naming(speaker.agent),
        ),
        Some(Stop::Shutdown) => journal.add(
            thread.id,
            EntryDraft::notice("Interrupted by a restart.")
                .by(voice)
                .naming(speaker.agent),
        ),
        None if !outcome.summarised => {
            let text = unfinished_turn(outcome.status, &outcome.complaints);
            journal.add(
                thread.id,
                EntryDraft::error(&text).by(voice).naming(speaker.agent),
            );
        }
        None => {}
    }
    let hit_limit =
        outcome.hit_limit || (!outcome.summarised && says_limit_hit(&outcome.complaints));
    let mut late = None;
    let reading = match (speaker.agent, outcome.limits.clone()) {
        (Agent::Claude, Some(limits)) => Reading::Fresh(limits),
        (Agent::Claude, None) => Reading::Nothing,
        (Agent::Codex, _) => {
            let home = codex_home_here(settings.codex.home.as_deref());
            let conversation = outcome.conversation.clone();
            spawn_blocking(move || {
                codex_reading(home.as_deref(), conversation.as_deref(), Utc::now())
            })
            .await
            .unwrap_or(Reading::Nothing)
        }
        // Teryon is about to end, and nobody would see the reading.
        (Agent::Grok, _) if outcome.stop == Some(Stop::Shutdown) => Reading::Nothing,
        (Agent::Grok, _) => {
            let program = settings.grok.program.clone();
            let home = grok_home_here(settings.grok.home.as_deref());
            late = Some(LateReading {
                agent: Agent::Grok,
                reading: Box::pin(async move {
                    grok_reading(&program, home.as_deref(), Utc::now()).await
                }),
            });
            Reading::Nothing
        }
        (Agent::Gemini, _) if hit_limit => Reading::Fresh(gemini_used_up(Utc::now())),
        (Agent::Gemini, _) if outcome.summarised && !outcome.early => Reading::Cleared,
        (Agent::Gemini, _) => Reading::Nothing,
    };
    journal.usage(
        thread.id,
        TurnUsage {
            agent: speaker.agent,
            tokens: outcome.summarised.then(|| {
                outcome.tokens.unwrap_or(Tokens {
                    read: 0,
                    written: 0,
                })
            }),
            reading,
            hit_limit,
            model: outcome.model.clone(),
        },
    );

    if hit_limit {
        journal.note(thread.id, ThreadChange::RanOut(speaker.agent));
    }
    if voice == Voice::Reviewer {
        let finished = outcome.stop.is_none() && outcome.summarised && !outcome.early;
        let change = if finished {
            ThreadChange::Reviewed(findings_in(&outcome.last_message))
        } else {
            ThreadChange::ReviewCut
        };
        journal.note(thread.id, change);
    }
    late
}

/// Two parts of a brief, one after the other, either of which may be missing.
fn joined(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}\n\n{second}")),
        (first, second) => first.or(second),
    }
}

/// Records that a reviewer's turn ended before the reviewer could finish, here because it never started.
fn review_cut(journal: &dyn Journal, thread: &Thread, voice: Voice) {
    if voice == Voice::Reviewer {
        journal.note(thread.id, ThreadChange::ReviewCut);
    }
}

/// The agent taking a turn, the conversation it continues, and the model it runs.
struct Speaker {
    voice: Voice,
    agent: Agent,
    /// Nothing when the turn starts a conversation.
    conversation: Option<String>,
    /// Nothing leaves the model to the program.
    model: Option<String>,
}

impl Speaker {
    /// The thread's agent, or its reviewer. Nothing when the thread has no such agent, or names one Teryon does not know.
    fn of(thread: &Thread, voice: Voice) -> Option<Self> {
        let (agent, conversation, model) = match voice {
            Voice::Author => (&thread.agent, &thread.conversation, &thread.model),
            Voice::Reviewer => {
                let reviewer = thread.reviewer.as_ref()?;
                (&reviewer.agent, &reviewer.conversation, &reviewer.model)
            }
        };
        Some(Self {
            voice,
            agent: Agent::parse(agent)?,
            conversation: conversation.clone(),
            model: model.clone(),
        })
    }
}

/// What became of a program run.
struct Outcome {
    /// Whether the program reported the end of its turn.
    summarised: bool,
    /// Whether it said that turn ended before the agent finished.
    early: bool,
    stop: Option<Stop>,
    status: Option<ExitStatus>,
    /// The end of what the program wrote to its error output.
    complaints: String,
    /// The last message the agent wrote, where a reviewer counts its findings.
    last_message: String,
    /// The conversation the turn belonged to, as the program last named it.
    conversation: Option<String>,
    /// The tokens the program said the turn read and wrote.
    tokens: Option<Tokens>,
    /// The limits the program last reported during the turn.
    limits: Option<Limits>,
    /// Whether the program said a usage limit was hit.
    hit_limit: bool,
    /// The model the turn ran on, as its summary names it.
    model: Option<String>,
}

/// Reads the program's report until it ends, stopping it when asked to.
#[allow(clippy::too_many_arguments)]
async fn follow(
    journal: &dyn Journal,
    thread: &Thread,
    speaker: &Speaker,
    watch_change: bool,
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
    let mut early = false;
    let voice = speaker.voice;
    let mut conversation = speaker.conversation.clone();
    let mut last_message = String::new();
    let mut tokens = None;
    let mut limits = None;
    let mut hit_limit = false;
    let mut model = None;
    let mut known_commands = thread.commands.clone();
    let mut stop = None;
    let mut listening = true;
    let mut kill_at: Option<TokioInstant> = None;
    let mut ticks = interval(LOOK_AT_CHANGE_EVERY);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    while let Some(report) = lines.as_mut() {
        tokio::select! {
            _ = ticks.tick(), if watch_change => {
                journal.look(thread.id, Look::Quick).await;
            }
            line = report.next_line() => {
                let Ok(Some(line)) = line else { break };
                match driver.interpret(&line) {
                    Ok(drafts) => {
                        for draft in drafts {
                            summarised |= draft.kind == EntryKind::TurnSummary;
                            early |= draft.is_early_end();
                            if draft.kind == EntryKind::TurnSummary {
                                tokens = from_value::<Tokens>(draft.body["tokens"].clone()).ok();
                                model = driver.model();
                            }
                            hit_limit |=
                                matches!(draft.kind, EntryKind::Notice | EntryKind::Error)
                                    && draft.body["text"].as_str().is_some_and(says_limit_hit);
                            remember_message(&mut last_message, &draft);
                            let draft = with_model(timed(draft, started), driver.model());
                            journal.add(thread.id, draft.by(voice).naming(speaker.agent));
                        }
                    }
                    Err(_) => warn!("skipped a line of an agent's report that was not understood: {line}"),
                }
                // Asked only after a line, so the program has started, and has
                // stored a conversation the driver chose for it. A resumed
                // conversation may come back under a new name.
                if let Some(reported) = driver.take_limits() {
                    limits = Some(reported);
                }
                if let Some(named) = driver.take_conversation() {
                    if conversation.as_ref() != Some(&named) {
                        conversation = Some(named.clone());
                        journal.note(thread.id, ThreadChange::Conversation(voice, named));
                    }
                }
                // The programs name their commands every turn, some several
                // times. Only a list that differs is stored, so that a turn
                // does not send the same thread to every client again and again.
                // The reviewer's are left out: the message box offers the author's.
                if let Some(commands) = driver.take_commands() {
                    if voice == Voice::Author && commands != known_commands {
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

    if watch_change {
        journal.look(thread.id, Look::Thorough).await;
    }

    let status = match timeout(grace, tree.child().wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            tree.signal(Signal::Kill);
            tree.child().wait().await.ok()
        }
    };
    for draft in driver.finish() {
        remember_message(&mut last_message, &draft);
        journal.add(thread.id, draft.by(voice).naming(speaker.agent));
    }

    Outcome {
        summarised,
        early,
        stop,
        status,
        complaints: complaints.await.unwrap_or_default(),
        last_message,
        conversation,
        tokens,
        limits,
        hit_limit,
        model,
    }
}

/// Keeps the text of the agent's latest message.
fn remember_message(last: &mut String, draft: &EntryDraft) {
    if draft.kind == EntryKind::AgentMessage {
        if let Some(text) = draft.body["text"].as_str() {
            *last = text.to_string();
        }
    }
}

/// The repository's history, when it lies outside `directory`.
///
/// It lies inside for a project at the top of its repository, and outside
/// for a worktree of another repository. Nothing when `directory` is not a
/// repository, including a folder within one, which Teryon does not take
/// for a repository at all.
async fn history_outside(directory: &Path) -> Option<PathBuf> {
    let printed = Git::at(directory)
        .attempt(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .await
        .ok()??;
    let history = PathBuf::from(first_line(&printed));
    let inside = |folder: &Path| history.starts_with(folder);
    let directory_now = canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    if history.as_os_str().is_empty() || inside(directory) || inside(&directory_now) {
        None
    } else {
        Some(history)
    }
}

/// Starts the program in the project directory, at the head of a process tree.
fn spawn(
    invocation: &Invocation,
    directory: &Path,
    environment: &[(String, OsString)],
) -> Result<ProcessTree, IoError> {
    let mut command = Command::new(program_path(&invocation.program));
    command
        .args(&invocation.arguments)
        .envs(environment.iter().map(|(name, value)| (name, value)))
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
pub(crate) fn program_path(program: &str) -> PathBuf {
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

/// Adds the model the turn ran on to its summary, when it is known.
fn with_model(mut draft: EntryDraft, model: Option<String>) -> EntryDraft {
    if let (EntryKind::TurnSummary, Some(model)) = (draft.kind, model) {
        draft.body["model"] = json!(model);
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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    use std::fs::create_dir;
    use std::process::Command as StdCommand;

    use lets_expect::lets_expect;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::*;

    /// Where a project's history lies, for a project made by `make` in a fresh folder.
    ///
    /// Paths are given relative to the fresh folder, so the answer does not
    /// depend on where temporary folders are.
    async fn history_of(make: fn(&Path) -> PathBuf) -> Option<String> {
        let folder = tempdir().unwrap();
        let top = canonicalize(folder.path()).unwrap();
        let project = make(&top);
        history_outside(&project).await.map(|history| {
            history
                .strip_prefix(&top)
                .unwrap_or(&history)
                .display()
                .to_string()
        })
    }

    fn run_git(directory: &Path, arguments: &[&str]) {
        let status = StdCommand::new("git")
            .args(arguments)
            .current_dir(directory)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?} failed");
    }

    /// A repository with one commit, at `top`.
    fn repository(top: &Path) -> PathBuf {
        run_git(top, &["init", "-q", "-b", "main"]);
        run_git(
            top,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "start",
            ],
        );
        top.to_path_buf()
    }

    fn folder_inside(top: &Path) -> PathBuf {
        repository(top);
        let inside = top.join("app");
        create_dir(&inside).unwrap();
        inside
    }

    fn worktree_beside(top: &Path) -> PathBuf {
        let main = top.join("main");
        create_dir(&main).unwrap();
        repository(&main);
        run_git(&main, &["worktree", "add", "-q", "../other", "-b", "other"]);
        top.join("other")
    }

    fn no_repository(top: &Path) -> PathBuf {
        top.to_path_buf()
    }

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

    lets_expect! { #tokio_test
        // requirement: an agent confined to its project may still write the repository's history when that lies outside the project
        expect(history_of(make).await) {
            let make: fn(&Path) -> PathBuf = repository;

            when(make = repository) as a_project_at_the_top_of_its_repository to need_nothing_more { equal(None) }
            when(make = folder_inside) as a_folder_inside_a_repository to be_taken_for_no_repository { equal(None) }
            when(make = worktree_beside) as a_worktree_of_another_repository to name_that_history { equal(Some("main/.git".to_string())) }
            when(make = no_repository) as a_folder_outside_any_repository to need_nothing { equal(None) }
        }
    }

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

        // requirement: a turn's summary names the model it ran on, when that is known
        expect(with_model(draft, model).body["model"].clone()) {
            let draft = EntryDraft::turn_summary(None, None);
            let model = Some("claude-opus-5-5".to_string());

            to name_the_model { equal(json!("claude-opus-5-5")) }
            when(model = None) as the_model_is_unknown to leave_it_out { equal(Value::Null) }
            when(draft = EntryDraft::notice("Hello")) as another_entry to be_left_alone { equal(Value::Null) }
        }
    }
}
