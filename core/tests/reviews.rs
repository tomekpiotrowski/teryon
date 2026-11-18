//! Specs for finishing a thread's change on GitHub: sending it for review, following the review, and tidying up.
//!
//! GitHub is never reached. The project's `origin` names GitHub, but git is
//! told to use a bare repository in a temporary folder in its place, and
//! `tests/samples/fake-gh.sh` stands in for GitHub's `gh` program, answering
//! as each test arranges.

mod common;

use std::fs::{read_to_string, remove_file, write};
use std::path::{Path, PathBuf};

use lets_expect::lets_expect;
use tempfile::{tempdir, TempDir};

use common::{commit, fake_agents, git, noted_turn, project_folder, put, repository, setup, Scene};
use teryon_core::changes::{Review, ReviewState};
use teryon_core::protocol::ProjectView;
use teryon_core::settings::GitHubSettings;
use teryon_core::store::Thread;

const GITHUB: &str = "https://github.com/owner/app.git";

/// A project on `main` whose `origin` is "on GitHub", with a workspace that uses the fake `gh`.
struct OnGitHub {
    scene: Scene,
    folder: TempDir,
    /// What stands in for the repository on GitHub.
    remote: TempDir,
    project: ProjectView,
}

fn on_github(gh: &str) -> OnGitHub {
    let remote = tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let folder = project_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), "README.md", "hello\n");
    commit(folder.path());
    git(folder.path(), &["remote", "add", "origin", GITHUB]);
    let instead = format!("url.{}.insteadOf", remote.path().display());
    git(folder.path(), &["config", &instead, GITHUB]);
    git(folder.path(), &["push", "-q", "origin", "main"]);

    let mut chosen = setup(fake_agents());
    chosen.github = GitHubSettings {
        program: gh.to_string(),
    };
    let scene = Scene::with_setup(chosen);
    let project = scene.project(&folder);
    OnGitHub {
        scene,
        folder,
        remote,
        project,
    }
}

fn fake_gh() -> String {
    "tests/samples/fake-gh.sh".to_string()
}

impl OnGitHub {
    /// A new thread with a change called `name` holding one new file, its agent stopped.
    async fn change(&self, name: &str) -> (Thread, PathBuf) {
        let thread = self.scene.thread(&self.project, "claude");
        let _ = remove_file(self.folder.path().join(".fake-agent-turn"));
        self.scene
            .say(&thread, "remember the turn and wait")
            .unwrap();
        let turn = noted_turn(&self.folder).await;
        let path = self
            .scene
            .workspace
            .start_change(thread.id, turn, name, None)
            .await
            .unwrap();
        self.scene.workspace.stop(thread.id).unwrap();
        self.scene.wait_until_idle(&thread).await;
        put(&path, &format!("{name}.txt"), "made by the agent\n");
        (thread, path)
    }

    async fn send(&self, thread: &Thread, description: &str) -> Result<(), String> {
        self.scene
            .workspace
            .send_for_review(thread.id, description)
            .await
            .map_err(|problem| problem.to_string())
    }

    fn review(&self, thread: &Thread) -> Option<Review> {
        self.scene
            .thread_now(thread)
            .change
            .and_then(|change| change.review)
    }

    fn gh_calls(&self) -> String {
        read_to_string(self.folder.path().join(".fake-gh-invocations")).unwrap_or_default()
    }

    /// Arranges what the fake `gh` says about the review.
    fn review_becomes(&self, state: &str) {
        write(self.folder.path().join(".fake-gh-review"), state).unwrap();
    }

    /// Accepts the change on "GitHub": its `main` moves to the change's branch, as a merge there would.
    fn accept_on_github(&self, name: &str) {
        let tip = git(self.remote.path(), &["rev-parse", name]);
        git(self.remote.path(), &["update-ref", "refs/heads/main", &tip]);
        self.review_becomes("MERGED");
    }

    fn last_note(&self, thread: &Thread) -> String {
        let entries = self.scene.entries(thread);
        entries
            .last()
            .map(|entry| entry.body["text"].as_str().unwrap_or_default().to_string())
            .unwrap_or_default()
    }
}

fn in_review() -> Option<Review> {
    Some(Review {
        number: 12,
        url: "https://github.com/owner/app/pull/12".to_string(),
        state: ReviewState::InReview,
    })
}

fn exists(path: &Path) -> bool {
    path.exists()
}

lets_expect! { #tokio_test
    // requirement: a project whose origin is on GitHub is known as one
    // requirement: a change is sent for review: pushed, then opened on GitHub with the description's first line as its title
    story a_change_is_sent_for_review {
        let github = on_github(&fake_gh());
        expect(github.scene.projects()[0].on_github) to be_true
        let (thread, _) = github.change("fix-login").await;

        let sent = github.send(&thread, "Fix the login\n\nIt redirected twice.").await;
        expect(sent) to equal(Ok(()))
        expect(git(github.remote.path(), &["ls-tree", "--name-only", "fix-login"]).contains("fix-login.txt")) to be_true
        expect(github.gh_calls().contains("pr create --head fix-login --base main --title Fix the login --body It redirected twice.")) to be_true
        expect(github.review(&thread)) to equal(in_review())
        expect(github.send(&thread, "Again").await) to equal(Err("This change is already in review on GitHub.".to_string()))
    }

    // requirement: sending for review is refused off GitHub, without gh, signed out of it, or without a description
    story sending_for_review_is_refused {
        let local = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let plain = local.project(&folder);
        let thread = local.thread(&plain, "claude");
        local.say(&thread, "remember the turn and wait").unwrap();
        let turn = noted_turn(&folder).await;
        local.workspace.start_change(thread.id, turn, "fix-login", None).await.unwrap();
        local.workspace.stop(thread.id).unwrap();
        local.wait_until_idle(&thread).await;
        let refused = local.workspace.send_for_review(thread.id, "Fix").await.map_err(|problem| problem.to_string());
        expect(refused) to equal(Err("This project is not on GitHub.".to_string()))

        let missing = on_github("/nowhere/gh");
        let (missing_thread, _) = missing.change("fix-login").await;
        expect(missing.send(&missing_thread, "Fix").await) to equal(Err("GitHub's own program, gh, is not installed. Install it from cli.github.com, then sign in with `gh auth login`.".to_string()))

        let github = on_github(&fake_gh());
        let (thread, _) = github.change("fix-login").await;
        expect(github.send(&thread, " ").await) to equal(Err("Describe the change first.".to_string()))
        write(github.folder.path().join(".fake-gh-signed-out"), "").unwrap();
        expect(github.send(&thread, "Fix").await) to equal(Err("Sign in to GitHub first: run `gh auth login` in a terminal.".to_string()))
        expect(github.review(&thread)) to equal(None)
    }

    // requirement: once accepted on GitHub, the change is tidied away and the project folder brought up to date
    story an_accepted_change_is_tidied_away {
        let github = on_github(&fake_gh());
        let (thread, copy) = github.change("fix-login").await;
        github.send(&thread, "Fix the login").await.unwrap();

        github.review_becomes("OPEN");
        github.scene.workspace.refresh_change(thread.id).await;
        expect(github.review(&thread)) to equal(in_review())

        github.accept_on_github("fix-login");
        github.scene.workspace.refresh_change(thread.id).await;
        expect(github.scene.thread_now(&thread).change) to equal(None)
        expect(github.last_note(&thread)) to equal("Accepted on GitHub.".to_string())
        expect(exists(&copy)) to be_false
        expect(git(github.folder.path(), &["branch", "--list", "fix-login"])) to equal(String::new())
        expect(exists(&github.folder.path().join("fix-login.txt"))) to be_true
    }

    // requirement: the project folder is left as it is when the person's own edits are in the way, and the note says so
    story an_accepted_change_leaves_own_edits_alone {
        let github = on_github(&fake_gh());
        let (thread, _) = github.change("fix-login").await;
        github.send(&thread, "Fix the login").await.unwrap();
        put(github.folder.path(), "fix-login.txt", "the person's own\n");
        let before = git(github.folder.path(), &["rev-parse", "main"]);

        github.accept_on_github("fix-login");
        github.scene.workspace.refresh_change(thread.id).await;
        expect(github.last_note(&thread)) to equal("Accepted on GitHub. Your project folder does not have it yet.".to_string())
        expect(git(github.folder.path(), &["rev-parse", "main"])) to equal(before)
        expect(read_to_string(github.folder.path().join("fix-login.txt")).unwrap()) to equal("the person's own\n".to_string())
    }

    // requirement: a declined review is shown as declined, and the change stays
    story a_declined_review_keeps_the_change {
        let github = on_github(&fake_gh());
        let (thread, copy) = github.change("fix-login").await;
        github.send(&thread, "Fix the login").await.unwrap();

        github.review_becomes("CLOSED");
        github.scene.workspace.refresh_change(thread.id).await;
        expect(github.review(&thread).map(|review| review.state)) to equal(Some(ReviewState::Declined))
        expect(exists(&copy)) to be_true
    }
}
