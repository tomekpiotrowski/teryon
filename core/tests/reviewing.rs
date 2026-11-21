//! Specs for a second agent reviewing a thread's change, in the same thread.
//!
//! A review that edits files or lands in the wrong conversation would
//! quietly undo its point, so these run the real workspace with the fake
//! agents and check what each agent was started with.

mod common;

use std::env::temp_dir;
use std::fs::{read_to_string, remove_file, write};
use std::path::{Path, PathBuf};

use lets_expect::lets_expect;
use serde_json::Value;
use tempfile::TempDir;

use common::{
    commit, fake_agents, git, invocations, noted_turn, project_folder, put, thread_file, Scene,
};
use teryon_core::agents::Voice;
use teryon_core::protocol::ProjectView;
use teryon_core::settings::{AgentSettings, GrokSettings};
use teryon_core::store::{Entry, Thread, Verdict};

/// The reviewer most of these specs ask for: Grok, or Claude when Grok wrote the change.
fn usual_reviewer(thread: &Thread) -> &'static str {
    if thread.agent == "grok" {
        "claude"
    } else {
        "grok"
    }
}

/// A project in a repository on `main` with a `README.md`.
struct Project {
    scene: Scene,
    folder: TempDir,
    project: ProjectView,
}

fn project() -> Project {
    project_with(fake_agents())
}

fn project_with(settings: AgentSettings) -> Project {
    let scene = Scene::with(settings);
    let folder = project_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), "README.md", "hello\n");
    commit(folder.path());
    let project = scene.project(&folder);
    Project {
        scene,
        folder,
        project,
    }
}

impl Project {
    /// A thread of `agent` whose change `fix-login` has one changed file, its agent stopped.
    async fn changed(&self, agent: &str) -> (Thread, PathBuf) {
        let thread = self.scene.thread(&self.project, agent);
        let _ = remove_file(self.folder.path().join(".fake-agent-turn"));
        self.scene
            .say(&thread, "remember the turn and wait")
            .unwrap();
        let turn = noted_turn(&self.folder).await;
        let path = self
            .scene
            .workspace
            .start_change(thread.id, turn, "fix-login", None)
            .await
            .expect("the change starts");
        self.scene.workspace.stop(thread.id).unwrap();
        self.scene.wait_until_idle(&thread).await;
        put(&path, "login.txt", "fixed\n");
        self.scene.workspace.refresh_change(thread.id).await;
        (thread, path)
    }

    /// Asks for a review by Grok, or by Claude for Grok's own work, and waits for it.
    async fn review(&self, thread: &Thread) -> Result<(), String> {
        self.review_by(thread, usual_reviewer(thread)).await
    }

    /// Asks a given agent to review, and waits for it.
    async fn review_by(&self, thread: &Thread, agent: &str) -> Result<(), String> {
        self.review_on(thread, agent, None).await
    }

    /// Asks a given agent to review on a given model, and waits for it.
    async fn review_on(
        &self,
        thread: &Thread,
        agent: &str,
        model: Option<&str>,
    ) -> Result<(), String> {
        self.scene
            .workspace
            .review_change(thread.id, agent, model)
            .map_err(|problem| problem.to_string())?;
        self.scene.wait_until_idle(thread).await;
        Ok(())
    }

    fn verdict(&self, thread: &Thread) -> Option<Verdict> {
        self.scene
            .thread_now(thread)
            .reviewer
            .and_then(|reviewer| reviewer.verdict)
    }

    /// The entries from `position` on, each as its kind, whether the reviewer's, and its text.
    fn lines_from(&self, thread: &Thread, position: usize) -> Vec<(String, bool, String)> {
        self.scene
            .entries(thread)
            .into_iter()
            .skip(position - 1)
            .map(line)
            .collect()
    }

    fn invocations(&self) -> Vec<String> {
        invocations(&self.folder)
    }
}

/// Puts a link to `/dev/zero`, which never ends, at `path`. Windows has no such file, so nothing happens there.
#[cfg(unix)]
fn link_to_an_endless_file(path: &Path) {
    std::os::unix::fs::symlink("/dev/zero", path).unwrap();
}

#[cfg(not(unix))]
fn link_to_an_endless_file(_path: &Path) {}

fn line(entry: Entry) -> (String, bool, String) {
    let reviewer = entry.body["reviewer"] == Value::Bool(true);
    let text = entry.body["text"].as_str().unwrap_or_default().to_string();
    (entry.kind, reviewer, text)
}

fn own(kind: &str, text: &str) -> (String, bool, String) {
    (kind.to_string(), false, text.to_string())
}

fn reviewers(kind: &str, text: &str) -> (String, bool, String) {
    (kind.to_string(), true, text.to_string())
}

const FIRST_REVIEW: &str =
    "1. `fix-login.txt` has no test.\n2. The redirect is not checked.\n\nFindings: 2";

lets_expect! { #tokio_test
    // requirement: a review is done by the other agent, in the same thread, with its lines marked as the reviewer's
    // requirement: the reviewer is told where the change is and to leave files alone, and counts what it found
    story claudes_change_is_reviewed_by_grok {
        let project = project();
        let (thread, path) = project.changed("claude").await;
        let before = project.scene.entries(&thread).len();

        expect(project.review(&thread).await) to equal(Ok(()))

        let lines = project.lines_from(&thread, before + 1);
        expect(lines[0].clone()) to equal(reviewers("notice", "Grok reviews the change."))
        expect(project.scene.entries(&thread)[before].body["starts_review"].clone()) to equal(Value::Bool(true))
        expect(lines.contains(&reviewers("agent_message", FIRST_REVIEW))) to be_true
        expect(lines.iter().all(|(_, reviewer, _)| *reviewer)) to be_true
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        let conversation = reviewer.conversation.clone().expect("the reviewer's conversation is kept");
        expect((reviewer.agent.as_str(), reviewer.reviewing)) to equal(("grok", false))
        expect(reviewer.verdict) to equal(Some(Verdict { findings: Some(2), answered: false, cut: false }))
        expect(Some(&conversation) == project.scene.thread_now(&thread).conversation.as_ref()) to be_false

        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.contains("--single=Review the change.")) to be_true
        expect(started.contains(&format!("git worktree `{}`", path.display()))) to be_true
        expect(started.contains("Do not create, change or delete any file")) to be_true
        expect(started.contains(&format!("--session-id {conversation}"))) to be_true
    }

    // requirement: a Claude reviewer has its editing tools switched off
    story groks_change_is_reviewed_by_claude_without_editing_tools {
        let project = project();
        let (thread, _) = project.changed("grok").await;
        project.review(&thread).await.unwrap();

        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.contains("--disallowedTools Edit,Write,NotebookEdit")) to be_true
        expect(started.contains("Review the change.")) to be_true
        expect(project.scene.entries(&thread).iter().any(|entry| entry.body["text"] == "Claude reviews the change.")) to be_true
    }

    // requirement: a message to the author leaves the review answered, and the next review continues the reviewer's conversation
    story a_change_is_reviewed_again_after_the_author_worked_on_it {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();

        project.scene.say(&thread, "fix what the review found").unwrap();
        project.scene.wait_until_idle(&thread).await;
        expect(project.verdict(&thread)) to equal(Some(Verdict { findings: Some(2), answered: true, cut: false }))
        expect(project.invocations().last().cloned().unwrap_or_default().contains("--session-id")) to be_false

        project.review(&thread).await.unwrap();
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.contains("Review it again")) to be_true
        expect(started.contains(&format!("--resume {}", reviewer.conversation.expect("the reviewer's conversation")))) to be_true
        expect(reviewer.verdict) to equal(Some(Verdict { findings: Some(0), answered: false, cut: false }))
    }

    // requirement: a review runs on the model asked for, which the reviewer keeps
    story a_review_runs_on_its_chosen_model {
        let project = project();
        let (thread, _) = project.changed("claude").await;

        project.review_on(&thread, "grok", Some("grok-4.6")).await.unwrap();
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        expect(reviewer.model.clone()) to equal(Some("grok-4.6".to_string()))
        expect(project.invocations().last().cloned().unwrap_or_default().contains("-m grok-4.6")) to be_true
        let summary = project.scene.entries(&thread).into_iter().rev().find(|entry| entry.kind == "turn_summary").unwrap();
        expect(summary.body["reviewer"].clone()) to equal(Value::Bool(true))
        expect(summary.body["model"].clone()) to equal(Value::String("grok-4.7-build".to_string()))
    }

    // requirement: the same reviewer asked again on another model continues its conversation on that model
    story a_review_again_on_another_model_continues {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review_on(&thread, "grok", Some("grok-4.6")).await.unwrap();
        project.scene.say(&thread, "fix what the review found").unwrap();
        project.scene.wait_until_idle(&thread).await;

        project.review_on(&thread, "grok", None).await.unwrap();
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        expect(reviewer.model.clone()) to equal(None)
        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.contains("--resume")) to be_true
        expect(started.contains(" -m ")) to be_false
    }

    // requirement: the reviewer's model can change between messages, leaving the author's alone
    story the_reviewers_model_changes {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();

        project.scene.workspace.change_agent(thread.id, Voice::Reviewer, "grok", Some("grok-4.5")).unwrap();
        project.scene.ask_reviewer(&thread, "is the second one real?").unwrap();
        project.scene.wait_until_idle(&thread).await;

        let now = project.scene.thread_now(&thread);
        expect(now.reviewer.and_then(|reviewer| reviewer.model)) to equal(Some("grok-4.5".to_string()))
        expect(now.model) to equal(None)
        expect(project.invocations().last().cloned().unwrap_or_default().contains("-m grok-4.5")) to be_true
    }

    // requirement: the reviewer can move to another agent, which starts its own conversation, told what the reviewer said
    story the_reviewer_moves_to_another_agent {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();

        let changed = project.scene.workspace.change_agent(thread.id, Voice::Reviewer, "codex", None).unwrap();
        let reviewer = changed.reviewer.clone().expect("a reviewer");
        expect(reviewer.agent.clone()) to equal("codex".to_string())
        expect(reviewer.conversation.clone()) to equal(None)
        expect(reviewer.verdict.is_some()) to be_true
        let notice = project.scene.entries(&thread).last().cloned().unwrap();
        expect(notice.body["text"].as_str().map(str::to_string)) to equal(Some("Codex takes over from Grok.".to_string()))
        expect(notice.body["agent"].as_str().map(str::to_string)) to equal(Some("codex".to_string()))

        project.scene.ask_reviewer(&thread, "is the second one real?").unwrap();
        project.scene.wait_until_idle(&thread).await;
        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.starts_with("exec --json")) to be_true
        expect(started.contains("You are taking over this thread")) to be_true
        expect(started.contains("has no test")) to be_true
        expect(started.contains("remember the turn and wait")) to be_false
        expect(project.scene.thread_now(&thread).reviewer.map(|reviewer| reviewer.handover)) to equal(Some(false))
    }

    // requirement: the author and the reviewer stay different agents when either moves
    story the_author_and_reviewer_stay_apart {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();

        let author = project.scene.workspace.change_agent(thread.id, Voice::Author, "grok", None).map_err(|problem| problem.to_string());
        expect(author.err()) to equal(Some("Grok reviews this thread, so it cannot also write it. Choose another reviewer first.".to_string()))
        let reviewer = project.scene.workspace.change_agent(thread.id, Voice::Reviewer, "claude", None).map_err(|problem| problem.to_string());
        expect(reviewer.err()) to equal(Some("Claude wrote this change, so another agent has to review it.".to_string()))
        let now = project.scene.thread_now(&thread);
        expect(now.agent.clone()) to equal("claude".to_string())
        expect(now.reviewer.map(|reviewer| reviewer.agent)) to equal(Some("grok".to_string()))
    }

    // requirement: the person can ask the reviewer something, which leaves the review's count alone
    story the_reviewer_answers_the_person {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        let before = project.scene.entries(&thread).len();

        project.scene.ask_reviewer(&thread, "is the second one real?").unwrap();
        project.scene.wait_until_idle(&thread).await;

        let lines = project.lines_from(&thread, before + 1);
        expect(lines[0].clone()) to equal(reviewers("user_message", "is the second one real?"))
        expect(lines.contains(&reviewers("agent_message", "ready"))) to be_true
        expect(project.verdict(&thread)) to equal(Some(Verdict { findings: Some(2), answered: false, cut: false }))
        expect(project.invocations().last().cloned().unwrap_or_default().contains("--single=is the second one real?")) to be_true
    }

    // requirement: the author's lines stay unmarked
    story the_authors_lines_are_its_own {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        let before = project.scene.entries(&thread).len();
        project.scene.say(&thread, "hello").unwrap();
        project.scene.wait_until_idle(&thread).await;
        expect(project.lines_from(&thread, before + 1)[0].clone()) to equal(own("user_message", "hello"))
        expect(project.scene.thread_now(&thread).reviewer) to equal(None)
    }

    // requirement: a reviewer that edits files is caught, and the thread says so, even in a new file the author has not committed
    story a_reviewer_that_edits_files_is_caught {
        let project = project();
        let (thread, path) = project.changed("claude").await;
        write(project.folder.path().join(".fake-reviewer-edits"), "").unwrap();
        project.review(&thread).await.unwrap();

        expect(read_to_string(path.join("login.txt")).unwrap_or_default()) to equal("fixed\nedited by the reviewer\n".to_string())
        expect(project.lines_from(&thread, 1).contains(&reviewers("error", "The reviewer changed files, though it was told not to. Look at the changes before applying them."))) to be_true
    }

    // requirement: a review is refused without changes, with nothing changed, while an agent works, and the reviewer cannot be asked before it exists
    story reviewing_is_refused {
        let project = project();
        let plain = project.scene.thread(&project.project, "claude");
        expect(project.review(&plain).await) to equal(Err("This thread has no changes.".to_string()))
        expect(project.scene.ask_reviewer(&plain, "hello").map(|_| ())) to equal(Err("Nobody reviews this thread yet. Ask for a review first.".to_string()))

        let (thread, path) = project.changed("claude").await;
        remove_file(path.join("login.txt")).unwrap();
        project.scene.workspace.refresh_change(thread.id).await;
        expect(project.review(&thread).await) to equal(Err("No files have changed, so there is nothing to review.".to_string()))

        put(&path, "login.txt", "fixed\n");
        project.scene.workspace.refresh_change(thread.id).await;
        project.scene.say(&thread, "be slow").unwrap();
        project.scene.wait_until_reporting(&thread).await;
        expect(project.review(&thread).await) to equal(Err("The agent is still working on the previous message.".to_string()))
        project.scene.workspace.stop(thread.id).unwrap();
        project.scene.wait_until_idle(&thread).await;
        expect(project.scene.thread_now(&thread).reviewer) to equal(None)
    }

    // requirement: the reviewer is another agent than the author, and one Teryon knows
    story a_reviewer_must_be_another_known_agent {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        expect(project.review_by(&thread, "claude").await) to equal(Err("Claude wrote this change, so another agent has to review it.".to_string()))
        expect(project.review_by(&thread, "gemini").await) to equal(Err("Choose Claude, Codex or Grok as the reviewer.".to_string()))
        expect(project.scene.thread_now(&thread).reviewer) to equal(None)
    }

    // requirement: a Codex reviewer runs read-only, in a sandbox nobody can approve more for
    // requirement: asking a different agent to review starts a review of its own, not a continuation of the last one
    story a_different_reviewer_starts_afresh {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        project.scene.say(&thread, "fix what the review found").unwrap();
        project.scene.wait_until_idle(&thread).await;

        project.review_by(&thread, "codex").await.unwrap();
        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.starts_with("exec --json")) to be_true
        expect(started.contains("-c sandbox_mode=\"read-only\" -c approval_policy=\"never\"")) to be_true
        expect(started.contains("Review the change.")) to be_true
        expect(started.contains("Review it again")) to be_false
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        expect(reviewer.agent.as_str()) to equal("codex")
        expect(reviewer.conversation) to equal(Some("01a1f8ed-247d-7683-bcf5-27a3f5553707".to_string()))
        expect(reviewer.verdict.map(|verdict| verdict.answered)) to equal(Some(false))
        expect(project.scene.entries(&thread).iter().any(|entry| entry.body["text"] == "Codex reviews the change.")) to be_true

        // requirement: each reviewer entry keeps the agent that wrote it, so a later reviewer does not take over earlier reviews
        let named: Vec<(String, String)> = project
            .scene
            .entries(&thread)
            .iter()
            .filter(|entry| entry.body["reviewer"] == Value::Bool(true))
            .map(|entry| (entry.kind.clone(), entry.body["agent"].as_str().unwrap_or("nobody").to_string()))
            .collect();
        let codex_from = named.iter().position(|(_, agent)| agent == "codex").unwrap_or(named.len());
        expect(codex_from > 0) to be_true
        expect(named[..codex_from].iter().all(|(_, agent)| agent == "grok")) to be_true
        expect(named[codex_from..].iter().all(|(_, agent)| agent == "codex")) to be_true
        expect(named.iter().any(|(kind, agent)| kind == "agent_message" && agent == "grok")) to be_true
    }

    // requirement: a question to the reviewer names the agent it went to
    story a_question_to_the_reviewer_names_it {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        let question = project.scene.ask_reviewer(&thread, "is the second one real?").unwrap();
        project.scene.wait_until_idle(&thread).await;
        expect(question.body["agent"].clone()) to equal(Value::String("grok".to_string()))
    }

    // requirement: a review cut short is recorded as that, so it can be asked for again
    story a_review_stopped_by_the_person_is_cut {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        project.scene.say(&thread, "fix it").unwrap();
        project.scene.wait_until_idle(&thread).await;

        write(project.folder.path().join(".fake-reviewer-slow"), "").unwrap();
        project.scene.workspace.review_change(thread.id, "grok", None).unwrap();
        project.scene.wait_until_reporting(&thread).await;
        project.scene.workspace.stop(thread.id).unwrap();
        project.scene.wait_until_idle(&thread).await;
        expect(project.verdict(&thread)) to equal(Some(Verdict { findings: None, answered: false, cut: true }))

        // A review after one cut short is a plain review: the author has fixed nothing since.
        remove_file(project.folder.path().join(".fake-reviewer-slow")).unwrap();
        project.review(&thread).await.unwrap();
        let started = project.invocations().last().cloned().unwrap_or_default();
        expect(started.contains("Review the change.")) to be_true
        expect(started.contains("Review it again")) to be_false
    }

    // requirement: a review asked for again before the author worked is a plain review
    story a_review_repeated_without_work_is_plain {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        project.review(&thread).await.unwrap();
        expect(project.invocations().last().cloned().unwrap_or_default().contains("Review it again")) to be_false
    }

    // requirement: a reviewer that cannot be started leaves the review recorded as cut
    story a_reviewer_that_cannot_start_cuts_the_review {
        let project = project_with(AgentSettings {
            grok: GrokSettings { program: "a-program-nobody-has".to_string(), home: Some(temp_dir().join("teryon-core-tests-grok")), ..GrokSettings::default() },
            ..fake_agents()
        });
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        expect((reviewer.reviewing, reviewer.verdict)) to equal((false, Some(Verdict { findings: None, answered: false, cut: true })))
    }

    // requirement: a review interrupted by a restart is recorded as cut
    story a_review_interrupted_by_a_restart_is_cut {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        let file = thread_file(&project.folder, &thread);
        let mut stored: Value = serde_json::from_str(&read_to_string(&file).unwrap()).unwrap();
        stored["status"] = Value::from("running");
        stored["reviewer"]["reviewing"] = Value::Bool(true);
        stored["reviewer"]["verdict"] = Value::Null;
        write(&file, stored.to_string()).unwrap();

        let Project { scene, folder, project: view } = project;
        let project = Project { scene: scene.restart(|_| {}), folder, project: view };
        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        expect((reviewer.reviewing, reviewer.verdict)) to equal((false, Some(Verdict { findings: None, answered: false, cut: true })))
    }

    // requirement: the check for a reviewer's edits never reads files, so a link to an endless file does not hold the review up
    story a_link_to_an_endless_file_does_not_hold_a_review_up {
        let project = project();
        let (thread, path) = project.changed("claude").await;
        link_to_an_endless_file(&path.join("endless"));
        expect(project.review(&thread).await) to equal(Ok(()))
        expect(project.verdict(&thread)) to equal(Some(Verdict { findings: Some(2), answered: false, cut: false }))
    }

    // requirement: a finished change takes its reviewer with it, so nobody can be asked to review outside a change
    story a_finished_change_takes_its_reviewer {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        project.scene.workspace.apply_change(thread.id, "Fix the login").await.unwrap();

        expect(project.scene.thread_now(&thread).reviewer) to equal(None)
        expect(project.scene.ask_reviewer(&thread, "anything else?").map(|_| ())) to equal(Err("Nobody reviews this thread yet. Ask for a review first.".to_string()))
    }

    // requirement: a new change starts without the last change's reviewer
    story a_new_change_gets_a_new_reviewer {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        project.scene.workspace.apply_change(thread.id, "Fix the login").await.unwrap();

        let _ = remove_file(project.folder.path().join(".fake-agent-turn"));
        project.scene.say(&thread, "remember the turn and wait").unwrap();
        let turn = noted_turn(&project.folder).await;
        project.scene.workspace.start_change(thread.id, turn, "fix-logout", None).await.unwrap();
        project.scene.workspace.stop(thread.id).unwrap();
        project.scene.wait_until_idle(&thread).await;
        expect(project.scene.thread_now(&thread).reviewer) to equal(None)
    }

    // requirement: a stopped question to the reviewer leaves the last review's count, and the thread free for either agent
    story a_stopped_review_counts_nothing {
        let project = project();
        let (thread, _) = project.changed("claude").await;
        project.review(&thread).await.unwrap();
        project.scene.ask_reviewer(&thread, "be slow").unwrap();
        project.scene.wait_until_reporting(&thread).await;
        expect(project.scene.thread_now(&thread).reviewer.map(|reviewer| reviewer.reviewing)) to equal(Some(true))
        project.scene.workspace.stop(thread.id).unwrap();
        project.scene.wait_until_idle(&thread).await;

        let reviewer = project.scene.thread_now(&thread).reviewer.expect("a reviewer");
        expect(reviewer.reviewing) to be_false
        expect(reviewer.verdict) to equal(Some(Verdict { findings: Some(2), answered: false, cut: false }))
        expect(project.lines_from(&thread, 1).last().cloned()) to equal(Some(reviewers("notice", "Stopped by you.")))
    }
}
