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
use tempfile::TempDir;
use tokio::time::sleep;
use uuid::Uuid;

use common::{
    commit, git, invocations, noted_turn, project_folder, put, repository, unknown,
    written_by_agent, Scene,
};
use teryon_core::changes::{Change, ChangeState, Fit};
use teryon_core::store::{Status, Thread};

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
}
