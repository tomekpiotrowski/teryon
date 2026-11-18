//! Specs for the history screen.
//!
//! Every project here is a real git repository made for the test, because the
//! answers come from the `git` program. Each commit is made at a moment the
//! test chooses, so that their order and who was working at the time are
//! known.

mod common;

use std::path::Path;

use lets_expect::lets_expect;
use tempfile::TempDir;
use uuid::Uuid;

use common::{git, git_at, project_folder, put, unknown, Scene};
use teryon_core::files::diff::LineChange;
use teryon_core::files::{Content, FileChange, FileView};
use teryon_core::history::{Changes, Commit, History};

/// The moment of the first commit of every repository here. Later commits come a minute apart.
const START: i64 = 1_700_000_000;

/// What the screen is told when the project's directory is not a git repository.
const NOT_A_REPOSITORY: &str =
    "This project's directory is not a git repository, so there are no files or history to show.";

/// A commit name that no repository here has.
const MISSING_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A message whose first line is followed by a longer explanation.
const LONG_MESSAGE: &str =
    "Change the app\n\nThe second line is louder now.\n\nAnd there is a fourth.";

/// Commits everything in a directory at a moment and returns the commit's name.
fn record(directory: &Path, minute: i64, message: &str) -> String {
    git(directory, &["add", "-A"]);
    git_at(
        directory,
        START + minute * 60,
        &["commit", "-q", "--allow-empty", "-m", message],
    );
    git(directory, &["rev-parse", "HEAD"])
}

/// Commits nothing at a moment, in seconds since 1970, with a message.
fn record_at(directory: &Path, time: i64, message: &str) {
    git_at(
        directory,
        time,
        &["commit", "-q", "--allow-empty", "-m", message],
    );
}

/// A project folder holding a repository with one commit on `main`.
fn repository() -> (TempDir, String) {
    let folder = project_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), "README.md", "# Sample\n");
    put(folder.path(), "src/app.txt", "one\ntwo\nthree\n");
    let first = record(folder.path(), 0, "Start the sample");
    (folder, first)
}

/// A repository whose branch `feature` has two commits of its own and has merged `main` once.
///
/// In the order they are listed: the merge, `Change the app` and `Add notes`
/// are the branch's own, `Move main on` came in through the merge, and
/// `Start the sample` is where the branch left `main`. The directory is left
/// on `feature`.
fn branched() -> TempDir {
    let (folder, _) = repository();
    git(folder.path(), &["switch", "-q", "-c", "feature"]);
    put(folder.path(), "notes.txt", "notes\n");
    record(folder.path(), 1, "Add notes");
    put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
    record(folder.path(), 2, "Change the app");
    git(folder.path(), &["switch", "-q", "main"]);
    put(folder.path(), "README.md", "# Sample, moved on\n");
    record(folder.path(), 3, "Move main on");
    git(folder.path(), &["switch", "-q", "feature"]);
    git_at(
        folder.path(),
        START + 4 * 60,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge main into feature",
            "main",
        ],
    );
    folder
}

/// The commits of a branch, with a refusal as its message.
async fn history(scene: &Scene, project: Uuid, branch: Option<&str>) -> Result<History, String> {
    scene
        .workspace
        .history(project, branch)
        .await
        .map_err(|problem| problem.to_string())
}

/// What a commit changed, with a refusal as its message.
async fn changes(
    scene: &Scene,
    project: Uuid,
    commit: &str,
    from: Option<&str>,
) -> Result<Changes, String> {
    scene
        .workspace
        .changes(project, commit, from)
        .await
        .map_err(|problem| problem.to_string())
}

/// One file a commit changed, with a refusal as its message.
async fn change(
    scene: &Scene,
    project: Uuid,
    commit: &str,
    from: Option<&str>,
    path: &str,
) -> Result<FileView, String> {
    scene
        .workspace
        .change(project, commit, from, path)
        .await
        .map_err(|problem| problem.to_string())
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn refusal(message: &str) -> Option<String> {
    Some(message.to_string())
}

fn branch_name(name: &str) -> Option<String> {
    Some(name.to_string())
}

/// One field of every commit listed, in order.
fn each<T>(history: &History, field: impl Fn(&Commit) -> T) -> Vec<T> {
    history.commits.iter().map(field).collect()
}

/// The commits listed, each as `subject` or, when the main branch has it too, `subject (main)`.
fn listed(history: &History) -> Vec<String> {
    each(history, |commit| {
        if commit.on_base {
            format!("{} (main)", commit.subject)
        } else {
            commit.subject.clone()
        }
    })
}

/// One commit listed, found by the first line of its message.
fn commit_named<'a>(history: &'a History, subject: &str) -> &'a Commit {
    history
        .commits
        .iter()
        .find(|commit| commit.subject == subject)
        .expect("the commit is listed")
}

/// The thread named by one commit listed, found by the first line of its message.
fn thread_of(history: &History, subject: &str) -> Option<Uuid> {
    commit_named(history, subject).thread
}

/// The files a commit changed: `M src/app.txt +1 -1`, without counts for a file that is not text.
fn changed(changes: &Changes) -> Vec<String> {
    changes
        .files
        .iter()
        .map(|file| {
            let letter = match file.change {
                FileChange::Added => "A",
                FileChange::Modified => "M",
                FileChange::Deleted => "D",
            };
            let counts = match (file.added, file.removed) {
                (Some(added), Some(removed)) => format!(" +{added} -{removed}"),
                _ => String::new(),
            };
            format!("{letter} {}{counts}", file.path)
        })
        .collect()
}

/// What a file holds, named as the screen names it.
fn kind(view: &FileView) -> &'static str {
    match view.content {
        Content::Text { .. } => "text",
        Content::Binary => "binary",
        Content::TooLarge => "too_large",
    }
}

/// The lines of a file: `2 two` unchanged, `+2 TWO` added, `- two` removed.
fn shown(view: &FileView) -> Vec<String> {
    let Content::Text { lines } = &view.content else {
        panic!("the file is not shown as text");
    };
    lines
        .iter()
        .map(|line| {
            let mark = match line.change {
                Some(LineChange::Added) => "+",
                Some(LineChange::Removed) => "-",
                None => "",
            };
            let number = line.number.map(|number| number.to_string());
            format!("{mark}{} {}", number.unwrap_or_default(), line.text)
        })
        .collect()
}

lets_expect! { #tokio_test
    // requirement: the commits of the project's directory are listed newest first, each with who made it, when, on top of what, and the files it changed
    story the_commits_of_the_project_directory_are_listed {
        let scene = Scene::new();
        let (folder, first) = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        let second = record(folder.path(), 1, "Change the app");
        put(folder.path(), "draft.txt", "not committed\n");

        let listing = history(&scene, project, None).await;
        expect(listing.is_ok()) to be_true
        let listing = listing.unwrap();
        expect(each(&listing, |commit| commit.subject.clone())) to equal(names(&["Change the app", "Start the sample"]))
        expect(each(&listing, |commit| commit.hash.clone())) to equal(vec![second.clone(), first.clone()])
        expect(each(&listing, |commit| commit.parents.clone())) to equal(vec![vec![first.clone()], Vec::new()])
        expect(each(&listing, |commit| commit.author.clone())) to equal(names(&["Tester", "Tester"]))
        expect(each(&listing, |commit| commit.email.clone())) to equal(names(&["tester@example.com", "tester@example.com"]))
        expect(each(&listing, |commit| commit.time)) to equal(vec![START + 60, START])
        expect(each(&listing, |commit| commit.files.clone())) to equal(vec![
            names(&["notes/new file \u{17c}.txt", "src/app.txt"]),
            names(&["README.md", "src/app.txt"]),
        ])
        expect(each(&listing, |commit| commit.on_base)) to equal(vec![false, false])
        expect(each(&listing, |commit| commit.thread)) to equal(vec![None, None])
        expect(listing.branches.clone()) to equal(names(&["main"]))
        expect(listing.branch.clone()) to equal(branch_name("main"))
        expect(listing.base.clone()) to equal(None)
        expect(listing.ahead) to equal(0)
        expect(listing.more) to be_false
    }

    // requirement: on a branch other than the main one, the commits the main branch has too are told apart, and the branch's own are counted
    // requirement: a merge lists the files it brought in
    story a_branch_is_told_apart_from_the_main_branch {
        let scene = Scene::new();
        let folder = branched();
        let project = scene.project(&folder).id;

        let listing = history(&scene, project, None).await.unwrap();
        expect(listed(&listing)) to equal(names(&[
            "Merge main into feature",
            "Change the app",
            "Add notes",
            "Move main on (main)",
            "Start the sample (main)",
        ]))
        expect(listing.branch.clone()) to equal(branch_name("feature"))
        expect(listing.base.clone()) to equal(branch_name("main"))
        expect(listing.ahead) to equal(3)
        expect(listing.branches.clone()) to equal(names(&["feature", "main"]))
        expect(commit_named(&listing, "Merge main into feature").parents.len()) to equal(2)
        expect(commit_named(&listing, "Merge main into feature").files.clone()) to equal(names(&["README.md"]))

        let named = history(&scene, project, Some("feature")).await.unwrap();
        expect(listed(&named)) to equal(listed(&listing))

        let main = history(&scene, project, Some("main")).await.unwrap();
        expect(listed(&main)) to equal(names(&["Move main on", "Start the sample"]))
        expect(main.branch.clone()) to equal(branch_name("main"))
        expect(main.base.clone()) to equal(None)
        expect(main.ahead) to equal(0)
    }

    // requirement: a branch is listed as it was committed, wherever it is checked out
    story a_branch_checked_out_nowhere_is_listed {
        let scene = Scene::new();
        let folder = branched();
        let project = scene.project(&folder).id;
        git(folder.path(), &["switch", "-q", "main"]);

        let own = history(&scene, project, None).await.unwrap();
        expect(listed(&own)) to equal(names(&["Move main on", "Start the sample"]))

        let listing = history(&scene, project, Some("feature")).await.unwrap();
        expect(listed(&listing)) to equal(names(&[
            "Merge main into feature",
            "Change the app",
            "Add notes",
            "Move main on (main)",
            "Start the sample (main)",
        ]))
        expect(listing.branch.clone()) to equal(branch_name("feature"))
    }

    // requirement: a directory that is not on a branch still has a history, told apart from the main branch
    story a_directory_off_any_branch_is_listed {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["checkout", "-q", "--detach"]);
        put(folder.path(), "notes.txt", "notes\n");
        record(folder.path(), 1, "Add notes");

        let listing = history(&scene, project, None).await.unwrap();
        expect(listed(&listing)) to equal(names(&["Add notes", "Start the sample (main)"]))
        expect(listing.branch.clone()) to equal(None)
        expect(listing.base.clone()) to equal(branch_name("main"))
        expect(listing.ahead) to equal(1)
    }

    // requirement: without a main branch no commit is told apart
    story a_repository_without_a_main_branch_tells_nothing_apart {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["branch", "-q", "-m", "trunk"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "notes.txt", "notes\n");
        record(folder.path(), 1, "Add notes");

        let listing = history(&scene, project, None).await.unwrap();
        expect(listed(&listing)) to equal(names(&["Add notes", "Start the sample"]))
        expect(listing.base.clone()) to equal(None)
        expect(listing.ahead) to equal(0)
    }

    // requirement: `master` counts as the main branch when there is no `main`
    story master_is_the_main_branch_of_an_older_repository {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["branch", "-q", "-m", "master"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "notes.txt", "notes\n");
        record(folder.path(), 1, "Add notes");

        let listing = history(&scene, project, None).await.unwrap();
        expect(listed(&listing)) to equal(names(&["Add notes", "Start the sample (main)"]))
        expect(listing.base.clone()) to equal(branch_name("master"))
    }

    // requirement: a repository without a commit has no history
    story a_repository_without_a_commit_has_no_history {
        let scene = Scene::new();
        let folder = project_folder();
        git(folder.path(), &["init", "-q", "-b", "main"]);
        put(folder.path(), "loose.txt", "two\n");
        let project = scene.project(&folder).id;

        let listing = history(&scene, project, None).await;
        expect(listing.is_ok()) to be_true
        let listing = listing.unwrap();
        expect(listed(&listing)) to equal(names(&[]))
        expect(listing.branches.clone()) to equal(names(&["main"]))
        expect(listing.branch.clone()) to equal(branch_name("main"))
        expect(listing.more) to be_false

        let named = history(&scene, project, Some("main")).await;
        expect(named.is_ok()) to be_true
        expect(listed(&named.unwrap())) to equal(names(&[]))
    }

    // requirement: at most 200 commits are listed, and the answer says when there are older ones
    story a_long_history_is_cut_short {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder).id;
        for number in 1..200 {
            record(folder.path(), number, &format!("Commit {number}"));
        }

        let whole = history(&scene, project, None).await.unwrap();
        expect(whole.commits.len()) to equal(200)
        expect(whole.more) to be_false

        record(folder.path(), 200, "Commit 200");
        let cut = history(&scene, project, None).await.unwrap();
        expect(cut.commits.len()) to equal(200)
        expect(cut.more) to be_true
        expect(cut.commits.first().map(|commit| commit.subject.clone())) to equal(Some("Commit 200".to_string()))
        expect(cut.commits.last().map(|commit| commit.subject.clone())) to equal(Some("Commit 1".to_string()))
    }

    // requirement: a commit made while an agent was working in one of the project's threads names that thread
    //
    // The fake agent really runs, so the turn's start and end are the times
    // its conversation was stored with, and the commits are dated around them.
    story a_commit_names_the_thread_whose_agent_was_working {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder);
        let elsewhere = project_folder();
        let other_project = scene.project(&elsewhere);

        let other_thread = scene.thread(&other_project, "claude");
        scene.say(&other_thread, "hello").unwrap();
        scene.wait_until_idle(&other_thread).await;
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        let entries = scene.entries(&thread);
        let started = entries.first().expect("a message").created_at.timestamp();
        let ended = entries.last().expect("a report").created_at.timestamp();
        let elsewhere_started = scene.entries(&other_thread)[0].created_at.timestamp();

        record_at(folder.path(), started - 3600, "Long before");
        record_at(folder.path(), elsewhere_started.min(started - 1), "During another project's turn");
        record_at(folder.path(), started, "As the turn started");
        record_at(folder.path(), ended, "As the turn ended");
        record_at(folder.path(), ended + 3600, "Long after");

        let listing = history(&scene, project.id, None).await.unwrap();
        expect(thread_of(&listing, "Long before")) to equal(None)
        expect(thread_of(&listing, "During another project's turn")) to equal(None)
        expect(thread_of(&listing, "As the turn started")) to equal(Some(thread.id))
        expect(thread_of(&listing, "As the turn ended")) to equal(Some(thread.id))
        expect(thread_of(&listing, "Long after")) to equal(None)

        // An agent still working may have made anything since it started.
        scene.say(&thread, "be slow").unwrap();
        let running = history(&scene, project.id, None).await.unwrap();
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(thread_of(&running, "Long after")) to equal(Some(thread.id))
        expect(thread_of(&running, "Long before")) to equal(None)
    }

    // requirement: a branch that does not exist is refused
    story an_unknown_branch_is_refused {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder).id;
        let no_branch = refusal("There is no such branch.");

        expect(history(&scene, project, Some("nothing")).await.err()) to equal(no_branch.clone())
        expect(history(&scene, project, Some("--all")).await.err()) to equal(no_branch.clone())
        expect(history(&scene, project, Some("HEAD")).await.err()) to equal(no_branch.clone())
    }

    // requirement: a project whose directory is not a git repository has no history
    story a_directory_that_is_not_a_repository_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        put(folder.path(), "README.md", "# Sample\n");
        let project = scene.project(&folder).id;

        expect(history(&scene, project, None).await.err()) to equal(refusal(NOT_A_REPOSITORY))
        expect(changes(&scene, project, "0123456789abcdef", None).await.err()) to equal(refusal(NOT_A_REPOSITORY))
        expect(change(&scene, project, "0123456789abcdef", None, "README.md").await.err()) to equal(refusal(NOT_A_REPOSITORY))
    }

    // requirement: only the history of a project Teryon knows is shown
    story an_unknown_project_is_refused {
        let scene = Scene::new();
        let (_folder, first) = repository();
        let no_project = refusal("There is no such project.");

        expect(history(&scene, unknown(), None).await.err()) to equal(no_project.clone())
        expect(changes(&scene, unknown(), &first, None).await.err()) to equal(no_project.clone())
        expect(change(&scene, unknown(), &first, None, "README.md").await.err()) to equal(no_project.clone())
    }

    // requirement: a commit shows the rest of its message and the files it changed, each with how many lines were added and removed
    story a_commit_shows_what_it_changed {
        let scene = Scene::new();
        let (folder, first) = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        put(folder.path(), "picture.bin", [137, 80, 78, 71, 0, 1, 2]);
        git(folder.path(), &["rm", "-q", "README.md"]);
        let second = record(folder.path(), 1, LONG_MESSAGE);

        let commit = changes(&scene, project, &second, None).await;
        expect(commit.is_ok()) to be_true
        let commit = commit.unwrap();
        expect(changed(&commit)) to equal(names(&[
            "D README.md +0 -1",
            "A notes/new file \u{17c}.txt +1 -0",
            "A picture.bin",
            "M src/app.txt +2 -1",
        ]))
        expect(commit.commit.clone()) to equal(second.clone())
        expect(commit.from.clone()) to equal(Some(first.clone()))
        expect(commit.body.as_str()) to equal("The second line is louder now.\n\nAnd there is a fourth.")

        let shortened = changes(&scene, project, &second[..7], None).await.unwrap();
        expect(shortened.commit.clone()) to equal(second.clone())
        expect(changed(&shortened)) to equal(changed(&commit))
    }

    // requirement: a commit without a parent added everything it holds
    story a_first_commit_added_everything {
        let scene = Scene::new();
        let (folder, first) = repository();
        let project = scene.project(&folder).id;

        let commit = changes(&scene, project, &first, None).await.unwrap();
        expect(changed(&commit)) to equal(names(&["A README.md +1 -0", "A src/app.txt +3 -0"]))
        expect(commit.from.clone()) to equal(None)
        expect(commit.body.as_str()) to equal("")

        let file = change(&scene, project, &first, None, "src/app.txt").await.unwrap();
        expect(shown(&file)) to equal(names(&["+1 one", "+2 two", "+3 three"]))
    }

    // requirement: a merge is compared with the branch that was merged into, so it shows what it brought in
    story a_merge_shows_what_it_brought_in {
        let scene = Scene::new();
        let folder = branched();
        let project = scene.project(&folder).id;
        let merge = git(folder.path(), &["rev-parse", "HEAD"]);
        let continued = git(folder.path(), &["rev-parse", "HEAD^1"]);

        let commit = changes(&scene, project, &merge, None).await.unwrap();
        expect(changed(&commit)) to equal(names(&["M README.md +1 -1"]))
        expect(commit.from.clone()) to equal(Some(continued.clone()))

        let file = change(&scene, project, &merge, None, "README.md").await.unwrap();
        expect(shown(&file)) to equal(names(&["- # Sample", "+1 # Sample, moved on"]))
    }

    // requirement: any two commits can be compared in place of a commit and the one before it
    story two_commits_are_compared {
        let scene = Scene::new();
        let (folder, first) = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        record(folder.path(), 1, "Change the app");
        put(folder.path(), "notes.txt", "notes\n");
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        let third = record(folder.path(), 2, "Add notes");

        let compared = changes(&scene, project, &third, Some(&first)).await.unwrap();
        expect(changed(&compared)) to equal(names(&["A notes.txt +1 -0", "M src/app.txt +2 -1"]))
        expect(compared.commit.clone()) to equal(third.clone())
        expect(compared.from.clone()) to equal(Some(first.clone()))

        let file = change(&scene, project, &third, Some(&first), "src/app.txt").await.unwrap();
        expect(shown(&file)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three", "+4 four"]))
        let alone = change(&scene, project, &third, None, "src/app.txt").await.unwrap();
        expect(shown(&alone)) to equal(names(&["1 one", "2 TWO", "3 three", "+4 four"]))

        let backwards = changes(&scene, project, &first, Some(&third)).await.unwrap();
        expect(changed(&backwards)) to equal(names(&["D notes.txt +0 -1", "M src/app.txt +1 -2"]))
        let same = changes(&scene, project, &third, Some(&third)).await.unwrap();
        expect(changed(&same)) to equal(names(&[]))
    }

    // requirement: a file a commit changed is shown line by line, with what was removed where it was and what was added marked
    story a_changed_file_shows_what_changed {
        let scene = Scene::new();
        let (folder, _) = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "notes.txt", "hello\n");
        put(folder.path(), "picture.bin", [137, 80, 78, 71, 0, 1, 2]);
        put(folder.path(), "too-large.txt", "a".repeat(1024 * 1024 + 1));
        git(folder.path(), &["rm", "-q", "README.md"]);
        let second = record(folder.path(), 1, "Change the app");
        put(folder.path(), "src/app.txt", "not committed\n");

        let modified = change(&scene, project, &second, None, "src/app.txt").await;
        expect(modified.is_ok()) to be_true
        let modified = modified.unwrap();
        expect(modified.path.as_str()) to equal("src/app.txt")
        expect(kind(&modified)) to equal("text")
        expect(shown(&modified)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three", "+4 four"]))
        let added = change(&scene, project, &second, None, "notes.txt").await.unwrap();
        expect(shown(&added)) to equal(names(&["+1 hello"]))
        let deleted = change(&scene, project, &second, None, "README.md").await.unwrap();
        expect(shown(&deleted)) to equal(names(&["- # Sample"]))
        let picture = change(&scene, project, &second, None, "picture.bin").await.unwrap();
        expect(kind(&picture)) to equal("binary")
        let too_large = change(&scene, project, &second, None, "too-large.txt").await.unwrap();
        expect(kind(&too_large)) to equal("too_large")
    }

    // requirement: a commit that does not exist, or a name that is no commit's, is refused
    story an_unknown_commit_is_refused {
        let scene = Scene::new();
        let (folder, first) = repository();
        let project = scene.project(&folder).id;
        let parent = format!("{first}^");
        let no_commit = refusal("There is no such commit.");

        expect(changes(&scene, project, MISSING_COMMIT, None).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, "main", None).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, "HEAD", None).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, "--all", None).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, "", None).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, &parent, None).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, &first, Some(MISSING_COMMIT)).await.err()) to equal(no_commit.clone())
        expect(changes(&scene, project, &first, Some("main")).await.err()) to equal(no_commit.clone())

        expect(change(&scene, project, MISSING_COMMIT, None, "README.md").await.err()) to equal(no_commit.clone())
        expect(change(&scene, project, &first, Some(MISSING_COMMIT), "README.md").await.err()) to equal(no_commit.clone())
    }

    // requirement: only files of the repository can be read through a commit: nothing outside it, nothing of git's own
    story a_path_that_leads_elsewhere_is_refused {
        let scene = Scene::new();
        let (folder, first) = repository();
        let project = scene.project(&folder).id;
        let above = format!("../{}/README.md", folder.path().file_name().unwrap().to_str().unwrap());
        let whole = folder.path().canonicalize().unwrap().join("README.md");
        let no_file = refusal("There is no such file.");

        expect(change(&scene, project, &first, None, "nothing.txt").await.err()) to equal(no_file.clone())
        expect(change(&scene, project, &first, None, "src").await.err()) to equal(no_file.clone())
        expect(change(&scene, project, &first, None, "").await.err()) to equal(no_file.clone())
        expect(change(&scene, project, &first, None, &above).await.err()) to equal(no_file.clone())
        expect(change(&scene, project, &first, None, "src/../README.md").await.err()) to equal(no_file.clone())
        expect(change(&scene, project, &first, None, whole.to_str().unwrap()).await.err()) to equal(no_file.clone())
        expect(change(&scene, project, &first, None, ".git/config").await.err()) to equal(no_file.clone())
    }
}
