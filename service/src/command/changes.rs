//! Commands for inspecting and finishing a thread's change.
//!
//! Each command resolves the thread from its identifier beginning, drafts a
//! description when none was supplied to `apply` or `send-for-review`, and
//! prints either plain text or JSON with `--json`.

use std::io::stdin;
use std::process::ExitCode;

use serde_json::json;
use uuid::Uuid;

use teryon_core::protocol::{Answer, Request};

use crate::client::Client;
use crate::command::naming::{resolve_thread, short_id};
use crate::command::output::format_change_show;
use crate::command::threads::{fail, print_json};
use crate::command::{connect, read_message, Context};

/// Shows where a thread's change stands: branch, folder, files changed and fit.
pub async fn show_change(context: Context, target: String) -> ExitCode {
    let (_, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread,
        Err(problem) => return fail(&problem.to_string()),
    };
    if context.json {
        print_json(&json!({
            "thread_id": thread.id,
            "change": thread.change,
            "finished": thread.finished,
        }))
    } else {
        println!("{}", format_change_show(thread));
        ExitCode::SUCCESS
    }
}

/// Applies a thread's change to the project, using the agent's commit message when none is given.
///
/// When the change overlaps with newer work, Teryon starts the thread's agent to catch up
/// and applies it automatically once the agent finishes.
pub async fn apply_change(
    context: Context,
    target: String,
    description: Option<String>,
) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    let description = match description {
        Some(raw) => match read_message(&raw, stdin()) {
            Ok(text) => Some(text),
            Err(problem) => return fail(&problem),
        },
        None => None,
    };
    match client
        .call(Request::ApplyChange {
            thread_id: thread.id,
            description,
        })
        .await
    {
        Ok(Answer::Applied { resolving }) => {
            if context.json {
                print_json(&json!({
                    "applied": !resolving,
                    "resolving": resolving,
                    "thread_id": thread.id,
                }))
            } else if resolving {
                println!(
                    "The change of {} overlaps with newer work. The agent is catching up and will apply it when done.",
                    short_id(thread.id)
                );
                ExitCode::SUCCESS
            } else {
                println!("Applied change of {}.", short_id(thread.id));
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Brings a thread's change up to date with newer work in the project, unless they overlap.
pub async fn catch_up(context: Context, target: String) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    match client
        .call(Request::CatchUp {
            thread_id: thread.id,
        })
        .await
    {
        Ok(Answer::CaughtUp { overlaps }) => {
            if context.json {
                print_json(&json!({
                    "thread_id": thread.id,
                    "overlaps": overlaps,
                }))
            } else if overlaps {
                println!(
                    "The change of {} overlaps with newer work in the project and was left as it was.",
                    short_id(thread.id)
                );
                ExitCode::SUCCESS
            } else {
                println!("Caught up {}.", short_id(thread.id));
                ExitCode::SUCCESS
            }
        }
        Ok(_) => fail("Teryon answered unexpectedly."),
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Discards a thread's change, stopping its agent first if running.
pub async fn discard_change(context: Context, target: String) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    match client
        .call(Request::DiscardChange {
            thread_id: thread.id,
        })
        .await
    {
        Ok(_) => {
            if context.json {
                print_json(&json!({
                    "discarded": true,
                    "thread_id": thread.id,
                }))
            } else {
                println!("Discarded change of {}.", short_id(thread.id));
                ExitCode::SUCCESS
            }
        }
        Err(problem) => fail(&problem.to_string()),
    }
}

/// Sends a thread's change for review on GitHub, drafting a description when none is given.
pub async fn send_for_review(
    context: Context,
    target: String,
    description: Option<String>,
) -> ExitCode {
    let (mut client, snapshot, _) = match connect(&context).await {
        Ok(connected) => connected,
        Err(problem) => return fail(&problem),
    };
    let thread = match resolve_thread(&snapshot.threads, &target) {
        Ok(thread) => thread.clone(),
        Err(problem) => return fail(&problem.to_string()),
    };
    let description = match resolve_description(&mut client, thread.id, description).await {
        Ok(text) => text,
        Err(problem) => return fail(&problem),
    };
    match client
        .call(Request::SendForReview {
            thread_id: thread.id,
            description: description.clone(),
        })
        .await
    {
        Ok(_) => {
            if context.json {
                print_json(&json!({
                    "sent_for_review": true,
                    "thread_id": thread.id,
                    "description": description,
                }))
            } else {
                println!("Sent change of {} for review.", short_id(thread.id));
                ExitCode::SUCCESS
            }
        }
        Err(problem) => fail(&problem.to_string()),
    }
}

async fn resolve_description(
    client: &mut Client,
    thread_id: Uuid,
    given: Option<String>,
) -> Result<String, String> {
    if let Some(raw) = given {
        return read_message(&raw, stdin());
    }
    match client.call(Request::DraftDescription { thread_id }).await {
        Ok(Answer::Draft { text }) => Ok(text),
        Ok(_) => Err("Teryon answered unexpectedly.".to_string()),
        Err(problem) => Err(problem.to_string()),
    }
}
