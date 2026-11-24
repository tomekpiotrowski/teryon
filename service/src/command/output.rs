//! Plain-text formatting for `teryon` management commands.
//!
//! Models and people read plain text faster and with fewer tokens than raw
//! JSON, while `--json` prints the full records for scripts. Everything here
//! is pure formatting over snapshots and entries so it can be unit tested
//! directly.

use serde_json::from_value;
use uuid::Uuid;

use teryon_core::agents::asking::{Choosing, Question};
use teryon_core::agents::models::Effort;
use teryon_core::agents::EntryKind;
use teryon_core::changes::{Change, Finish, Fit, ReviewState};
use teryon_core::protocol::{ProjectState, ProjectView};
use teryon_core::store::{Entry, RunEnd, Status, Thread};

use crate::command::naming::short_id;

/// Formats the project list: name, identifier, folder and state.
pub fn format_projects(projects: &[ProjectView]) -> String {
    if projects.is_empty() {
        return "No projects.".to_string();
    }
    projects
        .iter()
        .map(|project| {
            format!(
                "{}  {}  {}  {}",
                project.name,
                project.id,
                project.directory.display(),
                project_state(project.state)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Formats a list of threads across one or all projects.
pub fn format_threads(projects: &[ProjectView], threads: &[Thread]) -> String {
    if threads.is_empty() {
        return "No threads.".to_string();
    }
    threads
        .iter()
        .map(|thread| format_thread_row(projects, thread))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Formats one line for a thread in `teryon threads` or timeout reports.
pub fn format_thread_row(projects: &[ProjectView], thread: &Thread) -> String {
    let project = project_name(projects, thread.project_id);
    let title = thread_title(thread);
    let state = thread_state_summary(thread);
    let mut parts = vec![
        short_id(thread.id),
        project.to_string(),
        thread.stage.name().to_string(),
        state,
    ];
    if let Some(change) = change_summary(thread) {
        parts.push(change);
    }
    parts.push(title.to_string());
    parts.join("  ")
}

/// Formats a thread's state, change, plan and last `last` messages.
///
/// `last` counts non-activity entries (`user_message`, `agent_message`,
/// `questions`, `notice`, `error`, `stage`, `plan`). When `activity` is true,
/// tool calls within that window are included as well.
pub fn format_thread_show(
    project: Option<&ProjectView>,
    thread: &Thread,
    entries: &[Entry],
    last: usize,
    activity: bool,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Thread: {}", thread.id));
    if let Some(project) = project {
        lines.push(format!("Project: {} ({})", project.name, project.id));
    }
    lines.push(format!("Title: {}", thread_title(thread)));
    lines.push(format!(
        "Agent: {}",
        format_choice(&thread.agent, thread.model.as_deref(), thread.effort)
    ));
    if let Some(reviewer) = &thread.reviewer {
        let mut line = format!(
            "Reviewer: {}",
            format_choice(&reviewer.agent, reviewer.model.as_deref(), reviewer.effort)
        );
        if let Some(verdict) = reviewer.verdict {
            if verdict.cut {
                line.push_str(" (review stopped early)");
            } else {
                match verdict.findings {
                    Some(0) => line.push_str(" (0 findings)"),
                    Some(1) => line.push_str(" (1 finding)"),
                    Some(count) => line.push_str(&format!(" ({count} findings)")),
                    None => line.push_str(" (reviewed)"),
                }
            }
        }
        lines.push(line);
    }
    let workflow = if thread.workflow.is_empty() {
        "default"
    } else {
        &thread.workflow
    };
    lines.push(format!(
        "Workflow: {workflow} · stage: {}",
        thread.stage.name()
    ));
    let mut state = thread_state_summary(thread);
    if thread.rounds > 0 {
        state.push_str(&format!(" · rounds: {}", thread.rounds));
    }
    lines.push(format!("State: {state}"));
    if let Some(change) = change_summary(thread) {
        lines.push(format!("Change: {change}"));
    }
    if let Some(plan) = &thread.plan {
        lines.push(format!("Plan: {}", plan.title));
    }

    let visible: Vec<&Entry> = entries
        .iter()
        .filter(|entry| is_visible_entry(entry, activity))
        .collect();
    let start = window_start(&visible, last);
    let shown = &visible[start..];

    if !shown.is_empty() {
        lines.push(String::new());
        if start > 0 {
            lines.push(format!("(... {start} earlier entries omitted)"));
        }
        for entry in shown {
            if let Some(formatted) = format_entry(entry) {
                lines.push(formatted);
            }
        }
    }

    lines.join("\n")
}

/// Formats the result of `teryon thread wait` for one or more settled threads.
pub fn format_wait_result(items: &[(Option<&ProjectView>, &Thread, &[Entry])]) -> String {
    items
        .iter()
        .map(|(project, thread, entries)| format_thread_show(*project, thread, entries, 10, false))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// Formats the timeout message of `teryon thread wait` listing threads still working.
pub fn format_wait_timeout(projects: &[ProjectView], working: &[&Thread]) -> String {
    let mut lines = vec!["Timed out waiting. Still working:".to_string()];
    for thread in working {
        lines.push(format_thread_row(projects, thread));
    }
    lines.join("\n")
}

/// Formats `teryon change show`: state, branch, copy's folder and how it fits the project.
pub fn format_change_show(thread: &Thread) -> String {
    let mut lines = vec![format!(
        "Thread: {} ({})",
        thread.id,
        thread_title(thread)
    )];
    if let Some(change) = &thread.change {
        lines.push(format!("Change: {}", change.name));
        let branch = change.branch.as_deref().unwrap_or(&change.name);
        match &change.base {
            Some(base) => lines.push(format!("Branch: {branch} (from {base})")),
            None => lines.push(format!("Branch: {branch}")),
        }
        lines.push(format!("Folder: {}", change.path.display()));
        match &change.state {
            Some(state) => {
                let copy = if state.copy_present {
                    "present"
                } else {
                    "gone"
                };
                let files = match state.files_changed {
                    1 => "1 file changed".to_string(),
                    count => format!("{count} files changed"),
                };
                let mut detail = format!("copy {copy}, {files}");
                if state.unfinished {
                    detail.push_str(", unfinished");
                }
                lines.push(format!("State: {detail}"));
                let fit = if !state.base_present {
                    "base branch gone"
                } else {
                    fit_label(state.fit)
                };
                lines.push(format!("Fit: {fit}"));
            }
            None => lines.push("State: checking".to_string()),
        }
        if let Some(review) = &change.review {
            lines.push(format!(
                "Review: #{} ({}) {}",
                review.number,
                review_state(review.state),
                review.url
            ));
        }
    } else if let Some(finish) = thread.finished {
        lines.push(format!("State: {}", finish_label(finish)));
    } else {
        lines.push("State: no change".to_string());
    }
    lines.join("\n")
}

fn project_name(projects: &[ProjectView], project_id: Uuid) -> &str {
    projects
        .iter()
        .find(|project| project.id == project_id)
        .map_or("unknown", |project| project.name.as_str())
}

fn thread_title(thread: &Thread) -> &str {
    if thread.title.is_empty() {
        "(untitled)"
    } else {
        &thread.title
    }
}

fn project_state(state: ProjectState) -> &'static str {
    match state {
        ProjectState::Ready => "ready",
        ProjectState::Missing => "missing",
        ProjectState::InUse => "in use",
    }
}

fn thread_state_summary(thread: &Thread) -> String {
    if thread.status == Status::Running {
        if thread
            .reviewer
            .as_ref()
            .is_some_and(|reviewer| reviewer.reviewing)
        {
            return "reviewing".to_string();
        }
        return "working".to_string();
    }
    let mut parts = vec!["idle".to_string()];
    if thread.asking {
        parts.push("waiting for answers".to_string());
    }
    if let Some(ended) = thread.ended {
        parts.push(run_end_label(ended).to_string());
    }
    if thread.held {
        parts.push("stopped".to_string());
    }
    if thread.put_away {
        parts.push("put away".to_string());
    }
    parts.join(", ")
}

fn run_end_label(ended: RunEnd) -> &'static str {
    match ended {
        RunEnd::Finished => "finished",
        RunEnd::Failed => "failed",
        RunEnd::Limited => "limited",
        RunEnd::Stopped => "stopped",
    }
}

fn change_summary(thread: &Thread) -> Option<String> {
    if let Some(change) = &thread.change {
        return Some(format_change_brief(change));
    }
    thread
        .finished
        .map(|finish| format!("change: {}", finish_label(finish)))
}

fn format_change_brief(change: &Change) -> String {
    let mut details = Vec::new();
    if let Some(state) = &change.state {
        if !state.copy_present {
            details.push("copy gone".to_string());
        } else if state.unfinished {
            details.push("unfinished".to_string());
        } else {
            let files = match state.files_changed {
                1 => "1 file".to_string(),
                count => format!("{count} files"),
            };
            details.push(files);
            if !state.base_present {
                details.push("base gone".to_string());
            } else {
                details.push(fit_label(state.fit).to_string());
            }
        }
    }
    if let Some(review) = &change.review {
        details.push(review_state(review.state).to_string());
    }
    if details.is_empty() {
        format!("change: {}", change.name)
    } else {
        format!("change: {} ({})", change.name, details.join(", "))
    }
}

fn fit_label(fit: Fit) -> &'static str {
    match fit {
        Fit::Applies => "applies",
        Fit::Overlaps => "overlaps",
        Fit::Unknown => "unknown",
    }
}

fn finish_label(finish: Finish) -> &'static str {
    match finish {
        Finish::Applied => "applied",
        Finish::Accepted => "accepted",
        Finish::Discarded => "discarded",
        Finish::Removed => "removed",
    }
}

fn review_state(state: ReviewState) -> &'static str {
    match state {
        ReviewState::InReview => "in review",
        ReviewState::Accepted => "accepted",
        ReviewState::Declined => "declined",
    }
}

fn format_choice(agent: &str, model: Option<&str>, effort: Option<Effort>) -> String {
    let mut parts = vec![agent.to_string()];
    if let Some(model) = model {
        parts.push(model.to_string());
    }
    if let Some(effort) = effort {
        parts.push(effort.name().to_string());
    }
    parts.join(" · ")
}

fn is_visible_entry(entry: &Entry, activity: bool) -> bool {
    if entry.kind == EntryKind::TurnSummary.name() {
        return false;
    }
    if entry.kind == EntryKind::Activity.name() {
        return activity;
    }
    true
}

fn window_start(entries: &[&Entry], last: usize) -> usize {
    if last == 0 {
        return entries.len();
    }
    let mut remaining = last;
    for (index, entry) in entries.iter().enumerate().rev() {
        if entry.kind != EntryKind::Activity.name() {
            remaining -= 1;
            if remaining == 0 {
                return index;
            }
        }
    }
    0
}

fn format_entry(entry: &Entry) -> Option<String> {
    let kind = entry.kind.as_str();
    let reviewer = entry.body["reviewer"] == true;
    match kind {
        k if k == EntryKind::UserMessage.name() => {
            let text = entry.body["text"].as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            let mut label = if let Some(from) = entry.body.get("from").and_then(|v| v.as_object()) {
                let title = from
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if title.is_empty() {
                    "From another thread".to_string()
                } else {
                    format!("From «{title}»")
                }
            } else if entry.body["automatic"] == true {
                "Teryon".to_string()
            } else {
                "Person".to_string()
            };
            if reviewer {
                label.push_str(" (to reviewer)");
            }
            Some(format!("{label}: {text}"))
        }
        k if k == EntryKind::AgentMessage.name() => {
            let text = entry.body["text"].as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            let label = if reviewer {
                match entry.body["agent"].as_str() {
                    Some(agent) => format!("Reviewer ({agent})"),
                    None => "Reviewer".to_string(),
                }
            } else {
                "Agent".to_string()
            };
            Some(format!("{label}: {text}"))
        }
        k if k == EntryKind::Questions.name() => format_questions(entry),
        k if k == EntryKind::Activity.name() => {
            let tool = entry.body["tool"].as_str().unwrap_or_default();
            let summary = entry.body["summary"].as_str().unwrap_or_default();
            let joined = [tool, summary]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("  ");
            (!joined.is_empty()).then(|| format!("[tool] {joined}"))
        }
        k if k == EntryKind::Notice.name() => {
            let text = entry.body["text"].as_str()?.trim();
            (!text.is_empty()).then(|| format!("[notice] {text}"))
        }
        k if k == EntryKind::Stage.name() => {
            let text = entry.body["text"].as_str()?.trim();
            (!text.is_empty()).then(|| format!("[stage] {text}"))
        }
        k if k == EntryKind::Plan.name() => {
            let text = entry.body["text"].as_str()?.trim();
            (!text.is_empty()).then(|| format!("[plan] {text}"))
        }
        k if k == EntryKind::Error.name() => {
            let text = entry.body["text"].as_str()?.trim();
            (!text.is_empty()).then(|| format!("[error] {text}"))
        }
        _ => None,
    }
}

fn format_questions(entry: &Entry) -> Option<String> {
    let questions: Vec<Question> = from_value(entry.body["questions"].clone()).ok()?;
    let lines: Vec<String> = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let answers: Vec<String> = question
                .answers
                .iter()
                .enumerate()
                .map(|(answer, choice)| {
                    let recommended = match &question.choosing {
                        Choosing::One { recommended } => *recommended == answer,
                        Choosing::Several { recommended } => recommended.contains(&answer),
                    };
                    if recommended {
                        format!("{} (recommended)", choice.label)
                    } else {
                        choice.label.clone()
                    }
                })
                .collect();
            let several = match question.choosing {
                Choosing::One { .. } => "",
                Choosing::Several { .. } => ", any of",
            };
            format!(
                "  {}. {}{several}: {}",
                index + 1,
                question.question,
                answers.join(" / ")
            )
        })
        .collect();
    (!lines.is_empty()).then(|| format!("[questions]\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::Utc;
    use lets_expect::lets_expect;
    use serde_json::{json, Value};
    use uuid::Uuid;

    use teryon_core::changes::ChangeState;
    use teryon_core::plans::PlanInfo;
    use teryon_core::workflow::Stage;

    use super::*;

    fn id(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    const P1: &str = "11111111-0000-4000-8000-000000000001";
    const T1: &str = "abcd1234-0000-4000-8000-000000000001";

    fn sample_project() -> ProjectView {
        ProjectView {
            id: id(P1),
            name: "teryon".to_string(),
            directory: PathBuf::from("/work/teryon"),
            created_at: Utc::now(),
            state: ProjectState::Ready,
            on_github: false,
        }
    }

    fn sample_thread() -> Thread {
        Thread {
            id: id(T1),
            project_id: id(P1),
            title: "Fix login".to_string(),
            named: true,
            agent: "claude".to_string(),
            model: Some("opus".to_string()),
            effort: None,
            conversation: None,
            handover: false,
            status: Status::Idle,
            unread: false,
            asking: false,
            commands: Vec::new(),
            change: Some(Box::new(Change {
                name: "fix-login".to_string(),
                path: PathBuf::from("/work/teryon/.teryon/worktrees/fix-login"),
                base: Some("main".to_string()),
                start: "abc".to_string(),
                branch: Some("fix-login".to_string()),
                state: Some(ChangeState {
                    copy_present: true,
                    base_present: true,
                    files_changed: 2,
                    fit: Fit::Applies,
                    unfinished: false,
                }),
                review: None,
            })),
            finished: None,
            reviewer: None,
            workflow: "default".to_string(),
            stage: Stage::Implementing,
            stage_since: None,
            plan: Some(PlanInfo {
                title: "Login plan".to_string(),
                fingerprint: "abc".to_string(),
                too_large: false,
            }),
            rounds: 0,
            held: false,
            put_away: false,
            done_seen: false,
            picks: BTreeMap::new(),
            ended: Some(RunEnd::Finished),
            preview: None,
            created_at: Utc::now(),
        }
    }

    fn entry(position: u64, kind: &str, body: Value) -> Entry {
        Entry {
            id: Uuid::new_v4(),
            thread_id: id(T1),
            position,
            kind: kind.to_string(),
            body,
            created_at: Utc::now(),
        }
    }

    lets_expect! {
        expect(format_threads(&[sample_project()], &threads)) as formatted_threads {
            let threads = vec![sample_thread()];

            to show_id_project_stage_state_change_and_title {
                equal("abcd1234  teryon  implementing  idle, finished  change: fix-login (2 files, applies)  Fix login".to_string())
            }
            when(threads = Vec::new()) as empty to say_so { equal("No threads.".to_string()) }
            when(threads = vec![Thread { status: Status::Running, ended: None, ..sample_thread() }]) as working to say_working {
                have(contains("  working  ")) be_true
            }
            when(threads = vec![Thread { asking: true, ..sample_thread() }]) as asking to say_waiting_for_answers {
                have(contains("waiting for answers")) be_true
            }
            when(threads = vec![Thread { ended: Some(RunEnd::Failed), ..sample_thread() }]) as failed to say_failed {
                have(contains("idle, failed")) be_true
            }
        }

        expect(format_thread_show(Some(&sample_project()), &sample_thread(), &entries, last, activity)) as formatted_show {
            let last = 10usize;
            let activity = false;
            let entries = vec![
                entry(1, "user_message", json!({ "text": "Hello", "from": { "thread_id": P1, "project_id": P1, "title": "Coordinator" } })),
                entry(2, "activity", json!({ "tool": "Bash", "summary": "cargo test" })),
                entry(3, "agent_message", json!({ "text": "All done." })),
                entry(4, "turn_summary", json!({ "tokens": null })),
            ];

            to include_header_and_attribution_without_activity {
                have(contains("From «Coordinator»: Hello")) be_true
            }
            to include_agent_reply { have(contains("Agent: All done.")) be_true }
            to include_plan { have(contains("Plan: Login plan")) be_true }
            to leave_out_activity_by_default { have(contains("[tool]")) be_false }
            when(activity = true) as with_activity to include_tool_calls {
                have(contains("[tool] Bash  cargo test")) be_true
            }
            when(last = 1) as with_last_one {
                to keep_only_the_newest_message { have(contains("Agent: All done.")) be_true }
                to omit_the_earlier_message { have(contains("From «Coordinator»: Hello")) be_false }
                to note_omitted_entries { have(contains("earlier entries omitted")) be_true }
            }
        }

        expect(format_wait_result(&[(Some(&sample_project()), &sample_thread(), &entries)])) as formatted_wait {
            let entries = vec![entry(1, "agent_message", json!({ "text": "Final reply." }))];
            to end_with_the_last_agent_message { have(ends_with("Agent: Final reply.")) be_true }
        }

        expect(format_wait_timeout(&[sample_project()], &[&Thread { status: Status::Running, ended: None, ..sample_thread() }])) as formatted_timeout {
            to list_still_working_threads { have(contains("Still working:\nabcd1234")) be_true }
        }

        expect(format_change_show(&sample_thread())) as formatted_change {
            to include_branch_folder_and_fit {
                have(contains("Branch: fix-login (from main)\nFolder: /work/teryon/.teryon/worktrees/fix-login\nState: copy present, 2 files changed\nFit: applies")) be_true
            }
        }
    }
}
