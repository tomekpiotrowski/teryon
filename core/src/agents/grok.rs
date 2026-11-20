//! Starts Grok and reads its report.
//!
//! Grok is run non-interactively with `--single` and prints one JSON object
//! per line. Unlike Claude Code it sends text a few words at a time, so the
//! driver collects the pieces and releases a message when something else
//! follows it. The shapes handled here were recorded from the real program;
//! the recordings live in `tests/samples/agents/`.
//!
//! Grok names a conversation only on the last line of a turn, which a
//! stopped turn never prints. So the driver gives a new conversation its
//! identifier itself, and hands it over as soon as Grok has started.
//!
//! Grok runs without asking for approval, because a command it asks about
//! ends its turn: nobody is at its prompt to answer. Its sandbox sets the
//! limits instead, see [`crate::agents::grok_sandbox`].

use std::mem::take;

use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::agents::summary::summarise_input;

use crate::agents::grok_sandbox::{GrokSandbox, REVIEWER_PROFILE};
use crate::agents::{
    command_names, typed_report, AgentDriver, EntryDraft, Invocation, NotUnderstood, Tokens, Turn,
};
use crate::settings::GrokSettings;

/// The tools Grok edits files with, removed for a reviewer.
const EDITING_TOOLS: &str = "write,search_replace,apply_patch,hashline_edit";

pub struct GrokDriver {
    program: String,
    sandbox: GrokSandbox,
    /// Pieces of the message the agent is writing.
    text: String,
    /// The slash commands named by the last line, until they are taken.
    commands: Option<Vec<String>>,
    /// Whether a command waited for approval nobody could give, which ends Grok's turn.
    unapproved: bool,
    /// The conversation started for this turn, until it is taken.
    conversation: Option<String>,
}

impl GrokDriver {
    pub fn new(settings: &GrokSettings) -> Self {
        Self {
            program: settings.program.clone(),
            sandbox: GrokSandbox::new(settings),
            text: String::new(),
            commands: None,
            unapproved: false,
            conversation: None,
        }
    }

    /// Releases the collected text as a message, when there is any.
    fn message(&mut self) -> Vec<EntryDraft> {
        let text = take(&mut self.text);
        let text = text.trim();
        if text.is_empty() {
            Vec::new()
        } else {
            vec![EntryDraft::agent_message(text)]
        }
    }
}

impl AgentDriver for GrokDriver {
    /// Builds the command line for one turn.
    ///
    /// The message is attached to its option with `=`, so a message that
    /// begins with a dash is not read as an option.
    ///
    /// Only a new conversation names its sandbox: Grok refuses to resume a
    /// conversation under another one, and restores the one it began with.
    /// An author's profile is written on every turn all the same, so a
    /// resumed conversation finds it, with the folders the settings name now.
    fn invocation(&mut self, turn: &Turn) -> Invocation {
        let profile = if turn.read_only {
            REVIEWER_PROFILE.to_string()
        } else {
            self.sandbox
                .author_profile(&turn.directory, &turn.writable_outside)
        };
        let (conversation_option, conversation) = match &turn.conversation {
            Some(conversation) => ("--resume", conversation.clone()),
            None => {
                let started = Uuid::new_v4().to_string();
                self.conversation = Some(started.clone());
                ("--session-id", started)
            }
        };
        let mut arguments = vec![
            format!("--single={}", turn.message),
            "--output-format".to_string(),
            "streaming-json".to_string(),
            "--always-approve".to_string(),
        ];
        if turn.conversation.is_none() {
            arguments.push("--sandbox".to_string());
            arguments.push(profile);
        }
        if turn.read_only {
            arguments.push("--disallowed-tools".to_string());
            arguments.push(EDITING_TOOLS.to_string());
        }
        if let Some(brief) = &turn.brief {
            arguments.push("--rules".to_string());
            arguments.push(brief.clone());
        }
        arguments.push(conversation_option.to_string());
        arguments.push(conversation);

        Invocation {
            program: self.program.clone(),
            arguments,
            removed_variables: Vec::new(),
        }
    }

    fn interpret(&mut self, line: &str) -> Result<Vec<EntryDraft>, NotUnderstood> {
        let (kind, report) = typed_report(line)?;
        match kind.as_str() {
            "text" => {
                self.text
                    .push_str(report["data"].as_str().unwrap_or_default());
                Ok(Vec::new())
            }
            "usage" => Ok(self.message()),
            "tool_call" => {
                let mut entries = self.message();
                entries.push(EntryDraft::activity(
                    report["toolName"].as_str().unwrap_or("tool"),
                    &summarise_input(&report["rawInput"]),
                ));
                Ok(entries)
            }
            "tool_call_update" => {
                let entries = failed_tool_entries(&report);
                self.unapproved |= unapproved(&report);
                Ok(entries)
            }
            "end" => {
                let mut entries = self.message();
                entries.extend(end_entries(&report, self.unapproved));
                Ok(entries)
            }
            "available_commands" => {
                // Grok names its commands several times a turn. A line without
                // them is logged, because it means the report changed shape and
                // the menu would silently stop offering the commands.
                self.commands = command_names(&report["commands"]);
                if self.commands.is_none() {
                    warn!("Grok listed its commands without naming them");
                }
                Ok(Vec::new())
            }
            "thought" => Ok(Vec::new()),
            _ => Err(NotUnderstood),
        }
    }

    fn take_commands(&mut self) -> Option<Vec<String>> {
        self.commands.take()
    }

    /// Hands over the conversation started for this turn.
    ///
    /// The runner asks only after Grok's first line: before it Grok may not
    /// have stored the conversation, and resuming one it never stored fails.
    fn take_conversation(&mut self) -> Option<String> {
        self.conversation.take()
    }

    fn finish(&mut self) -> Vec<EntryDraft> {
        self.message()
    }
}

/// A tool call that did not run, with the reason the program gives.
///
/// This is how Grok reports a call its permission mode blocked.
fn failed_tool_entries(report: &Value) -> Vec<EntryDraft> {
    if report["status"].as_str() != Some("failed") {
        return Vec::new();
    }
    if unapproved(report) {
        return vec![EntryDraft::notice(UNAPPROVED)];
    }
    let reason = report["content"][0]["content"]["text"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("A tool call failed.");
    vec![EntryDraft::notice(reason.trim())]
}

/// What the thread says when Grok asked for approval of a command.
///
/// Grok calls this "User cancelled", which reads as if the person did
/// something. In Teryon nobody is at Grok's prompt to approve anything.
const UNAPPROVED: &str = "Grok wanted approval for this command, and there is nobody at its prompt to give it, so the command did not run.";

/// What the thread says when such a command ended Grok's turn.
const STOPPED_UNAPPROVED: &str =
    "Grok stopped, because a command it wanted to run needed approval.";

/// Whether a tool call was not run for want of an approval nobody could give.
///
/// Grok marks it as not run because it was cancelled; older versions only
/// said so in the text.
fn unapproved(report: &Value) -> bool {
    report["status"].as_str() == Some("failed")
        && (report["_meta"]["toolNotRun"].as_str() == Some("cancelled")
            || report["content"][0]["content"]["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("User cancelled the execution")))
}

/// The end of the turn, preceded by a notice when it did not end normally, and then marked as early.
fn end_entries(report: &Value, unapproved: bool) -> Vec<EntryDraft> {
    let mut entries = Vec::new();
    let summary = EntryDraft::turn_summary(
        Tokens::sum(
            &report["usage"],
            &[
                "input_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            ],
            &["output_tokens"],
        ),
        report["num_turns"].as_u64(),
    );
    match report["stopReason"].as_str() {
        Some("end_turn") | None => entries.push(summary),
        Some("cancelled") if unapproved => {
            entries.push(EntryDraft::notice(STOPPED_UNAPPROVED));
            entries.push(summary.ended_early());
        }
        Some(reason) => {
            entries.push(EntryDraft::notice(&format!(
                "The agent stopped early: {reason}."
            )));
            entries.push(summary.ended_early());
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;
    use serde_json::{from_str, json};

    use std::env::temp_dir;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::agents::grok_sandbox::profile_name;
    use crate::agents::EntryKind;

    const FIRST: &str = include_str!("../../tests/samples/agents/grok-first.jsonl");
    const TOOLS: &str = include_str!("../../tests/samples/agents/grok-tools.jsonl");

    /// The folder the turns run in.
    const PROJECT: &str = "/work/app";

    /// A driver whose sandbox profiles go to a folder of the tests' own.
    fn driver() -> GrokDriver {
        GrokDriver::new(&GrokSettings {
            program: "grok".to_string(),
            home: Some(temp_dir().join("teryon-grok-driver-tests")),
            writable: vec!["/opt/cache".to_string()],
        })
    }

    /// The profile an author in the project works in.
    fn profile() -> String {
        profile_name(Path::new(PROJECT))
    }

    const KNOWN: &str = "9da215a9-ae45-46d5-9c87-8f5220bd7207";

    fn turn(message: &str, conversation: Option<&str>, brief: Option<&str>) -> Turn {
        Turn {
            read_only: false,
            ..read_only_turn(message, conversation, brief)
        }
    }

    fn read_only_turn(message: &str, conversation: Option<&str>, brief: Option<&str>) -> Turn {
        Turn {
            message: message.to_string(),
            conversation: conversation.map(str::to_string),
            brief: brief.map(str::to_string),
            read_only: true,
            directory: PathBuf::from(PROJECT),
            writable_outside: Vec::new(),
        }
    }

    /// The commands the driver hands over after each of some report lines.
    fn commands_after(lines: Vec<Value>) -> Vec<Option<Vec<String>>> {
        let mut driver = driver();
        lines
            .iter()
            .map(|line| {
                driver
                    .interpret(&line.to_string())
                    .expect("the line is understood");
                driver.take_commands()
            })
            .collect()
    }

    /// The first lines of a recording of the real program.
    fn recorded(count: usize) -> Vec<Value> {
        FIRST
            .lines()
            .take(count)
            .map(|line| from_str(line).expect("a JSON line"))
            .collect()
    }

    fn names(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(ToString::to_string).collect())
    }

    /// Runs report lines through one driver, then asks for what it still holds.
    fn run(lines: &[String]) -> Vec<EntryDraft> {
        let mut driver = driver();
        let mut entries: Vec<EntryDraft> = lines
            .iter()
            .flat_map(|line| driver.interpret(line).expect("the line is understood"))
            .collect();
        entries.extend(driver.finish());
        entries
    }

    fn entries(recording: &str) -> Vec<EntryDraft> {
        run(&recording.lines().map(String::from).collect::<Vec<_>>())
    }

    fn kinds(recording: &str) -> Vec<EntryKind> {
        entries(recording).iter().map(|entry| entry.kind).collect()
    }

    /// What the program says when its permission mode blocks a tool call.
    const BLOCKED: &str =
        "Tool `run_terminal_command` was not executed: Auto mode blocked this action";

    fn text_of(entries: &[EntryDraft], index: usize) -> String {
        entries[index].body["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn reports(reports: Vec<Value>) -> Vec<EntryDraft> {
        run(&reports.iter().map(Value::to_string).collect::<Vec<_>>())
    }

    /// What starting a turn gives.
    struct Started {
        /// The arguments without the conversation they name last.
        arguments: Vec<String>,
        /// Whether the conversation named is a fresh identifier.
        fresh: bool,
        /// Whether the driver hands over the conversation the arguments name.
        handed_over: bool,
        /// What the driver hands over when asked again.
        again: Option<String>,
    }

    fn started(turn: &Turn) -> Started {
        let mut driver = driver();
        let mut arguments = driver.invocation(turn).arguments;
        let named = arguments.pop();
        let handed_over = driver.take_conversation();
        Started {
            fresh: named
                .as_deref()
                .is_some_and(|named| Uuid::parse_str(named).is_ok() && named != KNOWN),
            handed_over: handed_over.is_some() && handed_over == named,
            again: driver.take_conversation(),
            arguments,
        }
    }

    lets_expect! {
        expect(driver().invocation(&turn(message, conversation, brief))) {
            let message = "Fix the bug";
            let conversation = Some(KNOWN);
            let brief: Option<&str> = None;

            // requirement: later messages resume the conversation
            to resume_the_conversation {
                have(program.as_str()) equal("grok"),
                have(arguments.clone()) equal([
                    "--single=Fix the bug", "--output-format", "streaming-json", "--always-approve",
                    "--resume", KNOWN,
                ].map(String::from).to_vec()),
                have(removed_variables.len()) equal(0)
            }

            when(message = "--help") as keep_a_message_that_looks_like_an_option_attached to hold {
                have(arguments[0].as_str()) equal("--single=--help")
            }

            // requirement: a worktree instruction is added beside the message
            when(brief = Some("Stay in the worktree.")) as pass_the_worktree_instruction to hold {
                have(arguments.clone()) equal([
                    "--single=Fix the bug", "--output-format", "streaming-json", "--always-approve",
                    "--rules", "Stay in the worktree.",
                    "--resume", KNOWN,
                ].map(String::from).to_vec())
            }
        }

        // requirement: the first message starts a new conversation, with an identifier the driver chooses and hands over once
        // requirement: Grok runs without asking for approval, held in by the project's sandbox profile, which a new conversation names
        expect(started(&turn("Fix the bug", None, None))) as a_new_conversation {
            to start_one_and_hand_it_over {
                have(arguments.clone()) equal([
                    "--single=Fix the bug", "--output-format", "streaming-json", "--always-approve",
                    "--sandbox", &profile(), "--session-id",
                ].map(String::from).to_vec()),
                have(fresh) be_true,
                have(handed_over) be_true,
                have(again.clone()) equal(None)
            }
        }

        // requirement: a resumed conversation is not handed over again
        expect(started(&turn("Fix the bug", Some(KNOWN), None)).handed_over) as a_resumed_conversation {
            to be_left_as_it_is { be_false }
        }

        expect(kinds(recording)) {
            when(recording = FIRST) as read_a_plain_answer to hold {
                equal(vec![EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
            when(recording = TOOLS) as read_tool_use_in_order to hold {
                equal(vec![
                    EntryKind::AgentMessage, EntryKind::Activity, EntryKind::Activity, EntryKind::Notice,
                    EntryKind::Activity, EntryKind::AgentMessage, EntryKind::TurnSummary,
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
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(Tokens { read: 18779, written: 31 }), Some(1))))
            }

            when(recording = TOOLS) as join_the_pieces_of_a_message to hold {
                have(first().cloned()) equal(Some(EntryDraft::agent_message(
                    "I'll create `note.txt`, check its size, and see whether `/etc/shadow` is readable."
                )))
            }

            // requirement: each tool the agent uses becomes an activity line naming the tool and what it was used on
            when(recording = TOOLS) as name_the_tool_and_its_target to hold {
                have(get(1).cloned()) equal(Some(EntryDraft::activity("write", "/work/sample-project/note.txt"))),
                have(get(2).cloned()) equal(Some(EntryDraft::activity("run_terminal_command", "Count note.txt and test shadow read")))
            }

        }

        // requirement: a reviewer's turn has Grok's editing tools removed
        expect(driver().invocation(&read_only_turn("Review the change.", Some(KNOWN), None)).arguments) as a_reviewers_turn {
            to remove_the_editing_tools {
                equal([
                    "--single=Review the change.", "--output-format", "streaming-json", "--always-approve",
                    "--disallowed-tools", "write,search_replace,apply_patch,hashline_edit",
                    "--resume", KNOWN,
                ].map(String::from).to_vec())
            }
        }

        // requirement: a reviewer's new conversation runs in Grok's read-only sandbox
        expect(started(&read_only_turn("Review the change.", None, None)).arguments) as a_reviewers_first_turn {
            to read_only {
                equal([
                    "--single=Review the change.", "--output-format", "streaming-json", "--always-approve",
                    "--sandbox", "read-only",
                    "--disallowed-tools", "write,search_replace,apply_patch,hashline_edit", "--session-id",
                ].map(String::from).to_vec())
            }
        }

        // requirement: a tool call the permission mode denied becomes a notice
        expect(text_of(&entries(TOOLS), 3).starts_with(BLOCKED)) as a_blocked_tool_call {
            to be_explained { be_true }
        }

        expect(reports(lines)) {
            when(lines = vec![json!({"type": "text", "data": "Half a mess"})]) as release_text_when_the_program_ends to hold {
                equal(vec![EntryDraft::agent_message("Half a mess")])
            }
            when(lines = vec![json!({"type": "text", "data": "   "}), json!({"type": "usage"})]) as skip_blank_text to hold {
                equal(Vec::new())
            }
            when(lines = vec![json!({"type": "thought", "data": "Thinking"})]) as skip_thoughts to hold {
                equal(Vec::new())
            }
            when(lines = vec![json!({"type": "tool_call"})]) as name_an_unnamed_tool to hold {
                equal(vec![EntryDraft::activity("tool", "")])
            }
            when(lines = vec![json!({"type": "tool_call_update", "status": "completed"})]) as skip_a_finished_tool to hold {
                equal(Vec::new())
            }
            when(lines = vec![json!({"type": "tool_call_update", "status": "failed"})]) as mention_a_failure_without_details to hold {
                equal(vec![EntryDraft::notice("A tool call failed.")])
            }
            when(lines = vec![json!({"type": "end", "stopReason": "max_turns"})]) as say_why_the_turn_ended_early to hold {
                equal(vec![EntryDraft::notice("The agent stopped early: max_turns."), EntryDraft::turn_summary(None, None).ended_early()])
            }
            // requirement: a command nobody could approve is explained as that, not as something the person cancelled
            when(lines = vec![
                json!({"type": "tool_call_update", "status": "failed", "content": [{"type": "content", "content": {"type": "text", "text": "User cancelled the execution for tool `run_terminal_command`"}}], "_meta": {"toolNotRun": "cancelled"}}),
                json!({"type": "end", "stopReason": "cancelled"}),
            ]) as explain_a_command_nobody_could_approve to hold {
                equal(vec![
                    EntryDraft::notice(UNAPPROVED),
                    EntryDraft::notice(STOPPED_UNAPPROVED),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            when(lines = vec![
                json!({"type": "tool_call_update", "status": "failed", "content": [{"type": "content", "content": {"type": "text", "text": "User cancelled the execution for tool `run_terminal_command`"}}]}),
            ]) as recognise_it_by_its_text_alone to hold {
                equal(vec![EntryDraft::notice(UNAPPROVED)])
            }
            when(lines = vec![json!({"type": "end", "stopReason": "cancelled"})]) as leave_another_cancellation_as_it_is to hold {
                equal(vec![EntryDraft::notice("The agent stopped early: cancelled."), EntryDraft::turn_summary(None, None).ended_early()])
            }
            when(lines = vec![json!({"type": "end"})]) as end_without_details to hold {
                equal(vec![EntryDraft::turn_summary(None, None)])
            }
        }

        // requirement: a report line Teryon does not understand is skipped
        expect(driver().interpret(line)) {
            when(line = r#"{"type":"something_new"}"#) as refuse_an_unknown_type to hold { equal(Err(NotUnderstood)) }
            when(line = "plain text") as refuse_text to hold { equal(Err(NotUnderstood)) }
        }
    }

    // requirement: the slash commands Grok names during a turn are handed over
    lets_expect! {
        expect(commands_after(lines)) {
            when(lines = recorded(3)) as read_them_from_a_recording to hold {
                equal(vec![names(&["compact", "review"]), names(&["compact", "review"]), None])
            }
            when(lines = vec![json!({"type": "available_commands", "tools": ["grep"], "commands": ["compact", "user:plan"]})]) as keep_them_in_order to hold {
                equal(vec![names(&["compact", "user:plan"])])
            }
            when(lines = vec![json!({"type": "available_commands", "commands": []})]) as hand_over_an_empty_list to hold {
                equal(vec![names(&[])])
            }
            when(lines = vec![json!({"type": "available_commands", "commands": ["compact", 5, null]})]) as leave_out_what_is_not_a_name to hold {
                equal(vec![names(&["compact"])])
            }
            when(lines = vec![json!({"type": "available_commands"})]) as hand_over_nothing_when_none_are_named to hold {
                equal(vec![None])
            }
            when(lines = vec![json!({"type": "available_commands", "commands": "compact"})]) as hand_over_nothing_for_something_else_than_a_list to hold {
                equal(vec![None])
            }
            when(lines = vec![json!({"type": "thought", "commands": ["compact"]}), json!({"type": "text", "data": "hi", "commands": ["compact"]})]) as hand_over_nothing_for_other_lines to hold {
                equal(vec![None, None])
            }
            when(lines = vec![json!({"type": "available_commands", "commands": ["compact"]}), json!({"type": "thought"})]) as hand_them_over_once to hold {
                equal(vec![names(&["compact"]), None])
            }
        }
    }
}
