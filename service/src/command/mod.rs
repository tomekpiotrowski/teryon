//! Command-line parsing, dispatch, and shared helpers for `teryon`.
//!
//! Keeps command parsing and help text in one place so a person or an agent
//! reading `--help` gets everything they need without looking elsewhere.

pub mod changes;
pub mod naming;
pub mod output;
pub mod own;
pub mod threads;

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use uuid::Uuid;

use teryon_core::paths::Folders;
use teryon_core::protocol::{Event, ProjectView};
use teryon_core::store::Thread;

use crate::client::Client;
use crate::command::own::{data_folder, identifier};
use crate::command::threads::StartOptions;

/// Default wait timeout: 9 minutes, below the 10-minute command tool limit.
pub const DEFAULT_TIMEOUT: &str = "9m";

/// Teryon: agents working on your projects.
#[derive(Parser)]
#[command(name = "teryon", version)]
pub struct Arguments {
    /// Keep everything in this folder instead of the usual places.
    #[arg(long, global = true, env = "TERYON_HOME")]
    pub home: Option<PathBuf>,
    /// Print full records as JSON instead of short plain text.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the service that owns this machine's projects.
    Service {
        /// Print the address of the app at this address, with what it needs to connect.
        #[arg(long, value_name = "APP ADDRESS")]
        print_address: Option<String>,
    },
    /// Stop the running service, after stopping every agent.
    Stop,
    /// List every project: name, identifier, folder and state.
    Projects,
    /// List threads across projects, or put away quiet threads.
    Threads {
        /// Show only threads in this project (name or identifier).
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
        /// Include threads that were put away.
        #[arg(long)]
        all: bool,
        #[command(subcommand)]
        command: Option<ThreadsCommand>,
    },
    /// Inspect, start, message, wait for, stop or configure a thread.
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// Inspect, apply, catch up, discard or send a thread's change for review.
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    /// For agents working in Teryon: their own copy of the project to change files in.
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
}

#[derive(Subcommand)]
pub enum ThreadsCommand {
    /// Put away quiet threads (still talking, idle, no questions, no change or plan).
    PutAway {
        /// Put away quiet threads only in this project (name or identifier).
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ThreadCommand {
    /// Show a thread's state, change, plan and latest messages.
    Show {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
        /// Number of recent messages to show.
        #[arg(long, default_value_t = 10, value_name = "N")]
        last: usize,
        /// Also list tool calls.
        #[arg(long)]
        activity: bool,
    },
    /// Print a thread's plan as markdown.
    Plan {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
    },
    /// Start a new thread in a project and send its first message.
    Start {
        /// Project name or identifier.
        #[arg(long, value_name = "PROJECT")]
        project: String,
        /// Workflow name, such as `default` or `small`.
        #[arg(long, value_name = "WORKFLOW")]
        workflow: Option<String>,
        /// Agent program: `claude`, `codex`, `grok` or `gemini`.
        #[arg(long, value_name = "AGENT")]
        agent: Option<String>,
        /// Model name for the agent.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        /// Effort level: `low`, `medium`, `high`, `xhigh`, `max` or `ultra`.
        #[arg(long, value_name = "EFFORT")]
        effort: Option<String>,
        /// Short title for the thread.
        #[arg(long, value_name = "TEXT")]
        title: Option<String>,
        /// Wait until the thread settles, then print its latest messages.
        #[arg(long)]
        wait: bool,
        /// How long `--wait` waits before exiting with code 2 (e.g. `9m`, `30s`).
        #[arg(long, default_value = DEFAULT_TIMEOUT, value_name = "DURATION")]
        timeout: String,
        /// The first message, or `-` to read from standard input.
        message: String,
    },
    /// Send a message to a thread's agent or reviewer.
    Send {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
        /// Send to the thread's reviewer instead of its author.
        #[arg(long)]
        reviewer: bool,
        /// Wait until the thread settles, then print its latest messages.
        #[arg(long)]
        wait: bool,
        /// How long `--wait` waits before exiting with code 2 (e.g. `9m`, `30s`).
        #[arg(long, default_value = DEFAULT_TIMEOUT, value_name = "DURATION")]
        timeout: String,
        /// The message text, or `-` to read from standard input.
        message: String,
    },
    /// Wait until every named thread is idle or waiting for answers (exit code 2 on timeout).
    Wait {
        /// One or more thread identifiers or unique beginnings of at least 4 characters.
        #[arg(required = true)]
        threads: Vec<String>,
        /// How long to wait before exiting with code 2 (e.g. `9m`, `30s`).
        #[arg(long, default_value = DEFAULT_TIMEOUT, value_name = "DURATION")]
        timeout: String,
    },
    /// Stop a running thread's agent.
    Stop {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
    },
    /// Switch a thread's agent or reviewer to another agent, model or effort.
    Agent {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
        /// Agent program: `claude`, `codex`, `grok` or `gemini`.
        #[arg(long, value_name = "AGENT")]
        agent: String,
        /// Model name for the agent.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        /// Effort level: `low`, `medium`, `high`, `xhigh`, `max` or `ultra`.
        #[arg(long, value_name = "EFFORT")]
        effort: Option<String>,
        /// Switch the thread's reviewer instead of its author.
        #[arg(long)]
        reviewer: bool,
    },
    /// Ask a second agent to review a thread's change.
    Review {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
        /// Reviewing agent program: `claude`, `codex`, `grok` or `gemini`.
        #[arg(long, value_name = "AGENT")]
        agent: String,
        /// Model name for the reviewer.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        /// Effort level: `low`, `medium`, `high`, `xhigh`, `max` or `ultra`.
        #[arg(long, value_name = "EFFORT")]
        effort: Option<String>,
    },
    /// Put a thread on another workflow; without `--thread`, the agent's own thread.
    Workflow {
        /// The workflow's name, such as `small`.
        name: String,
        /// Another thread's identifier or unique beginning of at least 4 characters.
        #[arg(long, value_name = "THREAD")]
        thread: Option<String>,
    },
    /// Give this thread a title of a few words, shown in place of the start of its first message.
    Title {
        /// The title, such as `Fix the login redirect`.
        title: String,
    },
    /// Ask the person questions, written as JSON on standard input; their answers arrive as their next message.
    Ask,
    /// Say the thread's work has moved to another stage; the next stage's agent may take over once your turn ends.
    Stage {
        /// One of `talking`, `planning`, `implementing`, `review`, `ready`.
        stage: String,
    },
}

#[derive(Subcommand)]
pub enum ChangeCommand {
    /// Show a thread's change: state, branch, copy's folder and how it fits the project.
    Show {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
    },
    /// Apply a thread's change to the project; without `--description`, Teryon drafts one.
    Apply {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
        /// Description for the project's history, or `-` to read from standard input.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
    },
    /// Bring a thread's change up to date with newer work in the project, unless they overlap.
    CatchUp {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
    },
    /// Throw a thread's change away, stopping its agent first if working.
    Discard {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
    },
    /// Send a thread's change for review on GitHub; without `--description`, Teryon drafts one.
    SendForReview {
        /// Thread identifier or unique beginning of at least 4 characters.
        thread: String,
        /// Description for the review, or `-` to read from standard input.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum WorktreeCommand {
    /// Create a git worktree on a new branch for this thread, and print its path.
    Create {
        /// The name of the worktree and of its branch, such as `fix-login-redirect`.
        name: String,
        /// The branch to start from. Without it, the branch the project is on.
        #[arg(long, value_name = "BRANCH")]
        from: Option<String>,
    },
}

/// Shared execution context for CLI commands.
pub struct Context {
    pub home: Option<PathBuf>,
    pub folders: Folders,
    pub json: bool,
}

/// Initial projects and threads sent by the service right after `Hello`.
pub struct Snapshot {
    pub projects: Vec<ProjectView>,
    pub threads: Vec<Thread>,
}

/// Connects to the local service, passing `TERYON_THREAD` when set, and reads its snapshot.
pub async fn connect(context: &Context) -> Result<(Client, Snapshot, Option<Uuid>), String> {
    let data = data_folder(context.home.as_deref(), context.folders.clone());
    let acting = identifier("TERYON_THREAD");
    let mut client = Client::connect_for(&data, acting)
        .await
        .map_err(|problem| problem.to_string())?;
    let snapshot = match client.event().await.map_err(|problem| problem.to_string())? {
        Event::Snapshot {
            projects, threads, ..
        } => Snapshot { projects, threads },
        _ => return Err("The Teryon service did not send a snapshot.".to_string()),
    };
    Ok((client, snapshot, acting))
}

/// Dispatches non-service commands.
pub async fn run(command: Command, context: Context) -> ExitCode {
    match command {
        Command::Projects => threads::list_projects(context).await,
        Command::Threads {
            project,
            all,
            command: None,
        } => threads::list_threads(context, project, all).await,
        Command::Threads {
            project: outer,
            command: Some(ThreadsCommand::PutAway { project }),
            ..
        } => threads::put_away(context, project.or(outer)).await,
        Command::Thread { command } => run_thread(command, context).await,
        Command::Change { command } => run_change(command, context).await,
        Command::Worktree {
            command: WorktreeCommand::Create { name, from },
        } => own::create_worktree(context.home.as_deref(), context.folders, name, from).await,
        Command::Service { .. } | Command::Stop => unreachable!("handled by main"),
    }
}

async fn run_thread(command: ThreadCommand, context: Context) -> ExitCode {
    match command {
        ThreadCommand::Show {
            thread,
            last,
            activity,
        } => threads::show_thread(context, thread, last, activity).await,
        ThreadCommand::Plan { thread } => threads::show_plan(context, thread).await,
        ThreadCommand::Start {
            project,
            workflow,
            agent,
            model,
            effort,
            title,
            wait,
            timeout,
            message,
        } => {
            threads::start_thread(
                context,
                StartOptions {
                    project,
                    workflow,
                    agent,
                    model,
                    effort,
                    title,
                    wait,
                    timeout,
                    message,
                },
            )
            .await
        }
        ThreadCommand::Send {
            thread,
            reviewer,
            wait,
            timeout,
            message,
        } => threads::send_message(context, thread, reviewer, wait, timeout, message).await,
        ThreadCommand::Wait { threads, timeout } => {
            threads::wait_threads(context, threads, timeout).await
        }
        ThreadCommand::Stop { thread } => threads::stop_thread(context, thread).await,
        ThreadCommand::Agent {
            thread,
            agent,
            model,
            effort,
            reviewer,
        } => threads::change_agent(context, thread, agent, model, effort, reviewer).await,
        ThreadCommand::Review {
            thread,
            agent,
            model,
            effort,
        } => threads::review_thread(context, thread, agent, model, effort).await,
        ThreadCommand::Workflow { name, thread: None } => {
            own::choose_own_workflow(context.home.as_deref(), context.folders, name).await
        }
        ThreadCommand::Workflow {
            name,
            thread: Some(target),
        } => threads::choose_thread_workflow(context, target, name).await,
        ThreadCommand::Title { title } => {
            own::title_thread(context.home.as_deref(), context.folders, title).await
        }
        ThreadCommand::Ask => own::ask_questions(context.home.as_deref(), context.folders).await,
        ThreadCommand::Stage { stage } => {
            own::report_stage(context.home.as_deref(), context.folders, stage).await
        }
    }
}

async fn run_change(command: ChangeCommand, context: Context) -> ExitCode {
    match command {
        ChangeCommand::Show { thread } => changes::show_change(context, thread).await,
        ChangeCommand::Apply {
            thread,
            description,
        } => changes::apply_change(context, thread, description).await,
        ChangeCommand::CatchUp { thread } => changes::catch_up(context, thread).await,
        ChangeCommand::Discard { thread } => changes::discard_change(context, thread).await,
        ChangeCommand::SendForReview {
            thread,
            description,
        } => changes::send_for_review(context, thread, description).await,
    }
}

/// Reads a message from `argument`, or from `input` when `argument` is `-`.
pub fn read_message(argument: &str, mut input: impl Read) -> Result<String, String> {
    let text = if argument == "-" {
        let mut buffer = String::new();
        input
            .read_to_string(&mut buffer)
            .map_err(|problem| format!("Could not read the message from standard input: {problem}"))?;
        buffer
    } else {
        argument.to_string()
    };
    if text.trim().is_empty() {
        return Err("Write a message first.".to_string());
    }
    Ok(text)
}

/// Parses a human-friendly timeout such as `9m`, `30s`, `1m30s`, `500ms`, or plain seconds `60`.
pub fn parse_timeout(text: &str) -> Result<Duration, String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err("Give a timeout such as `9m` or `30s`.".to_string());
    }
    if let Ok(seconds) = raw.parse::<u64>() {
        return Ok(Duration::from_secs(seconds));
    }
    let bytes = raw.as_bytes();
    let mut index = 0;
    let mut total = Duration::ZERO;
    while index < bytes.len() {
        let digit_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == digit_start {
            return Err(format!(
                "Could not read timeout `{raw}`. Use forms such as `9m`, `30s`, `1m30s` or `500ms`."
            ));
        }
        let amount: u64 = raw[digit_start..index]
            .parse()
            .map_err(|_| format!("Timeout `{raw}` is too large."))?;
        let unit_start = index;
        while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
            index += 1;
        }
        let unit = &raw[unit_start..index];
        let part = match unit {
            "ms" => Duration::from_millis(amount),
            "s" => Duration::from_secs(amount),
            "m" => amount
                .checked_mul(60)
                .map(Duration::from_secs)
                .ok_or_else(|| format!("Timeout `{raw}` is too large."))?,
            "h" => amount
                .checked_mul(3600)
                .map(Duration::from_secs)
                .ok_or_else(|| format!("Timeout `{raw}` is too large."))?,
            _ => {
                return Err(format!(
                    "Unknown timeout unit `{unit}` in `{raw}`. Use `ms`, `s`, `m` or `h`."
                ))
            }
        };
        total = total
            .checked_add(part)
            .ok_or_else(|| format!("Timeout `{raw}` is too large."))?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        expect(parse_timeout(text)) as parsing_a_timeout {
            when(text = "9m") as nine_minutes to be_540_seconds { equal(Ok(Duration::from_secs(540))) }
            when(text = "30s") as thirty_seconds to be_30_seconds { equal(Ok(Duration::from_secs(30))) }
            when(text = "1h") as one_hour to be_3600_seconds { equal(Ok(Duration::from_secs(3600))) }
            when(text = "1m30s") as compound to add_parts { equal(Ok(Duration::from_secs(90))) }
            when(text = "500ms") as milliseconds to be_half_a_second { equal(Ok(Duration::from_millis(500))) }
            when(text = "60") as plain_seconds to be_60_seconds { equal(Ok(Duration::from_secs(60))) }
            when(text = "0s") as zero to be_allowed { equal(Ok(Duration::ZERO)) }
            when(text = "") as empty to be_refused { be_err }
            when(text = "abc") as non_numeric to be_refused { be_err }
            when(text = "5d") as unknown_unit to be_refused { be_err }
            when(text = "-5s") as negative to be_refused { be_err }
            when(text = "10m5") as trailing_number_without_unit to be_refused { be_err }
        }

        expect(read_message(argument, Cursor::new(stdin_text))) as reading_a_message {
            let stdin_text = "Hello from stdin\n";

            when(argument = "Direct message") as literal_text to return_it { equal(Ok("Direct message".to_string())) }
            when(argument = "-") as dash to read_stdin { equal(Ok("Hello from stdin\n".to_string())) }
            when(argument = "   ") as blank_literal to be_refused { be_err }
            when(argument = "-", stdin_text = "  \n ") as blank_stdin to be_refused { be_err }
        }
    }
}
