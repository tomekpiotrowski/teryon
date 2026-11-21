//! Starts Codex and reads its report.
//!
//! Codex, OpenAI's agent program, is run non-interactively with `codex exec
//! --json` and prints one JSON object per line: the conversation it started,
//! then items (messages, commands, file edits) as they start and complete,
//! then the end of the turn. The shapes handled here were recorded from the
//! real program; the recordings live in `tests/samples/agents/`.
//!
//! Codex runs commands in a sandbox. The author's sandbox lets it write in
//! the project, and sends anything more, such as a commit (which writes in
//! `.git`), to Codex's automatic approver instead of to a person, as nobody
//! sits at its prompt. A reviewer gets a read-only sandbox and no approver,
//! so it cannot change files at all.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::agents::summary::summarise_input;
use crate::agents::{
    typed_report, AgentDriver, EntryDraft, Invocation, NotUnderstood, Tokens, Turn,
};
use crate::settings::CodexSettings;

/// Variables that make Codex bill an API key instead of the subscription.
pub(super) const KEY_VARIABLES: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];

/// The settings for an author: writes in the project, everything else goes to the automatic approver.
const AUTHOR_SANDBOX: [&str; 4] = [
    "sandbox_mode=\"workspace-write\"",
    "sandbox_workspace_write.network_access=true",
    "approval_policy=\"on-request\"",
    "approvals_reviewer=\"auto_review\"",
];

/// The settings for a reviewer: reads only, and asks nobody for more.
const REVIEWER_SANDBOX: [&str; 2] = ["sandbox_mode=\"read-only\"", "approval_policy=\"never\""];

pub struct CodexDriver {
    program: String,
    use_subscription: bool,
    /// The conversation named by the last line, until it is taken.
    conversation: Option<String>,
    /// Items already shown as activity when they started, so their completion is not shown again.
    started: HashSet<String>,
    /// The last problem Codex reported, so the failure of the turn that follows does not repeat it.
    last_problem: Option<String>,
    /// The model asked for. Codex never names the one it runs.
    model: Option<String>,
}

impl CodexDriver {
    pub fn new(settings: &CodexSettings) -> Self {
        Self {
            program: settings.program.clone(),
            use_subscription: settings.use_subscription,
            conversation: None,
            started: HashSet::new(),
            last_problem: None,
            model: None,
        }
    }

    /// An item that started: the tool it is shown as, the first time it is seen.
    fn item_started(&mut self, item: &Value) -> Vec<EntryDraft> {
        let Some(activity) = activity(item) else {
            return Vec::new();
        };
        if let Some(id) = item["id"].as_str() {
            if !self.started.insert(id.to_string()) {
                return Vec::new();
            }
        }
        vec![activity]
    }

    /// An item that completed: a message, or a tool not yet shown, or a failed edit.
    fn item_completed(&mut self, item: &Value) -> Vec<EntryDraft> {
        match item["type"].as_str() {
            Some("agent_message") => {
                let text = item["text"].as_str().unwrap_or_default().trim();
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![EntryDraft::agent_message(text)]
                }
            }
            Some("error") => self.problem(item["message"].as_str()),
            _ => {
                let mut entries = self.item_started(item);
                if item["type"].as_str() == Some("file_change")
                    && item["status"].as_str() == Some("failed")
                {
                    entries.push(EntryDraft::notice("A file edit was not applied."));
                }
                entries
            }
        }
    }

    /// Something Codex reports going wrong that does not end the turn by itself.
    fn problem(&mut self, message: Option<&str>) -> Vec<EntryDraft> {
        let Some(message) = message.map(str::trim).filter(|text| !text.is_empty()) else {
            return Vec::new();
        };
        self.last_problem = Some(message.to_string());
        vec![EntryDraft::notice(message)]
    }

    /// The end of a turn that failed, with the reason unless it was just shown.
    fn failed(&mut self, report: &Value) -> Vec<EntryDraft> {
        let reason = report["error"]["message"]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let mut entries = Vec::new();
        match reason {
            Some(reason) if self.last_problem.as_deref() != Some(reason) => {
                entries.push(EntryDraft::error(reason))
            }
            Some(_) => {}
            None => entries.push(EntryDraft::error("The agent reported a failure.")),
        }
        entries.push(EntryDraft::turn_summary(None, None).ended_early());
        entries
    }
}

impl AgentDriver for CodexDriver {
    /// Builds the command line for one turn.
    ///
    /// The message comes last, after `--`, so a message that begins with a
    /// dash is not read as an option. A new conversation gets its identifier
    /// from Codex, which names it on the first line of its report. Settings
    /// are given on every turn, because a resumed conversation does not keep
    /// them and would fall back to reading only. Teryon's instruction goes in
    /// the message, see [`with_brief`].
    fn invocation(&mut self, turn: &Turn) -> Invocation {
        let mut arguments = vec!["exec".to_string()];
        if turn.conversation.is_some() {
            arguments.push("resume".to_string());
        }
        arguments.extend(["--json", "--skip-git-repo-check"].map(String::from));
        if let Some(model) = &turn.model {
            arguments.extend(["-m".to_string(), model.clone()]);
        }
        self.model = turn.model.clone();
        let sandbox: &[&str] = if turn.read_only {
            &REVIEWER_SANDBOX
        } else {
            &AUTHOR_SANDBOX
        };
        for setting in sandbox {
            arguments.extend(["-c".to_string(), (*setting).to_string()]);
        }
        arguments.push("--".to_string());
        if let Some(conversation) = &turn.conversation {
            arguments.push(conversation.clone());
        }
        arguments.push(with_brief(&turn.message, turn.brief.as_deref()));

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
            "thread.started" => {
                self.conversation = report["thread_id"].as_str().map(str::to_string);
                Ok(Vec::new())
            }
            "turn.started" | "item.updated" => Ok(Vec::new()),
            "item.started" => Ok(self.item_started(&report["item"])),
            "item.completed" => Ok(self.item_completed(&report["item"])),
            "turn.completed" => Ok(vec![EntryDraft::turn_summary(
                Tokens::sum(&report["usage"], &["input_tokens"], &["output_tokens"]),
                None,
            )]),
            "turn.failed" => Ok(self.failed(&report)),
            "error" => Ok(self.problem(report["message"].as_str())),
            _ => Err(NotUnderstood),
        }
    }

    /// Codex does not name its slash commands when run this way.
    fn take_commands(&mut self) -> Option<Vec<String>> {
        None
    }

    fn take_conversation(&mut self) -> Option<String> {
        self.conversation.take()
    }

    /// The model asked for. Codex does not say which model it runs, so on its default this is unknown.
    fn model(&self) -> Option<String> {
        self.model.clone()
    }
}

/// The message, preceded by Teryon's instruction for this turn.
///
/// The instruction changes during a conversation: first it says how to
/// start a worktree, later which worktree is the thread's. Codex takes its
/// own instruction setting only when a conversation starts and ignores a
/// new one on resuming, so a later turn would never hear where to work.
/// The message reaches it every turn.
fn with_brief(message: &str, brief: Option<&str>) -> String {
    match brief {
        Some(brief) => format!("<teryon-instruction>\n{brief}\n</teryon-instruction>\n\n{message}"),
        None => message.to_string(),
    }
}

/// The activity line for an item that uses a tool. Nothing for messages, reasoning and plans.
///
/// Its text goes through the same shortening as the other programs' tool
/// inputs, so a long script shows as one line.
fn activity(item: &Value) -> Option<EntryDraft> {
    match item["type"].as_str()? {
        "command_execution" => {
            let command = shell_command(item["command"].as_str().unwrap_or_default());
            Some(EntryDraft::activity(
                "shell",
                &summarise_input(&json!({ "command": command })),
            ))
        }
        "file_change" => {
            let paths: Vec<&str> = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|change| change["path"].as_str())
                .collect();
            Some(EntryDraft::activity(
                "edit",
                &summarise_input(&json!({ "path": paths.join(", ") })),
            ))
        }
        "mcp_tool_call" => {
            let tool = [item["server"].as_str(), item["tool"].as_str()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(".");
            let tool = if tool.is_empty() { "tool" } else { &tool };
            Some(EntryDraft::activity(
                tool,
                &summarise_input(&item["arguments"]),
            ))
        }
        "web_search" => Some(EntryDraft::activity(
            "web_search",
            &summarise_input(&json!({ "query": item["query"] })),
        )),
        _ => None,
    }
}

/// The command Codex ran, without the shell it ran it in.
///
/// Codex reports `/usr/bin/bash -lc 'git status'`; the thread shows
/// `git status`. A command in another form is shown as it is.
fn shell_command(command: &str) -> String {
    let Some((shell, script)) = command.split_once(" -lc ").or(command.split_once(" -c ")) else {
        return command.to_string();
    };
    if shell.contains(' ') || !(shell.ends_with("sh")) {
        return command.to_string();
    }
    let unquoted = [('\'', '\''), ('"', '"')]
        .into_iter()
        .find_map(|(open, close)| script.strip_prefix(open)?.strip_suffix(close));
    unquoted.unwrap_or(script).to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lets_expect::lets_expect;
    use serde_json::json;

    use super::*;
    use crate::agents::EntryKind;

    const FIRST: &str = include_str!("../../tests/samples/agents/codex-first.jsonl");
    const TOOLS: &str = include_str!("../../tests/samples/agents/codex-tools.jsonl");
    const RESUMED: &str = include_str!("../../tests/samples/agents/codex-resumed.jsonl");
    /// A real reviewer turn, read-only: its refused writes print nothing.
    const REVIEW: &str = include_str!("../../tests/samples/agents/codex-read-only.jsonl");
    /// A real turn continuing a conversation on another model than it started with.
    const OTHER_MODEL: &str = include_str!("../../tests/samples/agents/codex-other-model.jsonl");

    const KNOWN: &str = "01a1f8d6-5c6e-7282-b40e-8cb29b7aa5e1";

    fn driver(use_subscription: bool) -> CodexDriver {
        CodexDriver::new(&CodexSettings {
            program: "codex".to_string(),
            use_subscription,
            ..CodexSettings::default()
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

    fn arguments(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    /// Runs report lines through one driver, then asks for what it still holds.
    fn run(lines: &[String]) -> Vec<EntryDraft> {
        let mut driver = driver(true);
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

    /// The conversation the driver hands over after each of some report lines.
    fn conversations_after(lines: &[&str]) -> Vec<Option<String>> {
        let mut driver = driver(true);
        lines
            .iter()
            .map(|line| {
                driver.interpret(line).expect("the line is understood");
                driver.take_conversation()
            })
            .collect()
    }

    const AUTHOR: [&str; 8] = [
        "-c",
        "sandbox_mode=\"workspace-write\"",
        "-c",
        "sandbox_workspace_write.network_access=true",
        "-c",
        "approval_policy=\"on-request\"",
        "-c",
        "approvals_reviewer=\"auto_review\"",
    ];

    lets_expect! {
        // requirement: the summary names the model asked for, since Codex never says which one ran
        expect(model_after(asked, OTHER_MODEL)) {
            when(asked = None) as the_default_runs to be_unknown { equal(None) }
            when(asked = Some("gpt-6-astra")) as a_model_is_asked_for to be_that_model { equal(Some("gpt-6-astra".to_string())) }
        }

        // requirement: Codex's remark that the conversation started on another model is shown, and the turn still ends normally
        expect(kinds(OTHER_MODEL)) to note_the_switch {
            equal(vec![EntryKind::Notice, EntryKind::AgentMessage, EntryKind::TurnSummary])
        }

        expect(driver(use_subscription).invocation(&on_model(model, turn(message, conversation, brief, read_only)))) {
            let use_subscription = true;
            let message = "Fix the bug";
            let conversation: Option<&str> = None;
            let brief: Option<&str> = None;
            let read_only = false;
            let model: Option<&str> = None;

            // requirement: a chosen model is passed on
            when(model = Some("gpt-6-astra")) as pass_the_model to hold {
                have(arguments.clone()) equal(arguments(&[
                    &["exec", "--json", "--skip-git-repo-check", "-m", "gpt-6-astra"][..], &AUTHOR, &["--", "Fix the bug"],
                ].concat()))
            }

            // requirement: the model is passed when continuing a conversation too, so it can change between messages
            when(model = Some("gpt-6-astra"), conversation = Some(KNOWN)) as pass_the_model_when_continuing to hold {
                have(arguments.clone()) equal(arguments(&[
                    &["exec", "resume", "--json", "--skip-git-repo-check", "-m", "gpt-6-astra"][..], &AUTHOR, &["--", KNOWN, "Fix the bug"],
                ].concat()))
            }

            // requirement: a reviewer runs on its chosen model too
            when(model = Some("gpt-6-astra"), read_only = true) as pass_the_reviewers_model to hold {
                have(arguments[3..5].to_vec()) equal(arguments(&["-m", "gpt-6-astra"]))
            }

            // requirement: the first message starts a new conversation, which Codex names
            to start_a_conversation {
                have(program.as_str()) equal("codex"),
                have(arguments.clone()) equal(arguments(&[
                    &["exec", "--json", "--skip-git-repo-check"][..], &AUTHOR, &["--", "Fix the bug"],
                ].concat()))
            }

            // requirement: later messages resume the conversation Codex named, with the same settings
            when(conversation = Some(KNOWN)) as resume_the_conversation to hold {
                have(arguments.clone()) equal(arguments(&[
                    &["exec", "resume", "--json", "--skip-git-repo-check"][..], &AUTHOR, &["--", KNOWN, "Fix the bug"],
                ].concat()))
            }

            when(message = "--help") as keep_a_message_that_looks_like_an_option_last to hold {
                have(arguments.iter().rev().take(2).cloned().collect::<Vec<_>>()) equal(vec!["--help".to_string(), "--".to_string()])
            }

            // requirement: the worktree instruction goes before the message, on every turn, since Codex ignores a new instruction setting when resuming
            when(brief = Some("Stay in the worktree.")) as pass_the_worktree_instruction to hold {
                have(arguments.clone()) equal(arguments(&[
                    &["exec", "--json", "--skip-git-repo-check"][..], &AUTHOR,
                    &["--", "<teryon-instruction>\nStay in the worktree.\n</teryon-instruction>\n\nFix the bug"],
                ].concat()))
            }
            when(conversation = Some(KNOWN), brief = Some("Stay in the worktree.")) as pass_it_when_resuming_too to hold {
                have(arguments.last().cloned()) equal(Some("<teryon-instruction>\nStay in the worktree.\n</teryon-instruction>\n\nFix the bug".to_string())),
                have(arguments.join(" ").contains("developer_instructions")) be_false
            }

            // requirement: a reviewer's turn runs in a read-only sandbox that nobody can approve more for
            when(read_only = true) as read_only_without_approval to hold {
                have(arguments.clone()) equal(arguments(&[
                    "exec", "--json", "--skip-git-repo-check",
                    "-c", "sandbox_mode=\"read-only\"", "-c", "approval_policy=\"never\"",
                    "--", "Fix the bug",
                ]))
            }

            // requirement: by default the API key variables are not passed to Codex
            to remove_the_key_variables {
                have(removed_variables.clone()) equal(vec!["OPENAI_API_KEY", "CODEX_API_KEY"])
            }

            // requirement: a configuration switch passes the environment unchanged
            when(use_subscription = false) as leave_the_environment_alone to hold {
                have(removed_variables.len()) equal(0)
            }
        }

        expect(kinds(recording)) {
            when(recording = FIRST) as read_tool_use_in_order to hold {
                equal(vec![
                    EntryKind::AgentMessage, EntryKind::Activity, EntryKind::Activity, EntryKind::Activity,
                    EntryKind::Activity, EntryKind::AgentMessage, EntryKind::TurnSummary,
                ])
            }
            when(recording = RESUMED) as read_a_resumed_turn to hold {
                equal(vec![EntryKind::AgentMessage, EntryKind::Activity, EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
            when(recording = REVIEW) as read_a_review to hold {
                equal(vec![EntryKind::AgentMessage, EntryKind::Activity, EntryKind::Activity, EntryKind::AgentMessage, EntryKind::TurnSummary])
            }
        }

        expect(entries(recording)) {
            // requirement: text the agent writes becomes an agent message
            when(recording = FIRST) as give_the_answer to hold {
                have(first().cloned()) equal(Some(EntryDraft::agent_message("I’ll create `note.txt` and check its byte count."))),
                have(get(5).cloned()) equal(Some(EntryDraft::agent_message("done")))
            }

            // requirement: the end of the turn becomes a summary with the tokens read and written
            when(recording = FIRST) as end_with_a_summary to hold {
                have(last().cloned()) equal(Some(EntryDraft::turn_summary(Some(Tokens { read: 86322, written: 229 }), None)))
            }

            // requirement: each command the agent runs becomes an activity line showing the command without its shell
            when(recording = FIRST) as show_the_command to hold {
                have(get(1).cloned()) equal(Some(EntryDraft::activity("shell", "pwd; git rev-parse --show-toplevel 2>/dev/null"))),
                have(get(4).cloned()) equal(Some(EntryDraft::activity("shell", "printf 'hi' > note.txt wc -c note.txt")))
            }

            // requirement: a file edit becomes an activity line naming the files
            when(recording = TOOLS) as name_the_edited_file to hold {
                have(get(4).cloned()) equal(Some(EntryDraft::activity("edit", "/work/sample-project--edit/a.txt")))
            }
        }

        expect(reports(lines)) {
            // requirement: an item is shown once, when it starts, and not again when it completes
            when(lines = vec![
                json!({"type": "item.started", "item": {"id": "item_1", "type": "command_execution", "command": "ls", "status": "in_progress"}}),
                json!({"type": "item.updated", "item": {"id": "item_1", "type": "command_execution", "command": "ls"}}),
                json!({"type": "item.completed", "item": {"id": "item_1", "type": "command_execution", "command": "ls", "status": "completed"}}),
            ]) as show_an_item_once to hold {
                equal(vec![EntryDraft::activity("shell", "ls")])
            }
            when(lines = vec![json!({"type": "item.completed", "item": {"id": "item_1", "type": "web_search", "query": "codex json"}})]) as show_an_item_that_only_completed to hold {
                equal(vec![EntryDraft::activity("web_search", "codex json")])
            }
            when(lines = vec![json!({"type": "item.started", "item": {"id": "item_1", "type": "mcp_tool_call", "server": "github", "tool": "search", "arguments": {"query": "teryon"}}})]) as name_a_tool_of_a_server to hold {
                equal(vec![EntryDraft::activity("github.search", &summarise_input(&json!({"query": "teryon"})))])
            }
            when(lines = vec![json!({"type": "item.started", "item": {"id": "item_1", "type": "mcp_tool_call"}})]) as name_an_unnamed_tool to hold {
                equal(vec![EntryDraft::activity("tool", "")])
            }
            when(lines = vec![json!({"type": "item.completed", "item": {"id": "item_1", "type": "file_change", "changes": [{"path": "a"}, {"path": "b"}], "status": "failed"}})]) as mention_a_failed_edit to hold {
                equal(vec![EntryDraft::activity("edit", "a, b"), EntryDraft::notice("A file edit was not applied.")])
            }
            when(lines = vec![
                json!({"type": "item.completed", "item": {"id": "item_1", "type": "reasoning", "text": "Thinking"}}),
                json!({"type": "item.completed", "item": {"id": "item_2", "type": "todo_list", "items": []}}),
                json!({"type": "item.completed", "item": {"id": "item_3", "type": "agent_message", "text": "  "}}),
            ]) as skip_reasoning_plans_and_blank_text to hold {
                equal(Vec::new())
            }
            // requirement: a problem Codex reports is shown, and the failure of the turn that follows is not repeated
            when(lines = vec![
                json!({"type": "error", "message": "You've hit your usage limit."}),
                json!({"type": "turn.failed", "error": {"message": "You've hit your usage limit."}}),
            ]) as show_a_problem_once to hold {
                equal(vec![EntryDraft::notice("You've hit your usage limit."), EntryDraft::turn_summary(None, None).ended_early()])
            }
            when(lines = vec![json!({"type": "item.completed", "item": {"id": "item_1", "type": "error", "message": "Reconnecting"}})]) as show_a_problem_reported_as_an_item to hold {
                equal(vec![EntryDraft::notice("Reconnecting")])
            }
            // requirement: a failed turn's summary says the turn ended early
            when(lines = vec![json!({"type": "turn.failed", "error": {"message": "stream disconnected"}})]) as show_a_failed_turn to hold {
                equal(vec![EntryDraft::error("stream disconnected"), EntryDraft::turn_summary(None, None).ended_early()])
            }
            when(lines = vec![json!({"type": "turn.failed"})]) as show_a_failure_without_details to hold {
                equal(vec![EntryDraft::error("The agent reported a failure."), EntryDraft::turn_summary(None, None).ended_early()])
            }
            when(lines = vec![json!({"type": "error"})]) as skip_a_problem_without_text to hold {
                equal(Vec::new())
            }
            when(lines = vec![json!({"type": "turn.completed"})]) as end_without_usage to hold {
                equal(vec![EntryDraft::turn_summary(None, None)])
            }
        }

        // requirement: the conversation Codex names at the start of a turn is handed over once, so the next message continues it
        expect(conversations_after(&lines)) {
            let lines = vec![FIRST.lines().next().unwrap_or_default(), r#"{"type":"turn.started"}"#];

            to read_it_from_a_recording { equal(vec![Some(KNOWN.to_string()), None]) }
            when(lines = vec![r#"{"type":"thread.started"}"#]) as hand_over_nothing_when_none_is_named to hold { equal(vec![None]) }
        }

        // requirement: Codex is not asked for slash commands it does not name
        expect(driver(true).take_commands()) as the_slash_commands { to be_none { equal(None) } }

        // requirement: a report line Teryon does not understand is skipped
        expect(driver(true).interpret(line)) {
            when(line = r#"{"type":"something_new"}"#) as refuse_an_unknown_type to hold { equal(Err(NotUnderstood)) }
            when(line = "Reading additional input from stdin...") as refuse_text to hold { equal(Err(NotUnderstood)) }
        }

        expect(shell_command(command)) {
            when(command = "/usr/bin/bash -lc 'git status'") as unwrap_a_login_shell to hold { equal("git status".to_string()) }
            when(command = "/bin/sh -c \"ls -la\"") as unwrap_a_plain_shell to hold { equal("ls -la".to_string()) }
            when(command = "bash -lc ls") as unwrap_an_unquoted_script to hold { equal("ls".to_string()) }
            when(command = "git status") as keep_a_bare_command to hold { equal("git status".to_string()) }
            when(command = "python -c 'print(1)'") as keep_another_program_with_c to hold { equal("python -c 'print(1)'".to_string()) }
            when(command = "") as keep_nothing to hold { equal(String::new()) }
        }
    }
}
