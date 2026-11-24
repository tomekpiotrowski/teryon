//! Cross-project thread inspection and management commands.
//!
//! Each command connects to the running service (carrying `TERYON_THREAD` in
//! `Hello` when run inside an agent's turn), resolves projects and thread
//! identifier beginnings from the initial snapshot, and prints either short
//! plain text or full JSON with `--json`.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::io::stdin;
use std::process::ExitCode;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, to_string_pretty};
use tokio::time::Instant;
use uuid::Uuid;

use teryon_core::agents::models::Effort;
use teryon_core::agents::Voice;
use teryon_core::protocol::{Answer, Event, ProjectView, Request};
use teryon_core::store::{Entry, Status, Thread};

use crate::client::Client;
use crate::command::naming::{resolve_project, resolve_thread, short_id};
use crate::command::output::{
    format_projects, format_thread_show, format_threads, format_wait_result, format_wait_timeout,
};
use crate::command::{connect, parse_timeout, read_message, Context, Snapshot};

/// Exit code returned by `wait` when the timeout passes while threads are still working.
pub const TIMEOUT_EXIT_CODE: u8 = 2;

/// Lists every project known to the service.
pub async fn list_projects(context: Context) -> ExitCode {
    let (_, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    if context.json {
        print_json(&snapshot.projects)
    } else {
        println!("{}", format_projects(&snapshot.projects));
        ExitCode::SUCCESS
    }
}

/// Lists threads across every project, or inside one named project.
pub async fn list_threads(context: Context, project: Option<String>, all: bool) -> ExitCode {
    let (_, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let project_id = match project.as_deref() {
        Some(query) => match resolve_project(&snapshot.projects, query) {
            Ok(found) => Some(found.id),
            Err(problem) => return fail(&problem.to_string()),
        },
        None => None,
    };
    let mut threads: Vec<Thread> = snapshot
        .threads
        .into_iter()
        .filter(|thread| project_id.is_none_or(|id| thread.project_id == id))
        .filter(|thread| all || !thread.put_away)
        .collect();
    threads.sort_by_key(|thread| Reverse(thread.created_at));
    if context.json {
        print_json(&threads)
    } else {
        println!("{}", format_threads(&snapshot.projects, &threads));
        ExitCode::SUCCESS
    }
}

/// Puts away every quiet thread in one project, or in every project.
pub async fn put_away(context: Context, project: Option<String>) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let project_id = match project.as_deref() {
        Some(query) => match resolve_project(&snapshot.projects, query) {
            Ok(found) => Some(found.id),
            Err(problem) => return fail(&problem.to_string()),
        },
        None => None,
    };
    match client.call(Request::PutAwayQuiet { project_id }).await {
        Ok(Answer::PutAway { count }) => {
            if context.json {
                print_json(&json!({ "count": count }))
            } else {
                match count {
                    0 => println!("No quiet threads to put away."),
                    1 => println!("Put away 1 thread."),
                    count => println!("Put away {count} threads."),
                }
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Shows one thread's state, change, plan and last messages.
pub async fn show_thread(
    context: Context,
    target: String,
    last: usize,
    activity: bool,
) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    let entries = match fetch_entries(&mut client, thread.id).await {
        Ok(entries) => entries,
        Err(problem) => return fail(&problem),
    };
    let project = snapshot
        .projects
        .iter()
        .find(|project| project.id == thread.project_id);
    if context.json {
        print_json(&json!({
            "thread": thread,
            "project": project,
            "entries": entries,
        }))
    } else {
        println!(
            "{}",
            format_thread_show(project, &thread, &entries, last, activity)
        );
        ExitCode::SUCCESS
    }
}

/// Prints a thread's plan as markdown.
pub async fn show_plan(context: Context, target: String) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    match client.call(Request::Plan { thread_id: thread.id }).await {
        Ok(Answer::Plan { text }) => {
            if context.json {
                print_json(&json!({
                    "thread_id": thread.id,
                    "plan": thread.plan,
                    "text": text,
                }))
            } else {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Options for starting a new thread and sending its first message.
pub struct StartOptions {
    pub project: String,
    pub workflow: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title: Option<String>,
    pub wait: bool,
    pub timeout: String,
    pub message: String,
}

/// Starts a new thread in a project and sends its first message, optionally waiting for a reply.
pub async fn start_thread(context: Context, options: StartOptions) -> ExitCode {
    let effort = match parse_optional_effort(options.effort.as_deref()) {
        Ok(effort) => effort,
        Err(problem) => return fail(&problem),
    };
    let wait_duration = if options.wait {
        match parse_timeout(&options.timeout) {
            Ok(duration) => Some(duration),
            Err(problem) => return fail(&problem),
        }
    } else {
        None
    };
    let message = match read_message(&options.message, stdin()) {
        Ok(message) => message,
        Err(problem) => return fail(&problem),
    };
    let (mut client, mut snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let project = match resolve_project(&snapshot.projects, &options.project) {
        Ok(project) => project.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    let started = match client
        .call(Request::StartThread {
            project_id: project.id,
            agent: options.agent,
            model: options.model,
            effort,
            title: options.title.unwrap_or_default(),
            workflow: options.workflow,
        })
        .await
    {
        Ok(Answer::Thread { thread }) => *thread,
        Ok(_) => return fail("Teryon answered unexpectedly."),
        Err(problem) => return fail(&problem.to_string()),
    };
    if let Err(problem) = client
        .call(Request::SendMessage {
            thread_id: started.id,
            text: message,
            to: Voice::Author,
        })
        .await
    {
        return fail(&problem.to_string());
    }
    let mut running = started;
    running.status = Status::Running;
    snapshot.threads.push(running.clone());
    if let Some(duration) = wait_duration {
        return wait_on(
            &mut client,
            snapshot,
            vec![running.id],
            duration,
            context.json,
            true,
        )
        .await;
    }
    if context.json {
        print_json(&running)
    } else {
        println!(
            "Started {} in {} ({})",
            running.id,
            project.name,
            running.stage.name()
        );
        ExitCode::SUCCESS
    }
}

/// Sends a message to an existing thread, optionally waiting for its agent to settle.
pub async fn send_message(
    context: Context,
    target: String,
    reviewer: bool,
    wait: bool,
    timeout: String,
    message: String,
) -> ExitCode {
    let wait_duration = if wait {
        match parse_timeout(&timeout) {
            Ok(duration) => Some(duration),
            Err(problem) => return fail(&problem),
        }
    } else {
        None
    };
    let text = match read_message(&message, stdin()) {
        Ok(text) => text,
        Err(problem) => return fail(&problem),
    };
    let (mut client, mut snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    let to = if reviewer {
        Voice::Reviewer
    } else {
        Voice::Author
    };
    let entry = match client
        .call(Request::SendMessage {
            thread_id: thread.id,
            text,
            to,
        })
        .await
    {
        Ok(Answer::Entry { entry }) => entry,
        Ok(_) => return fail("Teryon answered unexpectedly."),
        Err(problem) => return fail(&problem.to_string()),
    };
    if let Some(duration) = wait_duration {
        if let Some(tracked) = snapshot
            .threads
            .iter_mut()
            .find(|candidate| candidate.id == thread.id)
        {
            tracked.status = Status::Running;
        }
        return wait_on(
            &mut client,
            snapshot,
            vec![thread.id],
            duration,
            context.json,
            true,
        )
        .await;
    }
    if context.json {
        print_json(&entry)
    } else {
        println!("Sent to {} ({}).", short_id(thread.id), display_title(&thread));
        ExitCode::SUCCESS
    }
}

/// Waits until every named thread is idle or waiting for answers, then prints each thread.
pub async fn wait_threads(context: Context, targets: Vec<String>, timeout: String) -> ExitCode {
    let duration = match parse_timeout(&timeout) {
        Ok(duration) => duration,
        Err(problem) => return fail(&problem),
    };
    let (mut client, snapshot, acting) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let mut ids = Vec::with_capacity(targets.len());
    for target in &targets {
        let thread = match resolve_thread(&snapshot.threads, target) {
            Ok(thread) => thread,
            Err(problem) => return fail(&problem.to_string()),
        };
        if acting == Some(thread.id) {
            return fail("An agent cannot wait for its own thread.");
        }
        if !ids.contains(&thread.id) {
            ids.push(thread.id);
        }
    }
    wait_on(&mut client, snapshot, ids, duration, context.json, false).await
}

/// Stops a running thread's agent.
pub async fn stop_thread(context: Context, target: String) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    match client.call(Request::Stop { thread_id: thread.id }).await {
        Ok(Answer::Stopped { stopped: true }) => {
            if context.json {
                print_json(&json!({ "stopped": true, "thread_id": thread.id }))
            } else {
                println!(
                    "Stopped {} ({}).",
                    short_id(thread.id),
                    display_title(&thread)
                );
                ExitCode::SUCCESS
            }
        }
        Ok(Answer::Stopped { stopped: false }) => fail(&format!(
            "{} ({}) is not running.",
            short_id(thread.id),
            display_title(&thread)
        )),
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Switches a thread's agent or reviewer to another agent, model or effort.
pub async fn change_agent(
    context: Context,
    target: String,
    agent: String,
    model: Option<String>,
    effort: Option<String>,
    reviewer: bool,
) -> ExitCode {
    let effort = match parse_optional_effort(effort.as_deref()) {
        Ok(effort) => effort,
        Err(problem) => return fail(&problem),
    };
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    let to = if reviewer {
        Voice::Reviewer
    } else {
        Voice::Author
    };
    match client
        .call(Request::ChangeAgent {
            thread_id: thread.id,
            to,
            agent,
            model,
            effort,
        })
        .await
    {
        Ok(Answer::Thread { thread }) => {
            if context.json {
                print_json(&thread)
            } else {
                println!(
                    "Updated {} ({}).",
                    short_id(thread.id),
                    display_title(&thread)
                );
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Asks another agent to review a thread's change.
pub async fn review_thread(
    context: Context,
    target: String,
    agent: String,
    model: Option<String>,
    effort: Option<String>,
) -> ExitCode {
    let effort = match parse_optional_effort(effort.as_deref()) {
        Ok(effort) => effort,
        Err(problem) => return fail(&problem),
    };
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    match client
        .call(Request::ReviewChange {
            thread_id: thread.id,
            agent,
            model,
            effort,
        })
        .await
    {
        Ok(Answer::Entry { entry }) => {
            if context.json {
                print_json(&entry)
            } else {
                println!(
                    "Started review on {} ({}).",
                    short_id(thread.id),
                    display_title(&thread)
                );
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Puts another thread on a named workflow.
pub async fn choose_thread_workflow(
    context: Context,
    target: String,
    workflow: String,
) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    match client
        .call(Request::ChooseWorkflow {
            thread_id: thread.id,
            turn: None,
            workflow,
        })
        .await
    {
        Ok(Answer::Thread { thread }) => {
            if context.json {
                print_json(&thread)
            } else {
                println!(
                    "{} now follows `{}` (stage: {}).",
                    short_id(thread.id),
                    thread.workflow,
                    thread.stage.name()
                );
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Waits until every thread in `ids` becomes idle, or exits with code 2 on timeout.
///
/// When `expect_start` is true (after `start --wait` or `send --wait`), stale
/// `Idle` events emitted before the new turn began are ignored until the turn's
/// `Running` event (or a lagged `Snapshot`) has been observed.
async fn wait_on(
    client: &mut Client,
    snapshot: Snapshot,
    ids: Vec<Uuid>,
    duration: Duration,
    as_json: bool,
    expect_start: bool,
) -> ExitCode {
    let projects = snapshot.projects;
    let mut tracked: HashMap<Uuid, Thread> = snapshot
        .threads
        .into_iter()
        .filter(|thread| ids.contains(&thread.id))
        .map(|thread| (thread.id, thread))
        .collect();
    let mut started: HashMap<Uuid, bool> = ids.iter().map(|id| (*id, !expect_start)).collect();
    let deadline = Instant::now() + duration;

    loop {
        let all_idle = ids.iter().all(|id| {
            started.get(id).copied().unwrap_or(false)
                && tracked
                    .get(id)
                    .is_some_and(|thread| thread.status == Status::Idle)
        });
        if all_idle {
            break;
        }
        match client.event_until(deadline).await {
            Ok(Some(Event::ThreadChanged { thread })) => {
                if ids.contains(&thread.id) {
                    if thread.status == Status::Running {
                        started.insert(thread.id, true);
                    }
                    if started.get(&thread.id).copied().unwrap_or(false) {
                        tracked.insert(thread.id, thread);
                    }
                }
            }
            Ok(Some(Event::Snapshot { threads, .. })) => {
                for thread in threads {
                    if ids.contains(&thread.id) {
                        started.insert(thread.id, true);
                        tracked.insert(thread.id, thread);
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                let working: Vec<&Thread> = ids
                    .iter()
                    .filter_map(|id| tracked.get(id))
                    .filter(|thread| thread.status == Status::Running)
                    .collect();
                if as_json {
                    let _ = print_json(&json!({
                        "timed_out": true,
                        "working": working,
                    }));
                } else {
                    println!("{}", format_wait_timeout(&projects, &working));
                }
                return ExitCode::from(TIMEOUT_EXIT_CODE);
            }
            Err(problem) => return fail(&problem.to_string()),
        }
    }

    let mut collected: Vec<(Option<&ProjectView>, Thread, Vec<Entry>)> = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some(thread) = tracked.get(id).cloned() else {
            continue;
        };
        let entries = match fetch_entries(client, thread.id).await {
            Ok(entries) => entries,
            Err(problem) => return fail(&problem),
        };
        let project = projects
            .iter()
            .find(|project| project.id == thread.project_id);
        collected.push((project, thread, entries));
    }

    if as_json {
        let payload: Vec<_> = collected
            .iter()
            .map(|(project, thread, entries)| {
                json!({
                    "thread": thread,
                    "project": project,
                    "entries": entries,
                })
            })
            .collect();
        if payload.len() == 1 {
            print_json(&payload[0])
        } else {
            print_json(&payload)
        }
    } else {
        let refs: Vec<(Option<&ProjectView>, &Thread, &[Entry])> = collected
            .iter()
            .map(|(project, thread, entries)| (*project, thread, entries.as_slice()))
            .collect();
        println!("{}", format_wait_result(&refs));
        ExitCode::SUCCESS
    }
}

async fn fetch_entries(client: &mut Client, thread_id: Uuid) -> Result<Vec<Entry>, String> {
    match client.call(Request::FollowThread { thread_id }).await {
        Ok(Answer::Entries { entries }) => Ok(entries),
        Ok(_) => Err("Teryon answered unexpectedly.".to_string()),
        Err(problem) => Err(problem.to_string()),
    }
}

fn parse_optional_effort(text: Option<&str>) -> Result<Option<Effort>, String> {
    match text {
        None => Ok(None),
        Some(raw) => Effort::parse(raw.trim()).map(Some).ok_or_else(|| {
            "Choose low, medium, high, xhigh, max or ultra for --effort.".to_string()
        }),
    }
}

fn display_title(thread: &Thread) -> &str {
    if thread.title.is_empty() {
        "untitled"
    } else {
        &thread.title
    }
}

pub(crate) fn print_json<T: Serialize>(value: &T) -> ExitCode {
    match to_string_pretty(value) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(problem) => fail(&problem.to_string()),
    }
}

pub(crate) fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}
