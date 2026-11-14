//! Starts Claude Code and reads its report.
//!
//! Claude Code is run non-interactively with `-p` and prints one JSON object
//! per line. The shapes handled here were recorded from the real program; the
//! recordings live in `tests/samples/agents/`.

use chrono::DateTime;
use serde_json::Value;

use crate::agents::summary::summarise_input;
use crate::agents::{typed_report, AgentDriver, EntryDraft, Invocation, NotUnderstood, Turn};
use crate::settings::ClaudeSettings;

/// Variables that make Claude Code bill an API key instead of the subscription.
const KEY_VARIABLES: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];

pub struct ClaudeDriver {
    program: String,
    use_subscription: bool,
}

impl ClaudeDriver {
    pub fn new(settings: &ClaudeSettings) -> Self {
        Self {
            program: settings.program.clone(),
            use_subscription: settings.use_subscription,
        }
    }
}

impl AgentDriver for ClaudeDriver {
    /// Builds the command line for one turn.
    ///
    /// The message comes last, after `--`, so a message that begins with a
    /// dash is not read as an option.
    fn invocation(&self, turn: &Turn) -> Invocation {
        let conversation_option = if turn.first {
            "--session-id"
        } else {
            "--resume"
        };
        let arguments = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "auto",
            conversation_option,
            &turn.conversation.to_string(),
            "--",
            &turn.message,
        ]
        .map(String::from)
        .to_vec();

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
            "system" => Ok(system_entries(&report)),
            "rate_limit_event" => Ok(limit_entries(&report)),
            "result" => Ok(result_entries(&report)),
            "user" | "stream_event" => Ok(Vec::new()),
            _ => Err(NotUnderstood),
        }
    }
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
    if report["is_error"].as_bool().unwrap_or(false) {
        let text = report["result"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .or(report["subtype"].as_str())
            .unwrap_or("The agent reported a failure.");
        entries.push(EntryDraft::error(text));
    }
    entries.push(EntryDraft::turn_summary(
        report["total_cost_usd"].as_f64(),
        report["num_turns"].as_u64(),
    ));
    entries
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::agents::EntryKind;

    const FIRST: &str = include_str!("../../tests/samples/agents/claude-first.jsonl");
    const TOOLS: &str = include_str!("../../tests/samples/agents/claude-tools.jsonl");
    const DENIED: &str = include_str!("../../tests/samples/agents/claude-denied.jsonl");

    fn driver(use_subscription: bool) -> ClaudeDriver {
        ClaudeDriver::new(&ClaudeSettings {
            program: "claude".to_string(),
            use_subscription,
        })
    }

    fn turn(message: &str, first: bool) -> Turn {
        Turn {
            message: message.to_string(),
            conversation: Uuid::nil(),
            first,
        }
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

    fn interpret(report: Value) -> Result<Vec<EntryDraft>, NotUnderstood> {
        driver(true).interpret(&report.to_string())
    }

    lets_expect! {
        expect(driver(use_subscription).invocation(&turn(message, first))) {
            let use_subscription = true;
            let message = "Fix the bug";
            let first = true;

            // requirement: the first message starts a new conversation with an identifier chosen by Teryon
            to start_a_conversation {
                have(program.as_str()) equal("claude"),
                have(arguments.clone()) equal([
                    "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto",
                    "--session-id", "00000000-0000-0000-0000-000000000000", "--", "Fix the bug",
                ].map(String::from).to_vec())
            }

            // requirement: later messages resume that conversation
            when(first = false) as resume_the_conversation to hold {
                have(arguments[6].as_str()) equal("--resume"),
                have(arguments[7].as_str()) equal("00000000-0000-0000-0000-000000000000")
            }

            when(message = "--help") as keep_a_message_that_looks_like_an_option_last to hold {
                have(arguments[8].as_str()) equal("--"),
                have(arguments[9].as_str()) equal("--help")
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

            // requirement: the end of the turn becomes a summary with cost where the program reports it
            when(recording = FIRST) as end_with_a_summary to hold {
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(0.212946), Some(1))))
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
            when(report = json!({"type": "system", "subtype": "init"})) as skip_the_start_of_the_report to hold {
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
                equal(Ok(vec![EntryDraft::error("Credit balance is too low"), EntryDraft::turn_summary(None, None)]))
            }
            when(report = json!({"type": "result", "is_error": true, "subtype": "error_max_turns"})) as name_the_failure_when_there_is_no_text to hold {
                equal(Ok(vec![EntryDraft::error("error_max_turns"), EntryDraft::turn_summary(None, None)]))
            }
            when(report = json!({"type": "result", "is_error": true})) as show_a_failure_without_details to hold {
                equal(Ok(vec![EntryDraft::error("The agent reported a failure."), EntryDraft::turn_summary(None, None)]))
            }

            // requirement: a report line Teryon does not understand is skipped
            when(report = json!({"type": "something_new"})) as refuse_an_unknown_type to hold {
                equal(Err(NotUnderstood))
            }
        }
    }
}
