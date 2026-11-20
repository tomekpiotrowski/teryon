//! Specs for a thread's change: starting one, taking over a copy made by hand, and reading where it stands.
//!
//! These drive the workspace directly with real git repositories. The agent
//! is the fake one, which notes the turn Teryon started it for, so a test can
//! ask for a change as that turn, the way `teryon worktree create` does. The
//! command itself is covered by the service's specs.

mod common;

use std::fs::{canonicalize, create_dir_all};
use std::path::{Path, PathBuf};
use std::time::Duration;

use lets_expect::lets_expect;
use serde_json::Value;
use tempfile::TempDir;
use tokio::join;
use tokio::time::sleep;
use uuid::Uuid;

use common::{
    commit, git, invocations, noted_turn, project_folder, put, repository, unknown,
    written_by_agent, Scene,
};
use teryon_core::changes::{Change, ChangeState, Finish, Fit};
use teryon_core::files::{Content, FileChange, FileView, Listing};
use teryon_core::store::{Status, Thread};
use teryon_core::workspace::WorkspaceError;

/// A project in a repository on `main`, with a thread whose agent is running and has noted its turn.
struct Running {
    scene: Scene,
    folder: TempDir,
    thread: Thread,
    turn: Uuid,
}

async fn running(prepare: impl FnOnce(&Path)) -> Running {
    let scene = Scene::new();
    let folder = project_folder();
    prepare(folder.path());
    let project = scene.project(&folder);
    let thread = scene.thread(&project, "claude");
    scene.say(&thread, "remember the turn and wait").unwrap();
    let turn = noted_turn(&folder).await;
    Running {
        scene,
        folder,
        thread,
        turn,
    }
}

fn in_repository(directory: &Path) {
    git(directory, &["init", "-q", "-b", "main"]);
    put(directory, "README.md", "hello\n");
    commit(directory);
}

impl Running {
    async fn start(&self, name: &str, from: Option<&str>) -> Result<PathBuf, String> {
        self.start_as(self.thread.id, self.turn, name, from).await
    }

    async fn start_as(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        name: &str,
        from: Option<&str>,
    ) -> Result<PathBuf, String> {
        self.scene
            .workspace
            .start_change(thread_id, turn, name, from)
            .await
            .map_err(|problem| problem.to_string())
    }

    /// Stops the agent and waits for the turn to end.
    async fn finish(&self) {
        self.scene.workspace.stop(self.thread.id).unwrap();
        self.scene.wait_until_idle(&self.thread).await;
    }

    fn change(&self) -> Option<Change> {
        self.scene
            .thread_now(&self.thread)
            .change
            .map(|change| *change)
    }

    /// Looks at the change again, as when the thread is opened, and gives where it stands.
    async fn state(&self) -> ChangeState {
        self.scene.workspace.refresh_change(self.thread.id).await;
        self.change()
            .and_then(|change| change.state)
            .expect("the change has a state")
    }

    fn copy(&self, name: &str) -> PathBuf {
        canonicalize(self.folder.path())
            .unwrap()
            .join(".teryon/worktrees")
            .join(name)
    }

    fn head(&self, revision: &str) -> String {
        git(self.folder.path(), &["rev-parse", revision])
    }
}

fn state(files_changed: u32, fit: Fit) -> ChangeState {
    ChangeState {
        copy_present: true,
        base_present: true,
        files_changed,
        fit,
        unfinished: false,
    }
}

/// The changed files of a listing, each after the letter of its change: `M login.txt`.
fn changed_files(listing: &Listing) -> Vec<String> {
    listing
        .files
        .iter()
        .filter_map(|entry| {
            let letter = match entry.change? {
                FileChange::Added => "A",
                FileChange::Modified => "M",
                FileChange::Deleted => "D",
            };
            Some(format!("{letter} {}", entry.path))
        })
        .collect()
}

/// The text of a file as it is now, without the lines only the version it is compared with has.
fn current_text(view: &FileView) -> Vec<String> {
    let Content::Text { lines } = &view.content else {
        panic!("the file is not shown as text");
    };
    lines
        .iter()
        .filter(|line| line.number.is_some())
        .map(|line| line.text.clone())
        .collect()
}

lets_expect! { #tokio_test
    // requirement: in a repository, a thread without a change is told to start one with `teryon worktree create`
    // requirement: a question that changes nothing starts no change
    story a_question_starts_no_change {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        expect(scene.thread_now(&thread).change) to equal(None)
        let runs = invocations(&folder);
        expect(runs[0].contains("--append-system-prompt")) to be_true
        expect(runs[0].contains("first run `teryon worktree create <name>`")) to be_true
        expect(git(folder.path(), &["status", "--porcelain", "--untracked-files=all"]).contains(".teryon")) to be_false
    }

    // requirement: Grok is told the same, through its own option
    story grok_is_told_how_to_start_a_change {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "grok");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs[0].contains("--rules")) to be_true
        expect(runs[0].contains("first run `teryon worktree create <name>`")) to be_true
    }

    // requirement: the agent is given its thread and its turn, to show `teryon worktree create`
    story the_agent_knows_its_thread_and_turn {
        let run = running(in_repository).await;
        let noted = written_by_agent(&run.folder, ".fake-agent-turn").await;
        expect(noted.starts_with(&format!("{} ", run.thread.id))) to be_true
        expect(run.scene.thread_now(&run.thread).status) to equal(Status::Running)
        run.finish().await;
    }

    // requirement: a change gets a copy in `.teryon/worktrees/<name>` on a new branch `<name>`, from the branch the project is on
    // requirement: the next turn is told where the change is
    story a_change_is_started {
        let run = running(in_repository).await;
        let main = run.head("main");

        let path = run.start("fix-login", None).await;
        expect(path.clone()) to equal(Ok(run.copy("fix-login")))
        expect(git(run.folder.path(), &["branch", "--list", "fix-login"]).contains("fix-login")) to be_true
        expect(git(&run.copy("fix-login"), &["rev-parse", "HEAD"])) to equal(main.clone())

        let change = run.change().expect("the thread has a change");
        expect(change.name.clone()) to equal("fix-login".to_string())
        expect(change.path.clone()) to equal(run.copy("fix-login"))
        expect(change.base.clone()) to equal(Some("main".to_string()))
        expect(change.start.clone()) to equal(main)
        expect(change.branch.clone()) to equal(Some("fix-login".to_string()))

        run.finish().await;
        let after = run.change().and_then(|change| change.state);
        expect(after) to equal(Some(state(0, Fit::Applies)))

        run.scene.say(&run.thread, "hello").unwrap();
        run.scene.wait_until_idle(&run.thread).await;
        let runs = invocations(&run.folder);
        expect(runs[1].contains(&format!("This thread's worktree is `{}`, on the branch `fix-login`.", run.copy("fix-login").display()))) to be_true
        expect(git(run.folder.path(), &["status", "--porcelain", "--untracked-files=all"]).contains(".teryon")) to be_false
    }

    // requirement: starting a change is refused with a message the agent can act on
    story starting_a_change_is_refused {
        let run = running(|directory| {
            in_repository(directory);
            git(directory, &["branch", "taken"]);
            create_dir_all(directory.join(".teryon/worktrees/occupied")).unwrap();
        }).await;

        expect(run.start_as(unknown(), run.turn, "fix-login", None).await) to equal(Err("There is no such thread.".to_string()))
        expect(run.start_as(run.thread.id, unknown(), "fix-login", None).await) to equal(Err("`teryon worktree create` is for agents working in Teryon, during their turn.".to_string()))
        expect(run.start("Fix-Login", None).await) to equal(Err("A worktree name is 1 to 48 lowercase letters, digits and single dashes, starting with a letter, such as `fix-login-redirect`.".to_string()))
        expect(run.start("taken", None).await) to equal(Err("There is already a worktree or a branch called `taken`. Pick another name, such as `taken-2`.".to_string()))
        expect(run.start("occupied", None).await) to equal(Err("There is already a worktree or a branch called `occupied`. Pick another name, such as `occupied-2`.".to_string()))
        expect(run.start("fix-login", Some("nowhere")).await) to equal(Err("There is no branch called `nowhere` to start from.".to_string()))
        expect(run.change()) to equal(None)

        run.start("fix-login", None).await.unwrap();
        expect(run.start("another", None).await) to equal(Err(format!("This thread already has a worktree at `{}`. Work there.", run.copy("fix-login").display())))
        expect(git(run.folder.path(), &["branch", "--list", "another"])) to equal(String::new())

        run.finish().await;
        expect(run.start("later", None).await) to equal(Err("`teryon worktree create` is for agents working in Teryon, during their turn.".to_string()))
    }

    // requirement: a change starts from the branch the agent names
    story a_change_starts_from_a_named_branch {
        let run = running(|directory| {
            in_repository(directory);
            git(directory, &["checkout", "-q", "-b", "develop"]);
            put(directory, "develop.txt", "on develop\n");
            commit(directory);
            git(directory, &["checkout", "-q", "main"]);
        }).await;

        run.start("fix-login", Some("develop")).await.unwrap();
        let change = run.change().expect("the thread has a change");
        expect(change.base.clone()) to equal(Some("develop".to_string()))
        expect(change.start.clone()) to equal(run.head("develop"))
        expect(run.copy("fix-login").join("develop.txt").exists()) to be_true
        run.finish().await;
    }

    // requirement: a project that is not on a branch starts the change from the commit it is on
    story a_change_starts_from_the_commit_when_not_on_a_branch {
        let run = running(|directory| {
            in_repository(directory);
            git(directory, &["checkout", "-q", "--detach"]);
        }).await;

        run.start("fix-login", None).await.unwrap();
        let change = run.change().expect("the thread has a change");
        expect(change.base.clone()) to equal(None)
        expect(change.start.clone()) to equal(run.head("HEAD"))
        run.finish().await;
        expect(run.state().await.base_present) to be_false
    }

    // requirement: a change needs a repository with a commit to start from
    story a_change_needs_a_repository {
        let plain = running(|_| {}).await;
        expect(plain.start("fix-login", None).await) to equal(Err("This project is not a git repository, so there is nowhere to create a worktree.".to_string()))
        plain.finish().await;

        let empty = running(|directory| { git(directory, &["init", "-q", "-b", "main"]); }).await;
        expect(empty.start("fix-login", None).await) to equal(Err("This repository has no commits yet. Make a first commit before creating a worktree.".to_string()))
        empty.finish().await;
    }

    // requirement: files changed counts the copy as it is, committed or not
    // requirement: reading where a change stands leaves the agent's files and index as they were
    // requirement: a change that conflicts with newer work in its base overlaps
    story where_a_change_stands {
        let run = running(in_repository).await;
        run.start("fix-login", None).await.unwrap();
        run.finish().await;
        let copy = run.copy("fix-login");

        put(&copy, "committed.txt", "committed\n");
        commit(&copy);
        put(&copy, "README.md", "changed by the agent\n");
        put(&copy, "untracked.txt", "new\n");
        let before = git(&copy, &["status", "--porcelain"]);

        expect(run.state().await) to equal(state(3, Fit::Applies))
        expect(git(&copy, &["status", "--porcelain"])) to equal(before)

        put(run.folder.path(), "README.md", "changed in the project\n");
        commit(run.folder.path());
        expect(run.state().await) to equal(state(3, Fit::Overlaps))

        git(&copy, &["checkout", "-q", "-b", "elsewhere"]);
        let moved = run.state().await;
        expect(moved.unfinished) to be_true
        expect(run.change().and_then(|change| change.branch)) to equal(Some("elsewhere".to_string()))
    }

    // requirement: a change whose copy or base is gone says so
    story a_change_whose_copy_or_base_is_gone {
        let run = running(|directory| {
            in_repository(directory);
            git(directory, &["branch", "develop"]);
        }).await;
        run.start("fix-login", Some("develop")).await.unwrap();
        run.finish().await;

        git(run.folder.path(), &["branch", "-D", "develop"]);
        let without_base = run.state().await;
        expect(without_base.copy_present) to be_true
        expect(without_base.base_present) to be_false
        expect(without_base.fit) to equal(Fit::Unknown)

        git(run.folder.path(), &["worktree", "remove", "--force", &run.copy("fix-login").to_string_lossy()]);
        expect(run.state().await) to equal(ChangeState::gone())
    }

    // requirement: a change whose copy is removed is kept while its branch is left, and comes back with its copy
    story a_removed_copy_whose_branch_is_left {
        let run = running(in_repository).await;
        let copy = run.start("fix-login", None).await.unwrap();
        run.finish().await;
        put(&copy, "login.txt", "fixed\n");
        commit(&copy);

        git(run.folder.path(), &["worktree", "remove", "--force", &copy.to_string_lossy()]);
        expect(run.state().await) to equal(ChangeState::gone())
        expect(run.scene.thread_now(&run.thread).finished) to equal(None)

        git(run.folder.path(), &["worktree", "add", "-q", &copy.to_string_lossy(), "fix-login"]);
        expect(run.state().await) to equal(state(1, Fit::Applies))
    }

    // requirement: a change whose copy and branch are both removed is let go quietly, once
    story a_removed_copy_and_branch {
        let run = running(in_repository).await;
        let copy = run.start("fix-login", None).await.unwrap();
        run.finish().await;

        git(run.folder.path(), &["worktree", "remove", "--force", &copy.to_string_lossy()]);
        git(run.folder.path(), &["branch", "-D", "fix-login"]);
        let entries = run.scene.entries(&run.thread).len();
        join!(
            run.scene.workspace.refresh_change(run.thread.id),
            run.scene.workspace.refresh_change(run.thread.id),
        );
        let thread = run.scene.thread_now(&run.thread);
        expect(thread.change) to equal(None)
        expect(thread.finished) to equal(Some(Finish::Removed))
        let added: Vec<_> = run.scene.entries(&run.thread)[entries..]
            .iter()
            .map(|entry| entry.body["text"].clone())
            .collect();
        expect(added) to equal(vec![Value::from("Copy removed.")])
    }

    // requirement: a copy an agent makes by hand in `.teryon/worktrees/` goes to its thread when it is the only one running
    story a_copy_made_by_hand_is_taken_over {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "make a worktree called by-hand").unwrap();
        scene.wait_until_idle(&thread).await;

        let change = scene.thread_now(&thread).change.expect("the copy is taken over");
        expect(change.name.clone()) to equal("by-hand".to_string())
        expect(change.base.clone()) to equal(Some("main".to_string()))
        expect(change.branch.clone()) to equal(Some("by-hand".to_string()))
        expect(change.state.map(|state| state.copy_present)) to equal(Some(true))
    }

    // requirement: a copy made by hand while two agents run in the project goes to neither, even once one has stopped
    story a_copy_made_while_two_agents_run_goes_to_neither {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let slow = scene.thread(&project, "claude");
        let maker = scene.thread(&project, "claude");

        scene.say(&slow, "be slow").unwrap();
        scene.wait_until_reporting(&slow).await;
        scene.say(&maker, "make a worktree called both-running and wait").unwrap();
        scene.wait_until_reporting(&maker).await;
        sleep(Duration::from_millis(1500)).await;

        scene.workspace.stop(slow.id).unwrap();
        scene.wait_until_idle(&slow).await;
        sleep(Duration::from_millis(1500)).await;
        scene.workspace.stop(maker.id).unwrap();
        scene.wait_until_idle(&maker).await;

        expect(scene.thread_now(&slow).change) to equal(None)
        expect(scene.thread_now(&maker).change) to equal(None)
    }

    // requirement: a copy named after a thread, from before changes had names, goes to that thread
    story a_copy_named_after_its_thread_is_taken_over {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        git(folder.path(), &["worktree", "add", "-q", "-b", "old-style", &format!(".teryon/worktrees/{}", thread.id), "HEAD"]);

        scene.workspace.refresh_change(thread.id).await;
        let change = scene.thread_now(&thread).change.expect("the copy is taken over");
        expect(change.name.clone()) to equal(thread.id.to_string())
        expect(change.branch.clone()) to equal(Some("old-style".to_string()))
    }

    // requirement: a copy that was there before Teryon looked is not given to a thread by guessing
    story a_copy_already_there_is_left_alone {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        git(folder.path(), &["worktree", "add", "-q", "-b", "leftover", ".teryon/worktrees/leftover", "HEAD"]);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "be slow").unwrap();
        scene.wait_until_reporting(&thread).await;
        sleep(Duration::from_millis(1500)).await;
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;

        expect(scene.thread_now(&thread).change) to equal(None)
    }

    // requirement: a thread's copy is shown from its directory, with the work not committed there, even off any branch
    story the_files_of_a_copy_off_any_branch_are_its_own {
        let run = running(in_repository).await;
        let copy = run.start("fix-login", None).await.unwrap();
        run.finish().await;
        put(&copy, "login.txt", "first try\n");
        commit(&copy);
        // Half way through a rebase: off any branch, with work not committed yet.
        git(&copy, &["checkout", "-q", "--detach", "main"]);
        put(&copy, "login.txt", "resolved\n");
        put(&copy, "draft.txt", "draft\n");

        let project = run.thread.project_id;
        let thread = Some(run.thread.id);
        let listing = run.scene.workspace.files(project, thread, None, None).await.unwrap();
        expect(changed_files(&listing)) to equal(vec!["A draft.txt".to_string(), "A login.txt".to_string()])
        expect(listing.branch.clone()) to equal(None)
        expect(listing.base.clone()) to equal(Some("main".to_string()))
        let login = run.scene.workspace.file(project, thread, None, None, "login.txt").await.unwrap();
        expect(current_text(&login)) to equal(vec!["resolved".to_string()])

        // The branch alone has only what was committed on it.
        let by_branch = run.scene.workspace.file(project, None, Some("fix-login"), None, "login.txt").await.unwrap();
        expect(current_text(&by_branch)) to equal(vec!["first try".to_string()])

        // A commit wins over the copy.
        let start = run.head("main");
        let at_start = run.scene.workspace.files(project, thread, None, Some(&start)).await.unwrap();
        expect(changed_files(&at_start)) to equal(Vec::<String>::new())
        expect(at_start.commit.clone()) to equal(Some(start.clone()))
    }

    // requirement: the files of a thread's copy are only shown for a copy of the project, while it is there
    story the_files_of_a_copy_are_refused {
        let run = running(in_repository).await;
        let copy = run.start("fix-login", None).await.unwrap();
        run.finish().await;
        let project = run.thread.project_id;
        let refusal = |result: Result<Listing, WorkspaceError>| result.err().map(|problem| problem.to_string());

        let other_folder = project_folder();
        repository(&other_folder);
        let other = run.scene.project(&other_folder).id;
        expect(refusal(run.scene.workspace.files(other, Some(run.thread.id), None, None).await)) to equal(Some("There is no such thread.".to_string()))
        expect(refusal(run.scene.workspace.files(project, Some(unknown()), None, None).await)) to equal(Some("There is no such thread.".to_string()))

        let own = run.scene.projects().into_iter().find(|candidate| candidate.id == project).unwrap();
        let unchanged = run.scene.thread(&own, "claude");
        expect(refusal(run.scene.workspace.files(project, Some(unchanged.id), None, None).await)) to equal(Some("This thread has no changes.".to_string()))

        git(run.folder.path(), &["worktree", "remove", "--force", &copy.to_string_lossy()]);
        expect(refusal(run.scene.workspace.files(project, Some(run.thread.id), None, None).await)) to equal(Some("This thread's copy is gone, so its files cannot be shown.".to_string()))
    }

}
