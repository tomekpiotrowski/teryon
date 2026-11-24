//! Specs for finishing a thread's change: applying it, catching it up, discarding it, and drafting its description.
//!
//! Applying and discarding are where a mistake costs most: a wrong apply
//! damages the project, a wrong discard loses work. So these run against
//! real repositories, and every refusal is checked to leave everything as it was.

mod common;

use std::fs::{canonicalize, read_to_string, remove_file};
use std::path::{Path, PathBuf};

use lets_expect::lets_expect;
use tempfile::TempDir;
use tokio::join;

use common::{commit, fake_agents, git, invocations, noted_turn, project_folder, put, setup, Scene};
use teryon_core::changes::finish::{apply, CaughtUp, STILL_OVERLAPS, TOO_MANY_ATTEMPTS};
use teryon_core::changes::{Change, Finish};
use teryon_core::protocol::ProjectView;
use teryon_core::settings::{ProviderChoice, StageSetting, StageSettings, Strength};
use teryon_core::store::{Status, Thread};
use teryon_core::workflow::Stage;
use teryon_core::workspace::{ApplyOutcome, Setup};

/// A project in a repository on `main` with a `README.md`.
struct Project {
    scene: Scene,
    folder: TempDir,
    project: ProjectView,
}

fn project(prepare: impl FnOnce(&Path)) -> Project {
    project_with_setup(setup(fake_agents()), prepare)
}

fn project_with_setup(setup: Setup, prepare: impl FnOnce(&Path)) -> Project {
    let scene = Scene::with_setup(setup);
    let folder = project_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), "README.md", "hello\n");
    commit(folder.path());
    prepare(folder.path());
    let project = scene.project(&folder);
    Project {
        scene,
        folder,
        project,
    }
}

impl Project {
    /// A new thread with a change called `name`, started as its agent would, the agent then stopped.
    async fn change(&self, name: &str, from: Option<&str>) -> (Thread, PathBuf) {
        let thread = self.scene.thread(&self.project, "claude");
        let _ = remove_file(self.folder.path().join(".fake-agent-turn"));
        self.scene
            .say(&thread, "remember the turn and wait")
            .unwrap();
        let turn = noted_turn(&self.folder).await;
        let path = self
            .scene
            .workspace
            .start_change(thread.id, turn, name, from)
            .await
            .expect("the change starts");
        self.scene.workspace.stop(thread.id).unwrap();
        self.scene.wait_until_idle(&thread).await;
        (thread, path)
    }

    async fn apply(&self, thread: &Thread, description: Option<&str>) -> Result<ApplyOutcome, String> {
        self.scene
            .workspace
            .apply_change(thread.id, description)
            .await
            .map_err(|problem| problem.to_string())
    }

    fn last_error(&self, thread: &Thread) -> String {
        self.scene
            .entries(thread)
            .iter()
            .rev()
            .find(|entry| entry.kind == "error")
            .map(|entry| entry.body["text"].as_str().unwrap_or_default().to_string())
            .unwrap_or_default()
    }

    async fn catch_up(&self, thread: &Thread) -> Result<CaughtUp, String> {
        self.scene
            .workspace
            .catch_up(thread.id)
            .await
            .map_err(|problem| problem.to_string())
    }

    async fn discard(&self, thread: &Thread) -> Result<(), String> {
        self.scene
            .workspace
            .discard_change(thread.id)
            .await
            .map_err(|problem| problem.to_string())
    }

    fn git(&self, arguments: &[&str]) -> String {
        git(self.folder.path(), arguments)
    }

    fn read(&self, path: &str) -> String {
        read_to_string(self.folder.path().join(path)).unwrap_or_default()
    }

    fn branch_exists(&self, name: &str) -> bool {
        !self.git(&["branch", "--list", name]).is_empty()
    }

    fn last_note(&self, thread: &Thread) -> String {
        let entries = self.scene.entries(thread);
        // The last notice: a line saying the thread moved to Done follows it.
        entries
            .iter()
            .rev()
            .find(|entry| entry.kind == "notice")
            .map(|entry| entry.body["text"].as_str().unwrap_or_default().to_string())
            .unwrap_or_default()
    }

    /// Everything a refusal must leave alone: the base's commit, the project folder's files and the copy's.
    fn snapshot(&self, copy: &Path, base: &str) -> (String, String, String, bool) {
        (
            self.git(&["rev-parse", base]),
            self.git(&["status", "--porcelain"]),
            git(copy, &["status", "--porcelain"]),
            copy.is_dir(),
        )
    }
}

lets_expect! { #tokio_test
    // requirement: applying adds the change to the project as one entry with the person's description
    // requirement: it updates the files in the project folder, removes the copy and forgets the change
    // requirement: files the agent left uncommitted are part of what is applied
    story a_change_is_applied {
        let project = project(|_| {});
        let main = project.git(&["rev-parse", "main"]);
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        commit(&copy);
        put(&copy, "README.md", "hello, edited by the agent\n");

        let applied = project.apply(&thread, Some("  Fix the login\n\nIt redirected twice.  ")).await;
        expect(applied) to equal(Ok(ApplyOutcome::Applied))

        expect(project.git(&["rev-list", "--count", "main"])) to equal("2".to_string())
        expect(project.git(&["rev-parse", "main~1"])) to equal(main)
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("Fix the login\n\nIt redirected twice.".to_string())
        expect(project.read("login.txt")) to equal("fixed\n".to_string())
        expect(project.read("README.md")) to equal("hello, edited by the agent\n".to_string())
        expect(copy.exists()) to be_false
        expect(project.branch_exists("fix-login")) to be_false
        expect(project.scene.thread_now(&thread).change) to equal(None)
        expect(project.last_note(&thread)) to equal("Applied to the project.".to_string())
        expect(project.scene.thread_now(&thread).finished) to equal(Some(Finish::Applied))
    }

    // requirement: applying without a description uses the agent's commit message, falling back to thread title and then change name
    story applying_without_description_uses_commit_message_or_fallback {
        let project = project(|_| {});

        // 1. Uses the agent's commit message (ignoring uncommitted leftovers saved by Teryon).
        let (committed, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        git(&copy, &["add", "-A"]);
        git(&copy, &["commit", "-q", "-m", "Fix login redirect\n\nAvoids duplicate redirect."]);
        put(&copy, "extra.txt", "leftover\n");
        expect(project.apply(&committed, None).await) to equal(Ok(ApplyOutcome::Applied))
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("Fix login redirect\n\nAvoids duplicate redirect.".to_string())
        expect(project.read("extra.txt")) to equal("leftover\n".to_string())

        // 2. Falls back to the thread title when the agent never committed.
        let (titled, copy) = project.change("add-notes", None).await;
        put(&copy, "notes.txt", "hello\n");
        expect(project.apply(&titled, None).await) to equal(Ok(ApplyOutcome::Applied))
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("remember the turn and wait".to_string())

        // 3. Falls back to the change name when the thread has no title and no commit exists.
        let worktree = project.folder.path().join(".teryon/worktrees/rename-settings");
        project.git(&["worktree", "add", "-q", "-b", "rename-settings", worktree.to_str().unwrap(), "main"]);
        put(&worktree, "settings.txt", "ok\n");
        let change = Change {
            name: "rename-settings".to_string(),
            path: worktree,
            base: Some("main".to_string()),
            start: project.git(&["rev-parse", "main"]),
            branch: Some("rename-settings".to_string()),
            state: None,
            review: None,
        };
        apply(project.folder.path(), &change, None, "").await.unwrap();
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("rename settings".to_string())
    }

    // requirement: a thread forgets how its last change ended once its agent starts another
    story a_new_change_forgets_how_the_last_one_ended {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        project.apply(&thread, Some("Fix the login")).await.unwrap();

        let _ = remove_file(project.folder.path().join(".fake-agent-turn"));
        project.scene.say(&thread, "remember the turn and wait").unwrap();
        let turn = noted_turn(&project.folder).await;
        project.scene.workspace.start_change(thread.id, turn, "fix-logout", None).await.unwrap();
        project.scene.workspace.stop(thread.id).unwrap();
        project.scene.wait_until_idle(&thread).await;

        let now = project.scene.thread_now(&thread);
        expect(now.finished) to equal(None)
        expect(now.change.map(|change| change.name)) to equal(Some("fix-logout".to_string()))
    }

    // requirement: a change to a version of the project that is checked out nowhere moves that version alone
    story a_change_is_applied_to_a_version_checked_out_nowhere {
        let project = project(|directory| { git(directory, &["branch", "develop"]); });
        let (thread, copy) = project.change("fix-login", Some("develop")).await;
        put(&copy, "login.txt", "fixed\n");

        expect(project.apply(&thread, Some("Fix the login")).await) to equal(Ok(ApplyOutcome::Applied))
        expect(project.git(&["log", "-1", "--format=%s", "develop"])) to equal("Fix the login".to_string())
        expect(project.git(&["rev-list", "--count", "main"])) to equal("1".to_string())
        expect(project.read("login.txt")) to equal(String::new())
    }

    // requirement: the person's own edits to other files in the project folder are kept
    story own_edits_to_other_files_are_kept {
        let project = project(|directory| { put(directory, "notes.txt", "mine\n"); commit(directory); });
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        put(project.folder.path(), "notes.txt", "mine, edited\n");

        expect(project.apply(&thread, Some("Fix the login")).await) to equal(Ok(ApplyOutcome::Applied))
        expect(project.read("notes.txt")) to equal("mine, edited\n".to_string())
        expect(project.read("login.txt")) to equal("fixed\n".to_string())
    }

    // requirement: applying is refused, leaving everything as it was, when the person's own edits touch the same files
    story own_edits_to_the_same_files_are_not_overwritten {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(&copy, "new.txt", "by the agent\n");
        put(project.folder.path(), "README.md", "by the person\n");
        put(project.folder.path(), "new.txt", "also by the person\n");
        let before = project.snapshot(&copy, "main");

        expect(project.apply(&thread, Some("Fix the login")).await) to equal(Err("You have edits of your own to README.md and new.txt in the project folder. Applying would overwrite them.".to_string()))
        let after = project.snapshot(&copy, "main");
        expect(after.0 == before.0 && after.1 == before.1 && after.3) to be_true
        expect(project.read("README.md")) to equal("by the person\n".to_string())
        expect(project.scene.thread_now(&thread).change.is_some()) to be_true
    }

    // requirement: applying an overlapping change starts the agent automatically and finishes applying once resolved
    story an_overlapping_change_resolves_and_applies_automatically {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(project.folder.path(), "README.md", "newer work\n");
        commit(project.folder.path());
        let newer = project.git(&["rev-parse", "main"]);

        expect(project.apply(&thread, None).await) to equal(Ok(ApplyOutcome::Resolving))
        let resolved = project.scene.wait_for(&thread, |current| current.status == Status::Idle && current.finished == Some(Finish::Applied)).await;

        expect(resolved.change) to equal(None)
        expect(resolved.stage) to equal(Stage::Done)
        expect(resolved.unread) to be_true
        expect(project.git(&["rev-parse", "main~1"])) to equal(newer)
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("Resolve overlap and keep fix".to_string())
        expect(project.read("README.md")) to equal("resolved by claude\n".to_string())
        expect(copy.exists()) to be_false
        expect(project.branch_exists("fix-login")) to be_false
        expect(project.last_note(&thread)) to equal("Applied to the project.".to_string())
        let ran = invocations(&project.folder);
        expect(ran.last().is_some_and(|line| line.contains("Teryon is about to apply this change to `main`"))) to be_true
    }

    // requirement: automatic apply retries when main moves again during resolution, and stops with a clear error when max attempts are reached or the same base still overlaps
    story overlapping_apply_retries_on_moving_base_and_stops_when_stuck {
        // 1. Retries once when main moves during resolution, then lands.
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-moving", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(project.folder.path(), "README.md", "first landing\n");
        commit(project.folder.path());
        put(project.folder.path(), ".fake-overlap-move-main", "1\n");

        expect(project.apply(&thread, None).await) to equal(Ok(ApplyOutcome::Resolving))
        let done = project.scene.wait_for(&thread, |current| current.status == Status::Idle && current.finished == Some(Finish::Applied)).await;
        expect(done.stage) to equal(Stage::Done)
        expect(project.read("README.md")) to equal("resolved by claude\n".to_string())

        // 2. Stops with a plain-language error when main keeps moving past max attempts.
        let busy = project_with_setup(setup(fake_agents()), |_| {});
        let (busy_thread, busy_copy) = busy.change("fix-busy", None).await;
        put(&busy_copy, "README.md", "by the agent\n");
        put(busy.folder.path(), "README.md", "landing 0\n");
        commit(busy.folder.path());
        put(busy.folder.path(), ".fake-overlap-move-main", "5\n");

        expect(busy.apply(&busy_thread, None).await) to equal(Ok(ApplyOutcome::Resolving))
        busy.scene.wait_until_idle(&busy_thread).await;
        let stuck = busy.scene.thread_now(&busy_thread);
        expect(stuck.finished) to equal(None)
        expect(stuck.change.is_some()) to be_true
        expect(stuck.unread) to be_true
        expect(busy.last_error(&busy_thread)) to equal(TOO_MANY_ATTEMPTS.to_string())

        // 3. Stops with a plain-language error if the same base still overlaps after the agent finishes.
        let same = project_with_setup(setup(fake_agents()), |_| {});
        let (same_thread, same_copy) = same.change("fix-same", None).await;
        put(&same_copy, "README.md", "by the agent\n");
        put(same.folder.path(), "README.md", "newer work\n");
        commit(same.folder.path());
        put(same.folder.path(), ".fake-overlap-noop", "\n");

        expect(same.apply(&same_thread, None).await) to equal(Ok(ApplyOutcome::Resolving))
        same.scene.wait_until_idle(&same_thread).await;
        let after = same.scene.thread_now(&same_thread);
        expect(after.finished) to equal(None)
        expect(after.change.is_some()) to be_true
        expect(after.unread) to be_true
        expect(same.last_error(&same_thread)) to equal(STILL_OVERLAPS.to_string())
    }

    // requirement: pending auto-apply is cancelled when stopped, failed, asking questions, or discarded mid-resolution
    story pending_apply_does_not_apply_on_stop_fail_ask_or_discard {
        // 1. Stopped by the person
        let stopped_project = project(|_| {});
        let (stopped, copy) = stopped_project.change("fix-stop", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(stopped_project.folder.path(), "README.md", "newer\n");
        commit(stopped_project.folder.path());
        put(stopped_project.folder.path(), ".fake-overlap-slow", "\n");

        expect(stopped_project.apply(&stopped, None).await) to equal(Ok(ApplyOutcome::Resolving))
        stopped_project.scene.wait_until_reporting(&stopped).await;
        stopped_project.scene.workspace.stop(stopped.id).unwrap();
        stopped_project.scene.wait_until_idle(&stopped).await;
        expect(stopped_project.scene.thread_now(&stopped).finished) to equal(None)
        expect(stopped_project.scene.thread_now(&stopped).change.is_some()) to be_true

        // 2. Agent fails
        let failed_project = project(|_| {});
        let (failing, copy) = failed_project.change("fix-fail", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(failed_project.folder.path(), "README.md", "newer\n");
        commit(failed_project.folder.path());
        put(failed_project.folder.path(), ".fake-overlap-fail", "\n");

        expect(failed_project.apply(&failing, None).await) to equal(Ok(ApplyOutcome::Resolving))
        failed_project.scene.wait_until_idle(&failing).await;
        expect(failed_project.scene.thread_now(&failing).finished) to equal(None)
        expect(failed_project.scene.thread_now(&failing).change.is_some()) to be_true

        // 3. Agent asks a question during resolution (even if it resolves and finishes cleanly)
        let asking_project = project(|_| {});
        let (asking, copy) = asking_project.change("fix-ask", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(asking_project.folder.path(), "README.md", "newer\n");
        commit(asking_project.folder.path());
        let _ = remove_file(asking_project.folder.path().join(".fake-agent-turn"));
        put(asking_project.folder.path(), ".fake-overlap-wait", "\n");

        expect(asking_project.apply(&asking, None).await) to equal(Ok(ApplyOutcome::Resolving))
        let turn = noted_turn(&asking_project.folder).await;
        asking_project
            .scene
            .workspace
            .ask_questions(
                asking.id,
                turn,
                r#"{"questions":[{"question":"Which way?","answers":[{"label":"A"},{"label":"B"}]}]}"#,
            )
            .unwrap();
        put(asking_project.folder.path(), ".fake-agent-release", "\n");
        asking_project.scene.wait_until_idle(&asking).await;
        expect(asking_project.scene.thread_now(&asking).asking) to be_true
        expect(asking_project.scene.thread_now(&asking).finished) to equal(None)
        expect(asking_project.scene.thread_now(&asking).change.is_some()) to be_true

        // 4. Discarded mid-resolution
        let discard_project = project(|_| {});
        let (discarded, copy) = discard_project.change("fix-discard", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(discard_project.folder.path(), "README.md", "newer\n");
        commit(discard_project.folder.path());
        put(discard_project.folder.path(), ".fake-overlap-slow", "\n");

        expect(discard_project.apply(&discarded, None).await) to equal(Ok(ApplyOutcome::Resolving))
        discard_project.scene.wait_until_reporting(&discarded).await;
        expect(discard_project.discard(&discarded).await) to equal(Ok(()))
        expect(discard_project.scene.thread_now(&discarded).finished) to equal(Some(Finish::Discarded))
        expect(copy.exists()) to be_false
    }

    // requirement: pending auto-apply is preserved across an automatic provider handoff after a usage limit
    story pending_apply_survives_usage_limit_handoff {
        let mut custom = setup(fake_agents());
        custom.stages = StageSettings {
            talking: Some(StageSetting {
                provider: Some(ProviderChoice::Any),
                strength: Some(Strength::Medium),
                ..StageSetting::default()
            }),
            implementing: Some(StageSetting {
                provider: Some(ProviderChoice::Any),
                strength: Some(Strength::Medium),
                ..StageSetting::default()
            }),
            ..StageSettings::default()
        };
        let project = project_with_setup(custom, |_| {});
        let (thread, copy) = project.change("fix-limit", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(project.folder.path(), "README.md", "newer work\n");
        commit(project.folder.path());
        put(project.folder.path(), ".fake-overlap-limit", "\n");

        expect(project.apply(&thread, None).await) to equal(Ok(ApplyOutcome::Resolving))
        let done = project.scene.wait_for(&thread, |current| current.status == Status::Idle && current.finished == Some(Finish::Applied)).await;

        expect(done.stage) to equal(Stage::Done)
        expect(done.agent != "claude") to be_true
        expect(project.read("README.md").starts_with("resolved by ")) to be_true
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("Resolve overlap and keep fix".to_string())
    }

    // requirement: OwnEdits during automatic apply after resolution surfaces cleanly as a thread error
    story own_edits_during_resolution_surfaces_as_thread_error {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-own-edit", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(project.folder.path(), "README.md", "newer work\n");
        commit(project.folder.path());
        put(project.folder.path(), ".fake-overlap-own-edit", "\n");

        expect(project.apply(&thread, None).await) to equal(Ok(ApplyOutcome::Resolving))
        project.scene.wait_until_idle(&thread).await;

        let after = project.scene.thread_now(&thread);
        expect(after.finished) to equal(None)
        expect(after.change.is_some()) to be_true
        expect(after.unread) to be_true
        expect(project.last_error(&thread)) to equal("You have edits of your own to README.md in the project folder. Applying would overwrite them.".to_string())
        expect(project.read("README.md")) to equal("person edit during resolution\n".to_string())
    }

    // requirement: a change that does not overlap is applied on top of newer work as one linear commit
    story a_change_is_applied_on_top_of_newer_work {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        put(project.folder.path(), "other.txt", "newer work\n");
        commit(project.folder.path());
        let newer = project.git(&["rev-parse", "main"]);

        expect(project.apply(&thread, Some("Fix the login")).await) to equal(Ok(ApplyOutcome::Applied))
        expect(project.read("other.txt")) to equal("newer work\n".to_string())
        expect(project.read("login.txt")) to equal("fixed\n".to_string())
        expect(project.git(&["rev-list", "--count", "main"])) to equal("3".to_string())
        expect(project.git(&["rev-parse", "main~1"])) to equal(newer)
    }

    // requirement: applying is refused with nothing to apply, without a description, while the agent works, without a base, or with the change unfinished
    story applying_is_refused {
        let project = project(|directory| { git(directory, &["branch", "develop"]); });
        let (unchanged, _) = project.change("unchanged", None).await;
        expect(project.apply(&unchanged, Some("Nothing")).await) to equal(Err("No files have changed, so there is nothing to apply.".to_string()))

        let (undescribed, copy) = project.change("undescribed", None).await;
        put(&copy, "a.txt", "a\n");
        expect(project.apply(&undescribed, Some("  \n ")).await) to equal(Err("Describe the change first.".to_string()))

        let (switched, switched_copy) = project.change("switched", None).await;
        put(&switched_copy, "a.txt", "a\n");
        git(&switched_copy, &["checkout", "-q", "-b", "elsewhere"]);
        expect(project.apply(&switched, Some("Switched")).await) to equal(Err("The agent left this change unfinished. Ask it to finish, or discard the changes.".to_string()))

        let (orphan, orphan_copy) = project.change("orphan", Some("develop")).await;
        put(&orphan_copy, "a.txt", "a\n");
        project.git(&["branch", "-D", "develop"]);
        expect(project.apply(&orphan, Some("Orphan")).await) to equal(Err("The version of the project this change started from is gone, so there is nothing to apply it to.".to_string()))

        project.scene.say(&undescribed, "be slow").unwrap();
        project.scene.wait_until_reporting(&undescribed).await;
        expect(project.apply(&undescribed, Some("Busy")).await) to equal(Err("Wait for the agent to finish, or stop it.".to_string()))
        project.scene.workspace.stop(undescribed.id).unwrap();
        project.scene.wait_until_idle(&undescribed).await;

        let plain = project.scene.thread(&project.project, "claude");
        expect(project.apply(&plain, Some("No change")).await) to equal(Err("This thread has no changes.".to_string()))
        expect(project.git(&["rev-list", "--count", "main"])) to equal("1".to_string())
        expect(project.scene.thread_now(&undescribed).finished) to equal(None)
    }

    // requirement: two changes applied at once in one project both land, one after the other
    story two_changes_are_applied_at_once {
        let project = project(|_| {});
        let (first, first_copy) = project.change("first", None).await;
        let (second, second_copy) = project.change("second", None).await;
        put(&first_copy, "first.txt", "1\n");
        put(&second_copy, "second.txt", "2\n");

        let (one, two) = join!(project.apply(&first, Some("First")), project.apply(&second, Some("Second")));
        expect((one, two)) to equal((Ok(ApplyOutcome::Applied), Ok(ApplyOutcome::Applied)))
        expect(project.git(&["rev-list", "--count", "main"])) to equal("3".to_string())
        expect((project.read("first.txt"), project.read("second.txt"))) to equal(("1\n".to_string(), "2\n".to_string()))
    }

    // requirement: discarding removes the copy with everything in it, and forgets the change
    story a_change_is_discarded {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "committed.txt", "1\n");
        commit(&copy);
        put(&copy, "uncommitted.txt", "2\n");

        expect(project.discard(&thread).await) to equal(Ok(()))
        expect(copy.exists()) to be_false
        expect(project.branch_exists("fix-login")) to be_false
        expect(project.git(&["rev-list", "--count", "main"])) to equal("1".to_string())
        expect(project.scene.thread_now(&thread).change) to equal(None)
        expect(project.last_note(&thread)) to equal("Changes discarded.".to_string())
        expect(project.scene.thread_now(&thread).finished) to equal(Some(Finish::Discarded))
    }

    // requirement: discarding stops the agent first
    story a_change_is_discarded_while_the_agent_works {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        project.scene.say(&thread, "be slow").unwrap();
        project.scene.wait_until_reporting(&thread).await;

        expect(project.discard(&thread).await) to equal(Ok(()))
        expect(copy.exists()) to be_false
        expect(project.scene.thread_now(&thread).status) to equal(Status::Idle)
    }

    // requirement: a change whose copy is already gone is discarded all the same
    story a_change_whose_copy_is_gone_is_discarded {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        project.git(&["worktree", "remove", "--force", &copy.to_string_lossy()]);

        expect(project.discard(&thread).await) to equal(Ok(()))
        expect(project.branch_exists("fix-login")) to be_false
        expect(project.scene.thread_now(&thread).change) to equal(None)
    }

    // requirement: catching up brings in newer work, keeping what the agent left uncommitted
    story a_change_catches_up {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        put(project.folder.path(), "other.txt", "newer work\n");
        commit(project.folder.path());

        expect(project.catch_up(&thread).await) to equal(Ok(CaughtUp::Done))
        expect(read_to_string(copy.join("other.txt")).unwrap()) to equal("newer work\n".to_string())
        expect(read_to_string(copy.join("login.txt")).unwrap()) to equal("fixed\n".to_string())
        expect(git(&copy, &["merge-base", "--is-ancestor", "main", "HEAD"]).is_empty()) to be_true
        expect(project.catch_up(&thread).await) to equal(Err("This change already has all the newer work in the project.".to_string()))
    }

    // requirement: catching up a change that overlaps leaves its files as they were
    story an_overlapping_change_does_not_catch_up {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(project.folder.path(), "README.md", "newer work\n");
        commit(project.folder.path());

        expect(project.catch_up(&thread).await) to equal(Ok(CaughtUp::Overlaps))
        expect(read_to_string(copy.join("README.md")).unwrap()) to equal("by the agent\n".to_string())
        expect(git(&copy, &["status", "--porcelain"])) to equal(String::new())
        expect(canonicalize(copy.join(".git")).is_ok()) to be_true
        let state = project.scene.thread_now(&thread).change.and_then(|change| change.state);
        expect(state.map(|state| state.unfinished)) to equal(Some(false))
    }

    // requirement: the description starts from what the agent last committed, or the thread's title
    story the_description_is_drafted {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        let untitled = project.scene.workspace.draft_description(thread.id).await.unwrap();
        expect(untitled) to equal("remember the turn and wait".to_string())

        git(&copy, &["add", "-A"]);
        git(&copy, &["commit", "-q", "-m", "Fix the login\n\nIt redirected twice."]);
        put(&copy, "tests.txt", "tested\n");
        git(&copy, &["add", "-A"]);
        git(&copy, &["commit", "-q", "-m", "Fix and test the login"]);
        let drafted = project.scene.workspace.draft_description(thread.id).await.unwrap();
        expect(drafted) to equal("Fix and test the login".to_string())
    }
}
