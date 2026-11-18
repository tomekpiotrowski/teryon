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

use common::{commit, git, noted_turn, project_folder, put, Scene};
use teryon_core::changes::finish::CaughtUp;
use teryon_core::protocol::ProjectView;
use teryon_core::store::{Status, Thread};

/// A project in a repository on `main` with a `README.md`.
struct Project {
    scene: Scene,
    folder: TempDir,
    project: ProjectView,
}

fn project(prepare: impl FnOnce(&Path)) -> Project {
    let scene = Scene::new();
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

    async fn apply(&self, thread: &Thread, description: &str) -> Result<(), String> {
        self.scene
            .workspace
            .apply_change(thread.id, description)
            .await
            .map_err(|problem| problem.to_string())
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
        entries
            .last()
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

        let applied = project.apply(&thread, "  Fix the login\n\nIt redirected twice.  ").await;
        expect(applied) to equal(Ok(()))

        expect(project.git(&["rev-list", "--count", "main"])) to equal("2".to_string())
        expect(project.git(&["rev-parse", "main~1"])) to equal(main)
        expect(project.git(&["log", "-1", "--format=%B", "main"])) to equal("Fix the login\n\nIt redirected twice.".to_string())
        expect(project.read("login.txt")) to equal("fixed\n".to_string())
        expect(project.read("README.md")) to equal("hello, edited by the agent\n".to_string())
        expect(copy.exists()) to be_false
        expect(project.branch_exists("fix-login")) to be_false
        expect(project.scene.thread_now(&thread).change) to equal(None)
        expect(project.last_note(&thread)) to equal("Applied to the project.".to_string())
    }

    // requirement: a change to a version of the project that is checked out nowhere moves that version alone
    story a_change_is_applied_to_a_version_checked_out_nowhere {
        let project = project(|directory| { git(directory, &["branch", "develop"]); });
        let (thread, copy) = project.change("fix-login", Some("develop")).await;
        put(&copy, "login.txt", "fixed\n");

        expect(project.apply(&thread, "Fix the login").await) to equal(Ok(()))
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

        expect(project.apply(&thread, "Fix the login").await) to equal(Ok(()))
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

        expect(project.apply(&thread, "Fix the login").await) to equal(Err("You have edits of your own to README.md and new.txt in the project folder. Applying would overwrite them.".to_string()))
        let after = project.snapshot(&copy, "main");
        expect(after.0 == before.0 && after.1 == before.1 && after.3) to be_true
        expect(project.read("README.md")) to equal("by the person\n".to_string())
        expect(project.scene.thread_now(&thread).change.is_some()) to be_true
    }

    // requirement: applying is refused, leaving everything as it was, when the change overlaps with newer work
    story an_overlapping_change_is_not_applied {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "README.md", "by the agent\n");
        put(project.folder.path(), "README.md", "newer work\n");
        commit(project.folder.path());
        let before = project.git(&["rev-parse", "main"]);

        expect(project.apply(&thread, "Fix the login").await) to equal(Err("This change overlaps with newer work in the project. Ask the agent to catch up first.".to_string()))
        expect(project.git(&["rev-parse", "main"])) to equal(before)
        expect(read_to_string(copy.join("README.md")).unwrap()) to equal("by the agent\n".to_string())
        expect(project.scene.thread_now(&thread).change.is_some()) to be_true
    }

    // requirement: a change that does not overlap is applied on top of newer work
    story a_change_is_applied_on_top_of_newer_work {
        let project = project(|_| {});
        let (thread, copy) = project.change("fix-login", None).await;
        put(&copy, "login.txt", "fixed\n");
        put(project.folder.path(), "other.txt", "newer work\n");
        commit(project.folder.path());

        expect(project.apply(&thread, "Fix the login").await) to equal(Ok(()))
        expect(project.read("other.txt")) to equal("newer work\n".to_string())
        expect(project.read("login.txt")) to equal("fixed\n".to_string())
        expect(project.git(&["rev-list", "--count", "main"])) to equal("3".to_string())
    }

    // requirement: applying is refused with nothing to apply, without a description, while the agent works, without a base, or with the change unfinished
    story applying_is_refused {
        let project = project(|directory| { git(directory, &["branch", "develop"]); });
        let (unchanged, _) = project.change("unchanged", None).await;
        expect(project.apply(&unchanged, "Nothing").await) to equal(Err("No files have changed, so there is nothing to apply.".to_string()))

        let (undescribed, copy) = project.change("undescribed", None).await;
        put(&copy, "a.txt", "a\n");
        expect(project.apply(&undescribed, "  \n ").await) to equal(Err("Describe the change first.".to_string()))

        let (switched, switched_copy) = project.change("switched", None).await;
        put(&switched_copy, "a.txt", "a\n");
        git(&switched_copy, &["checkout", "-q", "-b", "elsewhere"]);
        expect(project.apply(&switched, "Switched").await) to equal(Err("The agent left this change unfinished. Ask it to finish, or discard the changes.".to_string()))

        let (orphan, orphan_copy) = project.change("orphan", Some("develop")).await;
        put(&orphan_copy, "a.txt", "a\n");
        project.git(&["branch", "-D", "develop"]);
        expect(project.apply(&orphan, "Orphan").await) to equal(Err("The version of the project this change started from is gone, so there is nothing to apply it to.".to_string()))

        project.scene.say(&undescribed, "be slow").unwrap();
        project.scene.wait_until_reporting(&undescribed).await;
        expect(project.apply(&undescribed, "Busy").await) to equal(Err("Wait for the agent to finish, or stop it.".to_string()))
        project.scene.workspace.stop(undescribed.id).unwrap();
        project.scene.wait_until_idle(&undescribed).await;

        let plain = project.scene.thread(&project.project, "claude");
        expect(project.apply(&plain, "No change").await) to equal(Err("This thread has no changes.".to_string()))
        expect(project.git(&["rev-list", "--count", "main"])) to equal("1".to_string())
    }

    // requirement: two changes applied at once in one project both land, one after the other
    story two_changes_are_applied_at_once {
        let project = project(|_| {});
        let (first, first_copy) = project.change("first", None).await;
        let (second, second_copy) = project.change("second", None).await;
        put(&first_copy, "first.txt", "1\n");
        put(&second_copy, "second.txt", "2\n");

        let (one, two) = join!(project.apply(&first, "First"), project.apply(&second, "Second"));
        expect((one, two)) to equal((Ok(()), Ok(())))
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

    // requirement: the description starts from what the agent said when it committed, or the thread's title
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
        git(&copy, &["commit", "-q", "-m", "Test the login"]);
        let drafted = project.scene.workspace.draft_description(thread.id).await.unwrap();
        expect(drafted) to equal("Test the login\n\n- Fix the login".to_string())
    }
}
