//! Starts Claude Code and reads its report.
//!
//! Claude Code is run non-interactively with `-p` and prints one JSON object
//! per line. The shapes handled here were recorded from the real program; the
//! recordings live in `tests/samples/agents/`.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::agents::summary::summarise_input;
use tracing::warn;

use crate::agents::{
    command_names, typed_report, AgentDriver, EntryDraft, Invocation, NotUnderstood, Tokens, Turn,
};
use crate::settings::ClaudeSettings;
use crate::usage::limits::claude_limits;
use crate::usage::Limits;

/// Variables that make Claude Code bill an API key instead of the subscription.
const KEY_VARIABLES: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];

/// The tools Claude Code edits files with, switched off for a reviewer.
const READ_ONLY: &str = "Edit,Write,NotebookEdit";

pub struct ClaudeDriver {
    program: String,
    use_subscription: bool,
    /// The slash commands named by the last line, until they are taken.
    commands: Option<Vec<String>>,
    /// The conversation named by the last line, until it is taken.
    conversation: Option<String>,
    /// The model asked for, until Claude Code names the one it runs.
    model: Option<String>,
    /// The limits reported by the last line, until they are taken.
    limits: Option<Limits>,
}

impl ClaudeDriver {
    pub fn new(settings: &ClaudeSettings) -> Self {
        Self {
            program: settings.program.clone(),
            use_subscription: settings.use_subscription,
            commands: None,
            conversation: None,
            model: None,
            limits: None,
        }
    }
}

impl AgentDriver for ClaudeDriver {
    /// Builds the command line for one turn.
    ///
    /// The message comes last, after `--`, so a message that begins with a
    /// dash is not read as an option. A new conversation gets its identifier
    /// from Claude Code, which names it on the first line of its report.
    fn invocation(&mut self, turn: &Turn) -> Invocation {
        let mut arguments = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "auto",
        ]
        .map(String::from)
        .to_vec();
        if let Some(conversation) = &turn.conversation {
            arguments.extend(["--resume".to_string(), conversation.clone()]);
        }
        if let Some(model) = &turn.model {
            arguments.extend(["--model".to_string(), model.clone()]);
        }
        self.model = turn.model.clone();
        // The snapshot is off so a later turn of the same conversation is
        // told the branch the project directory is on then, not the one it
        // was on when the conversation started.
        if turn.read_only {
            arguments.extend(["--disallowedTools".to_string(), READ_ONLY.to_string()]);
        }
        if let Some(brief) = &turn.brief {
            arguments.extend([
                "--append-system-prompt".to_string(),
                brief.clone(),
                "--system-prompt-snapshot".to_string(),
                "off".to_string(),
            ]);
        }
        arguments.push("--".to_string());
        arguments.push(turn.message.clone());

        Invocation {
            program: self.program.clone(),
            arguments,
            removed_variables: if self.use_subscription {
                KEY_VARIABLES.to_vec()
            } else {
                Vec::new()
            },
        }
    }

    fn interpret(&mut self, line: &str) -> Result<Vec<EntryDraft>, NotUnderstood> {
        let (kind, report) = typed_report(line)?;
        match kind.as_str() {
            "assistant" => Ok(assistant_entries(&report)),
            "system" => {
                if report["subtype"].as_str() == Some("init") {
                    self.commands = offered_commands(&report);
                    self.conversation = report["session_id"].as_str().map(str::to_string);
                    if let Some(model) = report["model"].as_str() {
                        self.model = Some(model.to_string());
                    }
                }
                Ok(system_entries(&report))
            }
            "rate_limit_event" => {
                self.limits = claude_limits(&report, Utc::now());
                Ok(limit_entries(&report))
            }
            "result" => Ok(result_entries(&report)),
            "user" | "stream_event" => Ok(Vec::new()),
            _ => Err(NotUnderstood),
        }
    }

    fn take_commands(&mut self) -> Option<Vec<String>> {
        self.commands.take()
    }

    fn take_conversation(&mut self) -> Option<String> {
        self.conversation.take()
    }

    /// Claude Code names the model on the first line of every turn, as the full name: `claude-opus-5-5` for `opus`.
    fn model(&self) -> Option<String> {
        self.model.clone()
    }

    fn take_limits(&mut self) -> Option<Limits> {
        self.limits.take()
    }
}

/// The slash commands Claude Code names at the start of a turn that can be sent as a message.
///
/// It also names the ones that only work in its terminal, which are left
/// out here, as are its internal ones, whose names begin with two underscores.
/// A start without a list is logged, because it means the report changed
/// shape and the menu would silently stop offering the commands.
fn offered_commands(report: &Value) -> Option<Vec<String>> {
    let Some(names) = command_names(&report["slash_commands"]) else {
        warn!("Claude Code started a turn without naming its slash commands");
        return None;
    };
    let terminal_only = command_names(&report["terminal_slash_commands"]).unwrap_or_default();
    Some(
        names
            .into_iter()
            .filter(|name| !name.starts_with("__") && !terminal_only.contains(name))
            .collect(),
    )
}

/// Text the agent wrote and tools it started.
fn assistant_entries(report: &Value) -> Vec<EntryDraft> {
    let blocks = report["message"]["content"].as_array();
    blocks
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => {
                let text = block["text"].as_str().unwrap_or_default().trim();
                (!text.is_empty()).then(|| EntryDraft::agent_message(text))
            }
            Some("tool_use") => Some(EntryDraft::activity(
                block["name"].as_str().unwrap_or("tool"),
                &summarise_input(&block["input"]),
            )),
            _ => None,
        })
        .collect()
}

/// Of the program's own remarks, only a denied tool call is shown.
fn system_entries(report: &Value) -> Vec<EntryDraft> {
    if report["subtype"].as_str() != Some("permission_denied") {
        return Vec::new();
    }
    let tool = report["tool_name"].as_str().unwrap_or("a tool");
    let reason = report["message"].as_str().unwrap_or_default();
    vec![EntryDraft::notice(
        format!("Permission denied for {tool}. {reason}").trim(),
    )]
}

/// Says so when the subscription's allowance has run out.
///
/// The program reports the allowance after every model call. Any status that
/// starts with `allowed` means work continues, so only the others are shown.
fn limit_entries(report: &Value) -> Vec<EntryDraft> {
    let information = &report["rate_limit_info"];
    let status = information["status"].as_str().unwrap_or("allowed");
    if status.starts_with("allowed") {
        return Vec::new();
    }
    let resets = information["resetsAt"]
        .as_i64()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .map(|time| format!(" It resets at {} UTC.", time.format("%Y-%m-%d %H:%M")))
        .unwrap_or_default();
    vec![EntryDraft::notice(&format!(
        "The subscription's usage limit was reached.{resets}"
    ))]
}

/// The end of the turn, preceded by an error when the turn failed.
fn result_entries(report: &Value) -> Vec<EntryDraft> {
    let mut entries = Vec::new();
    let failed = report["is_error"].as_bool().unwrap_or(false);
    if failed {
        let text = report["result"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .or(report["subtype"].as_str())
            .unwrap_or("The agent reported a failure.");
        entries.push(EntryDraft::error(text));
    }
    let summary = EntryDraft::turn_summary(
        Tokens::sum(
            &report["usage"],
            &[
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            ],
            &["output_tokens"],
        ),
        report["num_turns"].as_u64(),
    );
    entries.push(if failed {
        summary.ended_early()
    } else {
        summary
    });
    entries
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lets_expect::lets_expect;
    use serde_json::{from_str, json};

    use super::*;
    use crate::agents::EntryKind;

    const FIRST: &str = include_str!("../../tests/samples/agents/claude-first.jsonl");
    const TOOLS: &str = include_str!("../../tests/samples/agents/claude-tools.jsonl");
    const DENIED: &str = include_str!("../../tests/samples/agents/claude-denied.jsonl");
    const OTHER_MODEL: &str = include_str!("../../tests/samples/agents/claude-other-model.jsonl");

    fn driver(use_subscription: bool) -> ClaudeDriver {
        ClaudeDriver::new(&ClaudeSettings {
            program: "claude".to_string(),
            use_subscription,
            models: None,
        })
    }

    fn turn(
        message: &str,
        conversation: Option<&str>,
        brief: Option<&str>,
        read_only: bool,
    ) -> Turn {
        Turn {
            message: message.to_string(),
            conversation: conversation.map(str::to_string),
            brief: brief.map(str::to_string),
            read_only,
            directory: PathBuf::from("/work/app"),
            writable_outside: Vec::new(),
            model: None,
        }
    }

    /// The same turn, on a chosen model.
    fn on_model(model: Option<&str>, turn: Turn) -> Turn {
        Turn {
            model: model.map(str::to_string),
            ..turn
        }
    }

    /// The model the driver says the turn runs on, after being asked for `asked` and reading `lines`.
    fn model_after(asked: Option<&str>, lines: &str) -> Option<String> {
        let mut driver = driver(true);
        driver.invocation(&Turn {
            model: asked.map(str::to_string),
            ..turn("Fix the bug", None, None, false)
        });
        for line in lines.lines() {
            driver.interpret(line).expect("the line is understood");
        }
        driver.model()
    }

    /// The commands the driver hands over after one report line.
    fn commands_after(report: Value) -> Option<Vec<String>> {
        let mut driver = driver(true);
        driver
            .interpret(&report.to_string())
            .expect("the line is understood");
        driver.take_commands()
    }

    /// The names of the limit windows the driver hands over after the given lines, and whether it hands them over again.
    fn windows_after(lines: Vec<Value>) -> (Option<Vec<String>>, bool) {
        let mut driver = driver(true);
        for line in lines {
            driver
                .interpret(&line.to_string())
                .expect("the line is understood");
        }
        let windows = driver.take_limits().map(|limits| {
            limits
                .windows
                .into_iter()
                .map(|window| window.name)
                .collect()
        });
        (windows, driver.take_limits().is_some())
    }

    fn limit_event(window: &str) -> Value {
        json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "unifiedWindows": {window: {"utilization": 0.1}}}})
    }

    /// The conversation the driver hands over after one report line.
    fn conversation_after(report: Value) -> Option<String> {
        let mut driver = driver(true);
        driver
            .interpret(&report.to_string())
            .expect("the line is understood");
        driver.take_conversation()
    }

    /// The commands the driver hands over the second time it is asked, after a start that named some.
    fn commands_taken_twice() -> Option<Vec<String>> {
        let mut driver = driver(true);
        driver
            .interpret(FIRST.lines().next().expect("a first line"))
            .expect("the line is understood");
        driver.take_commands();
        driver.take_commands()
    }

    /// The first line of a recording of the real program, which starts the report.
    fn first_line() -> Value {
        from_str(FIRST.lines().next().expect("a first line")).expect("a JSON line")
    }

    fn names(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(ToString::to_string).collect())
    }

    /// Runs a whole recording through the driver.
    fn entries(recording: &str) -> Vec<EntryDraft> {
        let mut driver = driver(true);
        let mut entries: Vec<EntryDraft> = recording
            .lines()
            .flat_map(|line| {
                driver
                    .interpret(line)
                    .expect("a line of the recording is understood")
            })
            .collect();
        entries.extend(driver.finish());
        entries
    }

    fn kinds(recording: &str) -> Vec<EntryKind> {
        entries(recording).iter().map(|entry| entry.kind).collect()
    }

    /// Whether the last entry a report gives is the summary of a turn that ended early.
    fn ends_early(report: Value) -> Option<bool> {
        interpret(report).ok()?.last().map(EntryDraft::is_early_end)
    }

    fn interpret(report: Value) -> Result<Vec<EntryDraft>, NotUnderstood> {
        driver(true).interpret(&report.to_string())
    }

    lets_expect! {
        expect(driver(use_subscription).invocation(&on_model(model, turn(message, conversation, brief, read_only)))) {
            let use_subscription = true;
            let message = "Fix the bug";
            let conversation: Option<&str> = None;
            let brief: Option<&str> = None;
            let read_only = false;
            let model: Option<&str> = None;

            // requirement: a chosen model is passed on, before the message
            when(model = Some("opus")) as pass_the_model to hold {
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--model", "opus", "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            // requirement: the model is passed when continuing a conversation too, so it can change between messages
            when(model = Some("sonnet"), conversation = Some("80bf47d7")) as pass_the_model_when_continuing to hold {
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--resume", "80bf47d7", "--model", "sonnet", "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            // requirement: the first message starts a new conversation, which Claude Code names
            to start_a_conversation {
                have(program.as_str()) equal("claude"),
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            // requirement: later messages resume the conversation Claude Code named
            when(conversation = Some("80bf47d7")) as resume_the_conversation to hold {
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--resume", "80bf47d7", "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            when(message = "--help") as keep_a_message_that_looks_like_an_option_last to hold {
                have(arguments[6].as_str()) equal("--"),
                have(arguments[7].as_str()) equal("--help")
            }

            // requirement: a worktree instruction is added beside the message, and is not frozen for the conversation
            when(brief = Some("Stay in the worktree.")) as pass_the_worktree_instruction to hold {
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--append-system-prompt", "Stay in the worktree.",
                    "--system-prompt-snapshot", "off",
                    "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            // requirement: a reviewer's turn has Claude's editing tools switched off
            when(read_only = true) as switch_off_editing to hold {
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--disallowedTools", "Edit,Write,NotebookEdit",
                    "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            // requirement: by default the API key variables are not passed to Claude
            to remove_the_key_variables {
                have(removed_variables.clone()) equal(vec!["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"])
            }

            // requirement: a configuration switch passes the environment unchanged
            when(use_subscription = false) as leave_the_environment_alone to hold {
                have(removed_variables.len()) equal(0)
            }
        }

        // requirement: the summary names the model that ran, not the one asked for
        expect(model_after(asked, lines)) {
            let asked: Option<&str> = None;
            let lines = "";

            when(asked = None) as nothing_is_asked_or_said to be_unknown { equal(None) }
            when(asked = Some("opus")) as nothing_is_said_yet to be_the_one_asked_for { equal(Some("opus".to_string())) }
            when(lines = FIRST) as the_default_runs to be_the_one_named { equal(Some("claude-fable-5-2".to_string())) }
            when(asked = Some("opus"), lines = OTHER_MODEL) as a_short_name_runs to be_its_full_name { equal(Some("claude-opus-5-5".to_string())) }
        }

        // requirement: a failed turn's summary says the turn ended early
        expect(ends_early(report)) {
            when(report = json!({"type": "result", "is_error": true, "subtype": "error_max_turns"})) as a_failed_turn to be_early { equal(Some(true)) }
            when(report = json!({"type": "result", "is_error": false})) as a_finished_turn to be_on_time { equal(Some(false)) }
        }

        expect(kinds(recording)) {
            when(recording = FIRST) as read_a_plain_answer to hold {
                equal(vec![EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
            when(recording = TOOLS) as read_tool_use_in_order to hold {
                equal(vec![
                    EntryKind::Activity, EntryKind::Activity, EntryKind::Activity, EntryKind::Activity,
                    EntryKind::Activity, EntryKind::AgentMessage, EntryKind::TurnSummary,
                ])
            }
            when(recording = DENIED) as read_denied_tool_calls to hold {
                equal(vec![
                    EntryKind::Activity, EntryKind::Notice, EntryKind::Activity, EntryKind::Notice,
                    EntryKind::AgentMessage, EntryKind::TurnSummary,
                ])
            }
        }

        expect(entries(recording)) {
            // requirement: text the agent writes becomes an agent message
            when(recording = FIRST) as give_the_answer to hold {
                have(first().cloned()) equal(Some(EntryDraft::agent_message("ready")))
            }

            // requirement: the end of the turn becomes a summary with the tokens read and written, cached ones counted as read
            when(recording = FIRST) as end_with_a_summary to hold {
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(Tokens { read: 24528, written: 4 }), Some(1))))
            }

            // requirement: each tool the agent uses becomes an activity line naming the tool and what it was used on
            when(recording = TOOLS) as name_the_tool_and_its_target to hold {
                have(first().cloned()) equal(Some(EntryDraft::activity("Bash", "Check repository state and existing worktrees"))),
                have(get(1).cloned()) equal(Some(EntryDraft::activity("Read", "/etc/shadow"))),
                have(get(3).cloned()) equal(Some(EntryDraft::activity("Write", "/work/sample-project--note/note.txt")))
            }

            // requirement: a tool call the permission mode denied becomes a notice
            when(recording = DENIED) as explain_a_denied_tool_call to hold {
                have(get(1).cloned()) equal(Some(EntryDraft::notice(
                    "Permission denied for Write. Claude requested permissions to write to /work/sample-project/x.txt, but you haven't granted it yet."
                )))
            }
        }

        expect(interpret(report)) {
            when(report = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "  "}]}})) as skip_blank_text to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": ""}]}})) as skip_thinking to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "assistant"})) as skip_a_message_without_content to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "input": {}}]}})) as name_an_unnamed_tool to hold {
                equal(Ok(vec![EntryDraft::activity("tool", "")]))
            }
            when(report = json!({"type": "system", "subtype": "init", "slash_commands": ["compact"]})) as show_nothing_for_the_start_of_the_report to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "system", "subtype": "permission_denied"})) as explain_a_denial_without_details to hold {
                equal(Ok(vec![EntryDraft::notice("Permission denied for a tool.")]))
            }
            when(report = json!({"type": "user"})) as skip_tool_results to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed"}})) as skip_an_allowance_that_is_not_used_up to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed_warning"}})) as skip_an_allowance_warning to hold {
                equal(Ok(Vec::new()))
            }
            when(report = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected", "resetsAt": 1794681000}})) as say_when_the_allowance_resets to hold {
                equal(Ok(vec![EntryDraft::notice("The subscription's usage limit was reached. It resets at 2026-11-14 18:30 UTC.")]))
            }
            when(report = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected"}})) as say_the_allowance_ran_out to hold {
                equal(Ok(vec![EntryDraft::notice("The subscription's usage limit was reached.")]))
            }
            when(report = json!({"type": "result", "is_error": true, "result": "Credit balance is too low", "subtype": "success"})) as show_a_failed_turn to hold {
                equal(Ok(vec![EntryDraft::error("Credit balance is too low"), EntryDraft::turn_summary(None, None).ended_early()]))
            }
            when(report = json!({"type": "result", "is_error": true, "subtype": "error_max_turns"})) as name_the_failure_when_there_is_no_text to hold {
                equal(Ok(vec![EntryDraft::error("error_max_turns"), EntryDraft::turn_summary(None, None).ended_early()]))
            }
            when(report = json!({"type": "result", "is_error": true})) as show_a_failure_without_details to hold {
                equal(Ok(vec![EntryDraft::error("The agent reported a failure."), EntryDraft::turn_summary(None, None).ended_early()]))
            }

            // requirement: a report line Teryon does not understand is skipped
            when(report = json!({"type": "something_new"})) as refuse_an_unknown_type to hold {
                equal(Err(NotUnderstood))
            }
        }
    }

    // requirement: the slash commands Claude Code names at the start of a turn are handed over, without the ones that need its terminal
    lets_expect! {
        expect(commands_after(report)) {
            when(report = first_line()) as read_them_from_a_recording to hold {
                equal(names(&["compact", "review"]))
            }
            when(report = json!({"type": "system", "subtype": "init", "slash_commands": ["compact", "review"]})) as keep_them_in_order to hold {
                equal(names(&["compact", "review"]))
            }
            when(report = json!({"type": "system", "subtype": "init", "slash_commands": ["compact", "doctor", "__internal"], "terminal_slash_commands": ["doctor"]})) as leave_out_what_needs_a_terminal_and_what_is_internal to hold {
                equal(names(&["compact"]))
            }
            when(report = json!({"type": "system", "subtype": "init", "slash_commands": []})) as hand_over_an_empty_list to hold {
                equal(names(&[]))
            }
            when(report = json!({"type": "system", "subtype": "init", "slash_commands": ["compact", 5, null, {"name": "x"}]})) as leave_out_what_is_not_a_name to hold {
                equal(names(&["compact"]))
            }
            when(report = json!({"type": "system", "subtype": "init"})) as hand_over_nothing_when_none_are_named to hold {
                equal(None)
            }
            when(report = json!({"type": "system", "subtype": "init", "slash_commands": "compact"})) as hand_over_nothing_for_something_else_than_a_list to hold {
                equal(None)
            }
            when(report = json!({"type": "system", "subtype": "permission_denied", "slash_commands": ["compact"]})) as hand_over_nothing_for_another_remark to hold {
                equal(None)
            }
            when(report = json!({"type": "assistant", "slash_commands": ["compact"]})) as hand_over_nothing_for_another_line to hold {
                equal(None)
            }
        }

        // requirement: the conversation Claude Code names at the start of a turn is handed over, so the next message continues it
        expect(conversation_after(report)) {
            when(report = first_line()) as read_it_from_a_recording to hold {
                equal(Some("80bf47d7-616f-4148-ab7c-41ff76681fcc".to_string()))
            }
            when(report = json!({"type": "system", "subtype": "init"})) as hand_over_nothing_when_none_is_named to hold {
                equal(None)
            }
            when(report = json!({"type": "system", "subtype": "init", "session_id": 5})) as hand_over_nothing_for_something_else_than_text to hold {
                equal(None)
            }
            when(report = json!({"type": "result", "session_id": "80bf47d7"})) as hand_over_nothing_for_another_line to hold {
                equal(None)
            }
        }

        // requirement: Claude's rate_limit_event becomes the agent's latest reading, the last of a turn winning
        expect(windows_after(lines)) as limits_handed_over {
            when(lines = vec![limit_event("five_hour")]) as after_one_event to give_it_once {
                equal((Some(vec!["five_hour".to_string()]), false))
            }
            when(lines = vec![limit_event("five_hour"), limit_event("seven_day")]) as after_several_events to give_the_last {
                equal((Some(vec!["seven_day".to_string()]), false))
            }
            when(lines = vec![json!({"type": "rate_limit_event"})]) as after_an_event_without_windows to give_nothing {
                equal((None, false))
            }
            when(lines = vec![first_line()]) as without_an_event to give_nothing {
                equal((None, false))
            }
        }

        expect(commands_taken_twice()) as hand_them_over_once {
            to hold { equal(None) }
        }
    }
}
