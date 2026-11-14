//! Starts Grok and reads its report.
//!
//! Grok is run non-interactively with `--single` and prints one JSON object
//! per line. Unlike Claude Code it sends text a few words at a time, so the
//! driver collects the pieces and releases a message when something else
//! follows it. The shapes handled here were recorded from the real program;
//! the recordings live in `tests/samples/agents/`.

use std::mem::take;

use serde_json::Value;

use crate::agents::summary::summarise_input;
use crate::agents::{typed_report, AgentDriver, EntryDraft, Invocation, NotUnderstood, Turn};
use crate::settings::GrokSettings;

pub struct GrokDriver {
    program: String,
    /// Pieces of the message the agent is writing.
    text: String,
}

impl GrokDriver {
    pub fn new(settings: &GrokSettings) -> Self {
        Self {
            program: settings.program.clone(),
            text: String::new(),
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
    fn invocation(&self, turn: &Turn) -> Invocation {
        let conversation_option = if turn.first {
            "--session-id"
        } else {
            "--resume"
        };
        Invocation {
            program: self.program.clone(),
            arguments: vec![
                format!("--single={}", turn.message),
                "--output-format".to_string(),
                "streaming-json".to_string(),
                "--permission-mode".to_string(),
                "auto".to_string(),
                conversation_option.to_string(),
                turn.conversation.to_string(),
            ],
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
            "tool_call_update" => Ok(failed_tool_entries(&report)),
            "end" => {
                let mut entries = self.message();
                entries.extend(end_entries(&report));
                Ok(entries)
            }
            "thought" | "available_commands" => Ok(Vec::new()),
            _ => Err(NotUnderstood),
        }
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
    let reason = report["content"][0]["content"]["text"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("A tool call failed.");
    vec![EntryDraft::notice(reason.trim())]
}

/// The end of the turn, preceded by a notice when it did not end normally.
fn end_entries(report: &Value) -> Vec<EntryDraft> {
    let mut entries = Vec::new();
    match report["stopReason"].as_str() {
        Some("end_turn") | None => {}
        Some(reason) => entries.push(EntryDraft::notice(&format!(
            "The agent stopped early: {reason}."
        ))),
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

    const FIRST: &str = include_str!("../../tests/samples/agents/grok-first.jsonl");
    const TOOLS: &str = include_str!("../../tests/samples/agents/grok-tools.jsonl");

    fn driver() -> GrokDriver {
        GrokDriver::new(&GrokSettings {
            program: "grok".to_string(),
        })
    }

    fn turn(message: &str, first: bool) -> Turn {
        Turn {
            message: message.to_string(),
            conversation: Uuid::nil(),
            first,
        }
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

    lets_expect! {
        expect(driver().invocation(&turn(message, first))) {
            let message = "Fix the bug";
            let first = true;

            // requirement: the first message starts a new conversation with an identifier chosen by Teryon
            to start_a_conversation {
                have(program.as_str()) equal("grok"),
                have(arguments.clone()) equal([
                    "--single=Fix the bug", "--output-format", "streaming-json", "--permission-mode", "auto",
                    "--session-id", "00000000-0000-0000-0000-000000000000",
                ].map(String::from).to_vec()),
                have(removed_variables.len()) equal(0)
            }

            // requirement: later messages resume that conversation
            when(first = false) as resume_the_conversation to hold {
                have(arguments[5].as_str()) equal("--resume"),
                have(arguments[6].as_str()) equal("00000000-0000-0000-0000-000000000000")
            }

            when(message = "--help") as keep_a_message_that_looks_like_an_option_attached to hold {
                have(arguments[0].as_str()) equal("--single=--help")
            }
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

            // requirement: the end of the turn becomes a summary with cost where the program reports it
            when(recording = FIRST) as end_with_a_summary to hold {
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(0.00767584), Some(1))))
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
                equal(vec![EntryDraft::notice("The agent stopped early: max_turns."), EntryDraft::turn_summary(None, None)])
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
}
