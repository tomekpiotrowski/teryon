//! Starts Antigravity (`agy`) for Gemini models and reads its report.
//!
//! Antigravity is run non-interactively with `-p=<message> --output-format
//! stream-json` and prints one JSON object per line, tagged by `event`:
//! `init`, `step_update` and `result`. The shapes handled here were recorded
//! from the real program; the recordings live in `tests/samples/agents/`.
//!
//! An author runs with `--dangerously-skip-permissions`, since nobody is at
//! its prompt to approve edits or commands. A reviewer runs inside
//! Antigravity's terminal sandbox (`--sandbox`, `proceed-in-sandbox`) with
//! `read_file(*)` allowed and `write_file(*)` denied: `write_file(*)` blocks
//! every Antigravity file-editing tool and mounts the workspace read-only
//! inside the sandbox, so shell commands such as `sed -i` or `git commit`
//! fail with a read-only filesystem error, while read tools and reading
//! commands such as `git diff` run without prompting. Any action outside
//! those rules is auto-denied immediately in headless mode instead of waiting
//! for approval.
//!
//! On resumed conversations, `result.usage` is cumulative across the whole
//! conversation, whereas each completed `agent_response` step carries the
//! tokens of that step alone. The driver sums the per-step counts for the
//! current turn, counting cached reads as read and leaving `thinking_tokens`
//! out because Antigravity already includes them in `output_tokens`. On a
//! resumed turn with no per-step counts, it reports no tokens rather than
//! charging the whole conversation again.

use std::collections::{BTreeMap, HashSet};
use std::mem::take;

use serde_json::{from_str, Value};

use crate::agents::summary::summarise_input;
use crate::agents::{AgentDriver, EntryDraft, Invocation, NotUnderstood, Tokens, Turn};
use crate::settings::GeminiSettings;

/// Session settings for a reviewer: OS sandbox on, reads allowed, all writes denied.
///
/// `agentMode` is cleared so `agy -p` does not fall back to `auto` mode, which
/// would auto-approve edits or unsandboxed commands. `write_file(*)` denies
/// Antigravity's file-editing tools and makes the terminal sandbox mount the
/// workspace read-only.
const REVIEWER_SETTINGS: &str = r#"{"agentMode":"","enableTerminalSandbox":true,"toolPermission":"proceed-in-sandbox","permissions":{"allow":["read_file(*)"],"deny":["write_file(*)"]}}"#;

/// What the thread says when an action needing approval cut Gemini's turn short.
const STOPPED_UNAPPROVED: &str =
    "Gemini stopped, because an action it wanted to run needed approval.";

pub struct GeminiDriver {
    program: String,
    /// Pieces of the message the agent is writing in the current response step.
    text: String,
    /// Whether any agent message has already been released during this turn.
    emitted_text: bool,
    /// Whether this turn resumes an earlier conversation, making `result.usage` cumulative.
    resumed: bool,
    /// The conversation named by the last line, until it is taken.
    conversation: Option<String>,
    /// The model asked for, until Antigravity names the one it runs.
    model: Option<String>,
    /// Indexed tool steps already shown as activity, so later updates do not repeat them.
    shown_steps: HashSet<u64>,
    /// Indexed tool steps seen active without details yet, held until details arrive or the step ends.
    pending_steps: BTreeMap<u64, (String, String)>,
    /// An unindexed tool step seen active, emitted once when it finishes or the program ends.
    pending_unindexed: Option<(String, String)>,
    /// Tokens read and written across the response steps of this turn alone.
    step_tokens: Option<Tokens>,
}

impl GeminiDriver {
    pub fn new(settings: &GeminiSettings) -> Self {
        Self {
            program: settings.program.clone(),
            text: String::new(),
            emitted_text: false,
            resumed: false,
            conversation: None,
            model: None,
            shown_steps: HashSet::new(),
            pending_steps: BTreeMap::new(),
            pending_unindexed: None,
            step_tokens: None,
        }
    }

    /// Releases the collected text as a message, when there is any.
    fn message(&mut self) -> Vec<EntryDraft> {
        let text = take(&mut self.text);
        let text = text.trim();
        if text.is_empty() {
            Vec::new()
        } else {
            self.emitted_text = true;
            vec![EntryDraft::agent_message(text)]
        }
    }

    /// Adds one response step's tokens into this turn's running total.
    fn add_tokens(&mut self, usage: &Value) {
        let Some(step) = Tokens::sum(
            usage,
            &["input_tokens", "cache_read_tokens"],
            &["output_tokens"],
        ) else {
            return;
        };
        self.step_tokens = Some(match self.step_tokens {
            Some(total) => Tokens {
                read: total.read + step.read,
                written: total.written + step.written,
            },
            None => step,
        });
    }

    fn step_entries(&mut self, update: &Value) -> Vec<EntryDraft> {
        match update["step_type"].as_str() {
            Some("agent_response") => {
                if let Some(delta) = update["text_delta"].as_str() {
                    self.text.push_str(delta);
                }
                if update["state"].as_str() == Some("DONE") {
                    self.add_tokens(&update["usage"]);
                    self.message()
                } else {
                    Vec::new()
                }
            }
            Some("tool") => {
                let mut entries = self.message();
                let tool = non_empty(&update["tool_name"])
                    .or_else(|| non_empty(&update["tool_info"]["name"]))
                    .unwrap_or("tool");
                let summary = summarise_input(&update["tool_info"]["parameters"]);
                let active = update["state"].as_str() == Some("ACTIVE");
                match update["step_index"].as_u64() {
                    Some(index) => {
                        if !self.shown_steps.contains(&index) {
                            if !active || !summary.is_empty() {
                                self.shown_steps.insert(index);
                                self.pending_steps.remove(&index);
                                entries.push(EntryDraft::activity(tool, &summary));
                            } else {
                                self.pending_steps
                                    .insert(index, (tool.to_string(), summary));
                            }
                        }
                    }
                    None => {
                        if active {
                            self.pending_unindexed = Some((tool.to_string(), summary));
                        } else {
                            self.pending_unindexed = None;
                            entries.push(EntryDraft::activity(tool, &summary));
                        }
                    }
                }
                if update["state"].as_str() == Some("ERROR") {
                    let reason =
                        error_text(&update["tool_info"]["error"]).unwrap_or("A tool call failed.");
                    entries.push(EntryDraft::notice(reason));
                }
                entries
            }
            _ => Vec::new(),
        }
    }

    fn result_entries(&mut self, result: &Value) -> Vec<EntryDraft> {
        let mut entries = self.message();
        let failed = result["status"].as_str() == Some("ERROR") || has_error(&result["error"]);
        if !failed && !self.emitted_text {
            if let Some(response) = non_empty(&result["response"]) {
                self.emitted_text = true;
                entries.push(EntryDraft::agent_message(response));
            }
        }
        let stopped_unapproved = !failed
            && !self.emitted_text
            && result["denied_actions"]
                .as_array()
                .is_some_and(|actions| !actions.is_empty());
        if failed {
            let reason = error_text(&result["error"])
                .or_else(|| non_empty(&result["finished_reason"]))
                .unwrap_or("The agent reported a failure.");
            entries.push(EntryDraft::error(reason));
        } else if stopped_unapproved {
            entries.push(EntryDraft::notice(STOPPED_UNAPPROVED));
        }
        let tokens = self.step_tokens.or_else(|| {
            if self.resumed {
                return None;
            }
            let reported = Tokens::sum(
                &result["usage"],
                &["input_tokens", "cache_read_tokens"],
                &["output_tokens"],
            );
            if failed
                && reported
                    == Some(Tokens {
                        read: 0,
                        written: 0,
                    })
            {
                None
            } else {
                reported
            }
        });
        let summary = EntryDraft::turn_summary(tokens, None);
        entries.push(if failed || stopped_unapproved {
            summary.ended_early()
        } else {
            summary
        });
        entries
    }
}

impl AgentDriver for GeminiDriver {
    /// Builds the command line for one turn.
    ///
    /// The message is attached to `-p` with `=`, so a message that begins with
    /// a dash is not read as another flag. Teryon's instruction goes in the
    /// message, see [`with_brief`], so later turns learn about a new worktree
    /// or stage.
    fn invocation(&mut self, turn: &Turn) -> Invocation {
        let mut arguments = vec![
            format!("-p={}", with_brief(&turn.message, turn.brief.as_deref())),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        if turn.read_only {
            arguments.extend([
                "--sandbox".to_string(),
                "--settings".to_string(),
                REVIEWER_SETTINGS.to_string(),
            ]);
        } else {
            arguments.push("--dangerously-skip-permissions".to_string());
            for folder in &turn.writable_outside {
                arguments.extend([
                    "--add-dir".to_string(),
                    folder.to_string_lossy().into_owned(),
                ]);
            }
        }
        self.resumed = turn.conversation.is_some();
        if let Some(conversation) = &turn.conversation {
            arguments.extend(["--conversation".to_string(), conversation.clone()]);
        }
        if let Some(model) = &turn.model {
            arguments.extend(["--model".to_string(), model.clone()]);
        }
        self.model = turn.model.clone();

        Invocation {
            program: self.program.clone(),
            arguments,
            removed_variables: Vec::new(),
        }
    }

    fn interpret(&mut self, line: &str) -> Result<Vec<EntryDraft>, NotUnderstood> {
        let report: Value = from_str(line).map_err(|_| NotUnderstood)?;
        let event = report
            .get("event")
            .and_then(Value::as_str)
            .ok_or(NotUnderstood)?;
        match event {
            "init" => {
                if let Some(id) = non_empty(&report["conversation_id"]) {
                    self.conversation = Some(id.to_string());
                }
                if let Some(model) = non_empty(&report["init"]["model"]) {
                    self.model = Some(model.to_string());
                }
                Ok(Vec::new())
            }
            "step_update" => Ok(self.step_entries(&report["step_update"])),
            "result" => Ok(self.result_entries(&report["result"])),
            _ => Err(NotUnderstood),
        }
    }

    /// Antigravity does not list its slash commands in its stream-json report.
    fn take_commands(&mut self) -> Option<Vec<String>> {
        None
    }

    fn take_conversation(&mut self) -> Option<String> {
        self.conversation.take()
    }

    /// The model Antigravity reported on `init`, else the one asked for.
    ///
    /// On its default model Antigravity leaves `init.model` out, so nothing is
    /// known when no model was asked for.
    fn model(&self) -> Option<String> {
        self.model.clone()
    }

    fn finish(&mut self) -> Vec<EntryDraft> {
        let mut entries = Vec::new();
        for (index, (tool, summary)) in take(&mut self.pending_steps) {
            if self.shown_steps.insert(index) {
                entries.push(EntryDraft::activity(&tool, &summary));
            }
        }
        if let Some((tool, summary)) = self.pending_unindexed.take() {
            entries.push(EntryDraft::activity(&tool, &summary));
        }
        entries.extend(self.message());
        entries
    }
}

/// The message, preceded by Teryon's instruction for this turn.
///
/// Antigravity takes no extra system-prompt flag on the command line, and the
/// instruction changes during a conversation as worktrees and stages change.
fn with_brief(message: &str, brief: Option<&str>) -> String {
    match brief {
        Some(brief) => format!("<teryon-instruction>\n{brief}\n</teryon-instruction>\n\n{message}"),
        None => message.to_string(),
    }
}

/// A trimmed string value when non-empty.
fn non_empty(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Whether an `error` field carries an actual failure rather than being absent or empty.
fn has_error(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::String(text) => !text.trim().is_empty(),
        Value::Object(map) => !map.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Number(_) => true,
    }
}

/// The human-readable message inside an `error` field, whether plain text or `{type, message}`.
fn error_text(value: &Value) -> Option<&str> {
    non_empty(value)
        .or_else(|| non_empty(&value["message"]))
        .or_else(|| non_empty(&value["type"]))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lets_expect::lets_expect;
    use serde_json::json;

    use super::*;
    use crate::agents::EntryKind;

    const FIRST: &str = include_str!("../../tests/samples/agents/gemini-first.jsonl");
    const TOOLS: &str = include_str!("../../tests/samples/agents/gemini-tools.jsonl");
    const OTHER_MODEL: &str = include_str!("../../tests/samples/agents/gemini-other-model.jsonl");
    const READ_ONLY: &str = include_str!("../../tests/samples/agents/gemini-read-only.jsonl");

    const KNOWN: &str = "9ec969d6-54b7-4375-9a8c-f907be2e11d0";

    fn driver() -> GeminiDriver {
        GeminiDriver::new(&GeminiSettings::default())
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

    fn on_model(model: Option<&str>, turn: Turn) -> Turn {
        Turn {
            model: model.map(str::to_string),
            ..turn
        }
    }

    fn arguments(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

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

    fn reports(reports: Vec<Value>) -> Vec<EntryDraft> {
        run(&reports.iter().map(Value::to_string).collect::<Vec<_>>())
    }

    fn resumed_reports(reports: Vec<Value>) -> Vec<EntryDraft> {
        let mut driver = driver();
        driver.invocation(&turn("Continue", Some(KNOWN), None, false));
        let mut entries: Vec<EntryDraft> = reports
            .iter()
            .flat_map(|report| {
                driver
                    .interpret(&report.to_string())
                    .expect("the line is understood")
            })
            .collect();
        entries.extend(driver.finish());
        entries
    }

    fn model_after(asked: Option<&str>, lines: &str) -> Option<String> {
        let mut driver = driver();
        driver.invocation(&Turn {
            model: asked.map(str::to_string),
            ..turn("Fix the bug", None, None, false)
        });
        for line in lines.lines() {
            driver.interpret(line).expect("the line is understood");
        }
        driver.model()
    }

    fn conversations_after(lines: &[&str]) -> Vec<Option<String>> {
        let mut driver = driver();
        lines
            .iter()
            .map(|line| {
                driver.interpret(line).expect("the line is understood");
                driver.take_conversation()
            })
            .collect()
    }

    lets_expect! {
        expect(driver().invocation(&on_model(model, turn(message, conversation, brief, read_only)))) {
            let message = "Fix the bug";
            let conversation: Option<&str> = None;
            let brief: Option<&str> = None;
            let read_only = false;
            let model: Option<&str> = None;

            // requirement: the first message starts a new conversation with automatic permissions
            to start_a_conversation {
                have(program.as_str()) equal("agy"),
                have(arguments.clone()) equal(arguments(&[
                    "-p=Fix the bug", "--output-format", "stream-json", "--dangerously-skip-permissions",
                ])),
                have(removed_variables.len()) equal(0)
            }

            // requirement: a chosen model is passed on
            when(model = Some("gemini-4-neon-high")) as pass_the_model to hold {
                have(arguments.clone()) equal(arguments(&[
                    "-p=Fix the bug", "--output-format", "stream-json", "--dangerously-skip-permissions",
                    "--model", "gemini-4-neon-high",
                ]))
            }

            // requirement: later messages resume the conversation and pass the model so it can change between turns
            when(conversation = Some(KNOWN), model = Some("gemini-3.8-flash-medium")) as resume_with_a_new_model to hold {
                have(arguments.clone()) equal(arguments(&[
                    "-p=Fix the bug", "--output-format", "stream-json", "--dangerously-skip-permissions",
                    "--conversation", KNOWN, "--model", "gemini-3.8-flash-medium",
                ]))
            }

            // requirement: a message starting with a dash stays attached to -p
            when(message = "--help") as keep_a_dash_message_attached to hold {
                have(arguments[0].as_str()) equal("-p=--help")
            }

            // requirement: a worktree instruction goes before the message on every turn
            when(brief = Some("Stay in the worktree.")) as pass_the_worktree_instruction to hold {
                have(arguments[0].as_str()) equal("-p=<teryon-instruction>\nStay in the worktree.\n</teryon-instruction>\n\nFix the bug")
            }

            // requirement: a reviewer runs in a read-only sandbox with reads allowed and writes denied
            when(read_only = true) as deny_writes_for_a_reviewer to hold {
                have(arguments.clone()) equal(arguments(&[
                    "-p=Fix the bug", "--output-format", "stream-json",
                    "--sandbox", "--settings", REVIEWER_SETTINGS,
                ]))
            }
        }

        // requirement: extra writable directories (such as a worktree's git history) are added for an author
        expect(driver().invocation(&Turn {
            writable_outside: vec![PathBuf::from("/work/repo/.git")],
            ..turn("Commit", None, None, false)
        }).arguments) as an_author_with_writable_outside {
            to add_the_directory {
                equal(arguments(&[
                    "-p=Commit", "--output-format", "stream-json", "--dangerously-skip-permissions",
                    "--add-dir", "/work/repo/.git",
                ]))
            }
        }

        // requirement: a reviewer never gets extra writable directories
        expect(driver().invocation(&Turn {
            writable_outside: vec![PathBuf::from("/work/repo/.git")],
            ..turn("Review", None, None, true)
        }).arguments) as a_reviewer_with_writable_outside {
            to leave_them_out {
                equal(arguments(&[
                    "-p=Review", "--output-format", "stream-json",
                    "--sandbox", "--settings", REVIEWER_SETTINGS,
                ]))
            }
        }

        // requirement: the summary names the model that ran, as init reports it, else the one asked for
        expect(model_after(asked, lines)) {
            let asked: Option<&str> = None;
            let lines = "";

            when(asked = None) as nothing_is_asked_or_said to be_unknown { equal(None) }
            when(asked = Some("gemini-4-neon-high")) as nothing_is_said_yet to be_the_one_asked_for { equal(Some("gemini-4-neon-high".to_string())) }
            when(lines = FIRST) as init_names_the_model to be_the_one_named { equal(Some("gemini-3.8-flash-low".to_string())) }
            when(asked = Some("gemini-3.8-flash-medium"), lines = OTHER_MODEL) as continuing_on_a_new_model to be_the_new_model { equal(Some("gemini-3.8-flash-medium".to_string())) }
            when(lines = r#"{"event":"init","conversation_id":"abc","init":{"cwd":"/work"}}"#) as default_omits_model to stay_unknown { equal(None) }
        }

        expect(kinds(recording)) {
            when(recording = FIRST) as read_a_plain_answer to hold {
                equal(vec![EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
            when(recording = TOOLS) as read_tool_use_in_order to hold {
                equal(vec![EntryKind::Activity, EntryKind::Activity, EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
            when(recording = READ_ONLY) as read_a_denied_write_in_review to hold {
                equal(vec![EntryKind::Activity, EntryKind::Activity, EntryKind::Notice, EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
            when(recording = OTHER_MODEL) as read_a_resumed_turn to hold {
                equal(vec![EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
        }

        expect(entries(recording)) {
            // requirement: text the agent writes becomes an agent message
            when(recording = FIRST) as give_the_answer to hold {
                have(first().cloned()) equal(Some(EntryDraft::agent_message("ready")))
            }

            // requirement: tokens sum input and cache reads, and output tokens (which already include thinking tokens)
            when(recording = TOOLS) as sum_this_turns_steps_including_cache_reads to hold {
                have(first().cloned()) equal(Some(EntryDraft::activity("write_file", "/work/sample-project/note.txt"))),
                have(get(1).cloned()) equal(Some(EntryDraft::activity("run_command", "wc -c note.txt"))),
                have(get(2).cloned()) equal(Some(EntryDraft::agent_message("done."))),
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(Tokens { read: 21752, written: 82 }), None)))
            }

            // requirement: a resumed turn counts only this turn's step tokens, not the cumulative conversation total
            when(recording = OTHER_MODEL) as count_only_this_turn_when_resumed to hold {
                have(first().cloned()) equal(Some(EntryDraft::agent_message("ready"))),
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(Tokens { read: 11038, written: 1 }), None)))
            }

            // requirement: a denied tool call becomes a notice explaining why
            when(recording = READ_ONLY) as explain_a_denied_write to hold {
                have(get(2).cloned()) equal(Some(EntryDraft::notice(
                    "permission check failed for write_file \"/work/sample-project/forbidden.txt\": Permission denied for write_file(/work/sample-project/forbidden.txt). Matches user-configured deny rule."
                )))
            }
        }

        expect(reports(lines)) {
            when(lines = vec![
                json!({"event": "step_update", "step_update": {"step_index": 1, "state": "ACTIVE", "step_type": "agent_response", "text_delta": "Half "}}),
                json!({"event": "step_update", "step_update": {"step_index": 1, "state": "ACTIVE", "step_type": "agent_response", "text_delta": "a mess"}}),
            ]) as release_buffered_text_when_the_program_ends to hold {
                equal(vec![EntryDraft::agent_message("Half a mess")])
            }
            when(lines = vec![
                json!({"event": "step_update", "step_update": {"step_index": 1, "state": "DONE", "step_type": "agent_response", "text_delta": "   "}}),
                json!({"event": "result", "result": {"status": "SUCCESS", "response": "Fallback reply", "usage": {"input_tokens": 10, "output_tokens": 2, "cache_read_tokens": 5}}}),
            ]) as fall_back_to_result_response_when_no_deltas_carried_text to hold {
                equal(vec![
                    EntryDraft::agent_message("Fallback reply"),
                    EntryDraft::turn_summary(Some(Tokens { read: 15, written: 2 }), None),
                ])
            }
            when(lines = vec![
                json!({"event": "step_update", "step_update": {"step_index": 2, "state": "ERROR", "step_type": "tool"}}),
            ]) as handle_an_unnamed_failed_tool to hold {
                equal(vec![EntryDraft::activity("tool", ""), EntryDraft::notice("A tool call failed.")])
            }
            // requirement: a failed result becomes an error and an early summary without zero tokens
            when(lines = vec![
                json!({"event": "result", "result": {"status": "ERROR", "error": "invalid model selection", "usage": {"input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0}}}),
            ]) as show_a_failed_turn to hold {
                equal(vec![
                    EntryDraft::error("invalid model selection"),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            when(lines = vec![
                json!({"event": "result", "result": {"status": "ERROR", "finished_reason": "timeout"}}),
            ]) as name_finished_reason_when_error_is_missing to hold {
                equal(vec![
                    EntryDraft::error("timeout"),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            when(lines = vec![
                json!({"event": "result", "result": {"status": "ERROR"}}),
            ]) as show_a_failure_without_details to hold {
                equal(vec![
                    EntryDraft::error("The agent reported a failure."),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            // requirement: an error object `{type, message}` surfaces its message, even without status ERROR
            when(lines = vec![
                json!({"event": "result", "result": {"status": "ERROR", "error": {"type": "RESOURCE_EXHAUSTED", "message": "Quota exceeded for Gemini"}}}),
            ]) as read_message_from_an_error_object to hold {
                equal(vec![
                    EntryDraft::error("Quota exceeded for Gemini"),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            when(lines = vec![
                json!({"event": "result", "result": {"error": {"type": "RESOURCE_EXHAUSTED", "message": "Quota exceeded"}}}),
            ]) as treat_an_error_object_without_status_as_failure to hold {
                equal(vec![
                    EntryDraft::error("Quota exceeded"),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            when(lines = vec![
                json!({"event": "result", "result": {"error": {"type": "RATE_LIMIT"}}}),
            ]) as fall_back_to_error_type_when_message_is_missing to hold {
                equal(vec![
                    EntryDraft::error("RATE_LIMIT"),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            // requirement: when headless mode auto-denies an unapproved action and ends with no reply, the turn is marked early
            when(lines = vec![
                json!({"event": "result", "result": {"status": "SUCCESS", "response": "", "denied_actions": [{"action": "command", "display_name": "Bash"}]}}),
            ]) as mark_an_unapproved_stop_early to hold {
                equal(vec![
                    EntryDraft::notice(STOPPED_UNAPPROVED),
                    EntryDraft::turn_summary(None, None).ended_early(),
                ])
            }
            // requirement: a tool whose details arrive only in DONE is shown once with those details, not empty
            when(lines = vec![
                json!({"event": "step_update", "step_update": {"step_index": 3, "state": "ACTIVE", "step_type": "tool", "tool_name": "run_command"}}),
                json!({"event": "step_update", "step_update": {"step_index": 3, "state": "DONE", "step_type": "tool", "tool_name": "run_command", "tool_info": {"name": "run_command", "parameters": {"CommandLine": "git diff"}}}}),
            ]) as wait_for_tool_details_arriving_on_done to hold {
                equal(vec![EntryDraft::activity("run_command", "git diff")])
            }
            // requirement: an unindexed tool updated several times is shown only once when it finishes
            when(lines = vec![
                json!({"event": "step_update", "step_update": {"state": "ACTIVE", "step_type": "tool", "tool_name": "read_file"}}),
                json!({"event": "step_update", "step_update": {"state": "ACTIVE", "step_type": "tool", "tool_name": "read_file", "tool_info": {"parameters": {"Path": "/work/a.rs"}}}}),
                json!({"event": "step_update", "step_update": {"state": "DONE", "step_type": "tool", "tool_name": "read_file", "tool_info": {"parameters": {"Path": "/work/a.rs"}}}}),
            ]) as show_an_unindexed_tool_only_once to hold {
                equal(vec![EntryDraft::activity("read_file", "/work/a.rs")])
            }
        }

        // requirement: on a resumed turn with no per-step token counts, cumulative result.usage is not charged to this turn
        expect(resumed_reports(vec![
            json!({"event": "result", "result": {"status": "SUCCESS", "response": "Done", "usage": {"input_tokens": 50000, "output_tokens": 1200, "cache_read_tokens": 20000}}}),
        ])) as a_resumed_turn_without_step_tokens {
            to report_no_tokens {
                equal(vec![
                    EntryDraft::agent_message("Done"),
                    EntryDraft::turn_summary(None, None),
                ])
            }
        }

        // requirement: the conversation Antigravity names on init is handed over once
        expect(conversations_after(&lines)) {
            let lines = vec![FIRST.lines().next().unwrap_or_default(), r#"{"event":"step_update","step_update":{"step_index":0,"step_type":"user_input","state":"DONE"}}"#];

            to read_it_from_a_recording { equal(vec![Some(KNOWN.to_string()), None]) }
            when(lines = vec![r#"{"event":"init"}"#]) as hand_over_nothing_when_none_is_named to hold { equal(vec![None]) }
        }

        expect(driver().take_commands()) as the_slash_commands { to be_none { equal(None) } }

        expect(driver().interpret(line)) {
            when(line = r#"{"event":"something_new"}"#) as refuse_an_unknown_event to hold { equal(Err(NotUnderstood)) }
            when(line = r#"{"type":"text"}"#) as refuse_json_without_an_event to hold { equal(Err(NotUnderstood)) }
            when(line = "plain text") as refuse_text to hold { equal(Err(NotUnderstood)) }
        }
    }
}
