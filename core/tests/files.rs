//! Specs for the file browser.
//!
//! Every project here is a real git repository made for the test, because the
//! answers come from the `git` program. The repositories ignore the settings
//! of whoever runs the tests.

mod common;

use std::fs::remove_file;

use lets_expect::lets_expect;
use tempfile::{tempdir, TempDir};
use uuid::Uuid;

use common::{commit, git, project_folder, put, unknown, Scene};
use teryon_core::files::diff::LineChange;
use teryon_core::files::{Content, FileChange, FileView, Listing};

/// What the screen is told when the project's directory is not a git repository.
const NOT_A_REPOSITORY: &str =
    "This project's directory is not a git repository, so there are no files or history to show.";

/// A commit name that no repository here has.
const MISSING_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A project folder holding a repository with one commit on `main`.
fn repository() -> TempDir {
    let folder = project_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), ".gitignore", "secret.env\n");
    put(folder.path(), "README.md", "# Sample\n");
    put(folder.path(), "src/app.txt", "one\ntwo\nthree\n");
    put(folder.path(), "src/old.txt", "old\n");
    commit(folder.path());
    folder
}

/// The files of a project, with a refusal as its message.
async fn files(
    scene: &Scene,
    project: Uuid,
    branch: Option<&str>,
    commit: Option<&str>,
) -> Result<Listing, String> {
    scene
        .workspace
        .files(project, branch, commit)
        .await
        .map_err(|problem| problem.to_string())
}

/// One file of a project, with a refusal as its message.
async fn file(
    scene: &Scene,
    project: Uuid,
    branch: Option<&str>,
    commit: Option<&str>,
    path: &str,
) -> Result<FileView, String> {
    scene
        .workspace
        .file(project, branch, commit, path)
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

/// The files of a listing, each after the letter of its change: `M src/app.txt`.
fn listed(listing: &Listing) -> Vec<String> {
    listing
        .files
        .iter()
        .map(|file| {
            let letter = match file.change {
                Some(FileChange::Added) => "A",
                Some(FileChange::Modified) => "M",
                Some(FileChange::Deleted) => "D",
                None => " ",
            };
            format!("{letter} {}", file.path)
        })
        .collect()
}

/// Only the changed files of a listing.
fn changed(listing: &Listing) -> Vec<String> {
    let mut changed = listed(listing);
    changed.retain(|line| !line.starts_with(' '));
    changed
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
    // requirement: the files of the project's directory are listed, each with its place in the repository
    story the_project_directory_is_listed {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;

        let listing = files(&scene, project, None, None).await;
        expect(listing.is_ok()) to be_true
        let listing = listing.unwrap();
        expect(listed(&listing)) to equal(names(&["  .gitignore", "  README.md", "  src/app.txt", "  src/old.txt"]))
        expect(listing.branches.clone()) to equal(names(&["main"]))
        expect(listing.branch.clone()) to equal(branch_name("main"))
        expect(listing.base.clone()) to equal(None)
    }

    // requirement: on the main branch, the changes are the work that is not committed yet
    // requirement: a file git ignores is not listed
    story work_that_is_not_committed_is_marked {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        put(folder.path(), "secret.env", "TOKEN=1\n");
        remove_file(folder.path().join("src/old.txt")).unwrap();

        let listing = files(&scene, project, None, None).await.unwrap();
        expect(listed(&listing)) to equal(names(&[
            "  .gitignore",
            "  README.md",
            "A notes/new file \u{17c}.txt",
            "M src/app.txt",
            "D src/old.txt",
        ]))

        git(folder.path(), &["rm", "-q", "README.md"]);
        git(folder.path(), &["add", "notes"]);
        let recorded = files(&scene, project, None, None).await.unwrap();
        expect(changed(&recorded)) to equal(names(&[
            "D README.md",
            "A notes/new file \u{17c}.txt",
            "M src/app.txt",
            "D src/old.txt",
        ]))
    }

    // requirement: on any other branch, the changes are counted from where it left the main branch
    story a_branch_in_the_project_directory_is_compared_with_main {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["switch", "-q", "-c", "feature/files"]);
        put(folder.path(), "src/app.txt", "one\ntwo\nthree\nfour\n");
        commit(folder.path());
        put(folder.path(), "draft.txt", "draft\n");

        let listing = files(&scene, project, None, None).await.unwrap();
        expect(changed(&listing)) to equal(names(&["A draft.txt", "M src/app.txt"]))
        expect(listing.branches.clone()) to equal(names(&["feature/files", "main"]))
        expect(listing.branch.clone()) to equal(branch_name("feature/files"))
        expect(listing.base.clone()) to equal(branch_name("main"))

        let named = files(&scene, project, Some("feature/files"), None).await.unwrap();
        expect(changed(&named)) to equal(names(&["A draft.txt", "M src/app.txt"]))

        let main = files(&scene, project, Some("main"), None).await.unwrap();
        expect(changed(&main)) to equal(names(&[]))
        expect(main.branch.clone()) to equal(branch_name("main"))
        expect(main.base.clone()) to equal(None)
    }

    // requirement: a branch checked out in another directory is shown from that directory, with the work not committed there
    story a_branch_checked_out_elsewhere_shows_its_uncommitted_work {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        let elsewhere = project_folder();
        let checkout = elsewhere.path().join("checkout");
        git(folder.path(), &["worktree", "add", "-q", "-b", "feature", checkout.to_str().unwrap()]);
        put(&checkout, "src/app.txt", "one\ntwo\n");
        put(&checkout, "draft.txt", "draft\n");

        let listing = files(&scene, project, Some("feature"), None).await.unwrap();
        expect(changed(&listing)) to equal(names(&["A draft.txt", "M src/app.txt"]))
        expect(listing.branch.clone()) to equal(branch_name("feature"))
        expect(listing.base.clone()) to equal(branch_name("main"))

        let shown_file = file(&scene, project, Some("feature"), None, "src/app.txt").await.unwrap();
        expect(shown(&shown_file)) to equal(names(&["1 one", "2 two", "- three"]))

        let own = files(&scene, project, None, None).await.unwrap();
        expect(changed(&own)) to equal(names(&[]))
    }

    // requirement: a branch checked out nowhere is shown as it was committed
    story a_branch_checked_out_nowhere_is_shown_as_committed {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        put(folder.path(), "added.txt", "added\n");
        git(folder.path(), &["rm", "-q", "src/old.txt"]);
        commit(folder.path());
        git(folder.path(), &["switch", "-q", "main"]);
        put(folder.path(), "README.md", "# Sample, moved on\n");
        commit(folder.path());

        let listing = files(&scene, project, Some("feature"), None).await.unwrap();
        expect(listed(&listing)) to equal(names(&[
            "  .gitignore",
            "  README.md",
            "A added.txt",
            "M src/app.txt",
            "D src/old.txt",
        ]))
        expect(listing.base.clone()) to equal(branch_name("main"))

        let changed_file = file(&scene, project, Some("feature"), None, "src/app.txt").await.unwrap();
        expect(shown(&changed_file)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three"]))
        let untouched = file(&scene, project, Some("feature"), None, "README.md").await.unwrap();
        expect(shown(&untouched)) to equal(names(&["1 # Sample"]))
        let removed = file(&scene, project, Some("feature"), None, "src/old.txt").await.unwrap();
        expect(shown(&removed)) to equal(names(&["- old"]))
        let missing = file(&scene, project, Some("feature"), None, "nothing.txt").await;
        expect(missing.err()) to equal(refusal("There is no such file."))
    }

    // requirement: a branch checked out in a directory outside the project is read from that directory
    story a_checkout_outside_the_project_is_read {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        let outside = tempdir().unwrap();
        let checkout = outside.path().join("checkout");
        git(folder.path(), &["worktree", "add", "-q", "-b", "feature", checkout.to_str().unwrap()]);
        put(&checkout, "src/app.txt", "changed outside the project\n");
        put(&checkout, "draft.txt", "draft\n");

        let listing = files(&scene, project, Some("feature"), None).await;
        expect(listing.is_ok()) to be_true
        expect(changed(&listing.unwrap())) to equal(names(&["A draft.txt", "M src/app.txt"]))

        let shown_file = file(&scene, project, Some("feature"), None, "src/app.txt").await.unwrap();
        expect(shown(&shown_file)) to equal(names(&["- one", "- two", "- three", "+1 changed outside the project"]))
        let draft = file(&scene, project, Some("feature"), None, "draft.txt").await.unwrap();
        expect(shown(&draft)) to equal(names(&["+1 draft"]))
    }

    // requirement: a directory that is not on a branch is still shown, compared with the main branch
    story a_directory_off_any_branch_is_listed {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["checkout", "-q", "--detach"]);
        put(folder.path(), "src/app.txt", "one\n");

        let listing = files(&scene, project, None, None).await.unwrap();
        expect(changed(&listing)) to equal(names(&["M src/app.txt"]))
        expect(listing.branch.clone()) to equal(None)
        expect(listing.base.clone()) to equal(branch_name("main"))
        expect(listing.branches.clone()) to equal(names(&["main"]))
    }

    // requirement: without a main branch, the only changes are the ones not committed yet
    story a_repository_without_a_main_branch_shows_uncommitted_work_only {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["branch", "-q", "-m", "trunk"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\n");
        commit(folder.path());
        put(folder.path(), "draft.txt", "draft\n");

        let listing = files(&scene, project, None, None).await.unwrap();
        expect(changed(&listing)) to equal(names(&["A draft.txt"]))
        expect(listing.base.clone()) to equal(None)
        expect(listing.branches.clone()) to equal(names(&["feature", "trunk"]))
    }

    // requirement: `master` counts as the main branch when there is no `main`
    story master_is_the_main_branch_of_an_older_repository {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        git(folder.path(), &["branch", "-q", "-m", "master"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\n");
        commit(folder.path());

        let listing = files(&scene, project, None, None).await.unwrap();
        expect(changed(&listing)) to equal(names(&["M src/app.txt"]))
        expect(listing.base.clone()) to equal(branch_name("master"))
    }

    // requirement: in a repository without a commit, every file is new
    story a_repository_without_a_commit_lists_everything_as_added {
        let scene = Scene::new();
        let folder = project_folder();
        git(folder.path(), &["init", "-q", "-b", "main"]);
        put(folder.path(), "recorded.txt", "one\n");
        git(folder.path(), &["add", "recorded.txt"]);
        put(folder.path(), "loose.txt", "two\n");
        let project = scene.project(&folder).id;

        let listing = files(&scene, project, None, None).await.unwrap();
        expect(listed(&listing)) to equal(names(&["A loose.txt", "A recorded.txt"]))
        expect(listing.branches.clone()) to equal(names(&["main"]))
        expect(listing.branch.clone()) to equal(branch_name("main"))
        expect(listing.base.clone()) to equal(None)

        let shown_file = file(&scene, project, None, None, "recorded.txt").await.unwrap();
        expect(shown(&shown_file)) to equal(names(&["+1 one"]))
        let named = files(&scene, project, Some("main"), None).await.unwrap();
        expect(listed(&named)) to equal(names(&["A loose.txt", "A recorded.txt"]))
    }

    // requirement: a commit can be shown in place of a branch, as it was committed and compared with the main branch
    story a_commit_is_shown_as_it_was_committed {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        let first = git(folder.path(), &["rev-parse", "HEAD"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        commit(folder.path());
        let second = git(folder.path(), &["rev-parse", "HEAD"]);
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "added.txt", "added\n");
        commit(folder.path());
        put(folder.path(), "draft.txt", "not committed\n");

        let listing = files(&scene, project, None, Some(&second)).await;
        expect(listing.is_ok()) to be_true
        let listing = listing.unwrap();
        expect(listed(&listing)) to equal(names(&["  .gitignore", "  README.md", "M src/app.txt", "  src/old.txt"]))
        expect(listing.commit.clone()) to equal(Some(second.clone()))
        expect(listing.branch.clone()) to equal(None)
        expect(listing.base.clone()) to equal(branch_name("main"))
        expect(listing.branches.clone()) to equal(names(&["feature", "main"]))

        let shown_file = file(&scene, project, None, Some(&second), "src/app.txt").await.unwrap();
        expect(shown(&shown_file)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three"]))
        let later = file(&scene, project, None, Some(&second), "added.txt").await;
        expect(later.err()) to equal(refusal("There is no such file."))

        let shortened = files(&scene, project, None, Some(&second[..7])).await.unwrap();
        expect(shortened.commit.clone()) to equal(Some(second.clone()))
        let over_a_branch = files(&scene, project, Some("main"), Some(&second)).await.unwrap();
        expect(over_a_branch.commit.clone()) to equal(Some(second.clone()))

        let on_main = files(&scene, project, None, Some(&first)).await.unwrap();
        expect(changed(&on_main)) to equal(names(&[]))
        expect(on_main.base.clone()) to equal(branch_name("main"))
        let directory = files(&scene, project, None, None).await.unwrap();
        expect(directory.commit.clone()) to equal(None)
    }

    // requirement: a commit that does not exist, or a name that is no commit's, is refused
    story an_unknown_commit_is_refused {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        let no_commit = refusal("There is no such commit.");

        expect(files(&scene, project, None, Some(MISSING_COMMIT)).await.err()) to equal(no_commit.clone())
        expect(files(&scene, project, None, Some("main")).await.err()) to equal(no_commit.clone())
        expect(files(&scene, project, None, Some("HEAD")).await.err()) to equal(no_commit.clone())
        expect(files(&scene, project, None, Some("--all")).await.err()) to equal(no_commit.clone())
        expect(files(&scene, project, None, Some("")).await.err()) to equal(no_commit.clone())

        let shown_file = file(&scene, project, None, Some("0123456789abcdef"), "README.md").await;
        expect(shown_file.err()) to equal(no_commit.clone())
    }

    // requirement: a branch that does not exist is refused
    story an_unknown_branch_is_refused {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        let no_branch = refusal("There is no such branch.");

        expect(files(&scene, project, Some("nothing"), None).await.err()) to equal(no_branch.clone())
        expect(files(&scene, project, Some("--all"), None).await.err()) to equal(no_branch.clone())
        let shown_file = file(&scene, project, Some("nothing"), None, "README.md").await;
        expect(shown_file.err()) to equal(no_branch.clone())
    }

    // requirement: a project whose directory is not a git repository has no file browser
    //
    // The project's directory here sits inside another repository, which must
    // not be mistaken for the project's.
    story a_directory_that_is_not_a_repository_is_refused {
        let scene = Scene::new();
        let around = repository();
        let directory = around.path().join("inside");
        put(&directory, "README.md", "# Sample\n");
        let project = scene.workspace.add_project("Sample", &directory.to_string_lossy()).unwrap().id;

        expect(files(&scene, project, None, None).await.err()) to equal(refusal(NOT_A_REPOSITORY))
        expect(file(&scene, project, None, None, "README.md").await.err()) to equal(refusal(NOT_A_REPOSITORY))
    }

    // requirement: only the files of a project Teryon knows are shown
    story an_unknown_project_is_refused {
        let scene = Scene::new();
        let no_project = refusal("There is no such project.");

        expect(files(&scene, unknown(), None, None).await.err()) to equal(no_project.clone())
        expect(file(&scene, unknown(), None, None, "README.md").await.err()) to equal(no_project.clone())
    }

    // requirement: a file is shown line by line, each line with its number
    story an_unchanged_file_is_shown {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;

        let view = file(&scene, project, None, None, "src/app.txt").await;
        expect(view.is_ok()) to be_true
        let view = view.unwrap();
        expect(view.path.as_str()) to equal("src/app.txt")
        expect(kind(&view)) to equal("text")
        expect(shown(&view)) to equal(names(&["1 one", "2 two", "3 three"]))
    }

    // requirement: a changed file shows its removed lines where they were and marks the added ones
    story a_changed_file_shows_what_changed {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        remove_file(folder.path().join("src/old.txt")).unwrap();

        let modified = file(&scene, project, None, None, "src/app.txt").await.unwrap();
        expect(shown(&modified)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three", "+4 four"]))
        let added = file(&scene, project, None, None, "notes/new file \u{17c}.txt").await.unwrap();
        expect(shown(&added)) to equal(names(&["+1 hello"]))
        let deleted = file(&scene, project, None, None, "src/old.txt").await.unwrap();
        expect(shown(&deleted)) to equal(names(&["- old"]))
    }

    // requirement: a file that is not text, or is larger than a mebibyte, is named as such instead of shown
    story a_file_that_cannot_be_shown_says_why {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "picture.bin", [137, 80, 78, 71, 0, 1, 2]);
        put(folder.path(), "largest.txt", "a".repeat(1024 * 1024));
        put(folder.path(), "too-large.txt", "a".repeat(1024 * 1024 + 1));
        put(folder.path(), "empty.txt", "");

        let picture = file(&scene, project, None, None, "picture.bin").await;
        expect(picture.is_ok()) to be_true
        expect(kind(&picture.unwrap())) to equal("binary")
        let largest = file(&scene, project, None, None, "largest.txt").await.unwrap();
        expect(kind(&largest)) to equal("text")
        let too_large = file(&scene, project, None, None, "too-large.txt").await.unwrap();
        expect(kind(&too_large)) to equal("too_large")
        let empty = file(&scene, project, None, None, "empty.txt").await.unwrap();
        expect(kind(&empty)) to equal("text")
        expect(shown(&empty)) to equal(names(&[]))

        commit(folder.path());
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        git(folder.path(), &["switch", "-q", "main"]);
        let committed = file(&scene, project, Some("feature"), None, "too-large.txt").await.unwrap();
        expect(kind(&committed)) to equal("too_large")
        let committed_picture = file(&scene, project, Some("feature"), None, "picture.bin").await.unwrap();
        expect(kind(&committed_picture)) to equal("binary")
    }

    // requirement: only files of the repository can be read: nothing outside it, nothing git ignores, nothing of git's own
    story a_path_that_leads_elsewhere_is_refused {
        let scene = Scene::new();
        let folder = repository();
        let project = scene.project(&folder).id;
        put(folder.path(), "secret.env", "TOKEN=1\n");
        let above = format!("../{}/README.md", folder.path().file_name().unwrap().to_str().unwrap());
        let whole = folder.path().canonicalize().unwrap().join("README.md");
        let no_file = refusal("There is no such file.");

        expect(file(&scene, project, None, None, "secret.env").await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, "nothing.txt").await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, "src").await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, "").await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, &above).await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, "src/../README.md").await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, whole.to_str().unwrap()).await.err()) to equal(no_file.clone())
        expect(file(&scene, project, None, None, ".git/config").await.err()) to equal(no_file.clone())
    }
}

/// Links in a project's directory, which only Unix-like systems make without special rights.
#[cfg(unix)]
mod links {
    use std::os::unix::fs::symlink;

    use super::*;

    lets_expect! { #tokio_test
        // requirement: only files of the repository can be read: nothing outside it, nothing git ignores, nothing of git's own
        story a_link_that_leads_elsewhere_is_refused {
            let scene = Scene::new();
            let folder = repository();
            let project = scene.project(&folder).id;
            let outside = tempdir().unwrap();
            put(outside.path(), "private.txt", "private\n");
            put(folder.path(), "secret.env", "TOKEN=1\n");
            symlink(outside.path().join("private.txt"), folder.path().join("way-out.txt")).unwrap();
            symlink(folder.path().join("secret.env"), folder.path().join("way-to-secret.txt")).unwrap();
            symlink(folder.path().join(".git/config"), folder.path().join("way-to-settings.txt")).unwrap();
            symlink(folder.path().join("README.md"), folder.path().join("way-inside.txt")).unwrap();
            let no_file = refusal("There is no such file.");

            expect(file(&scene, project, None, None, "way-out.txt").await.err()) to equal(no_file.clone())
            expect(file(&scene, project, None, None, "way-to-secret.txt").await.err()) to equal(no_file.clone())
            expect(file(&scene, project, None, None, "way-to-settings.txt").await.err()) to equal(no_file.clone())

            let inside = file(&scene, project, None, None, "way-inside.txt").await.unwrap();
            expect(shown(&inside)) to equal(names(&["+1 # Sample"]))
        }
    }
}
