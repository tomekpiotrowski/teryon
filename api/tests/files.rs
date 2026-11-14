//! Request specs for the file browser.
//!
//! Every project here is a real git repository made for the test, because the
//! answers come from the `git` program. The repositories ignore the settings
//! of whoever runs the tests.

mod common;

use std::fs::remove_file;
use std::os::unix::fs::symlink;

use axum::http::StatusCode;
use axum_test::TestResponse;
use lets_expect::lets_expect;
use serde_json::Value;
use tempfile::{tempdir, TempDir};

use common::{commit, git, project_folder, put, scene, text_field, Scene};
use teryon::workspace::project;

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

async fn files(scene: &Scene, project: &project::Model, branch: &[(&str, &str)]) -> TestResponse {
    scene
        .get(&format!("/api/projects/{}/files", project.id), branch)
        .await
}

async fn file(scene: &Scene, project: &project::Model, query: &[(&str, &str)]) -> TestResponse {
    scene
        .get(&format!("/api/projects/{}/file", project.id), query)
        .await
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// The files of a listing, each after the letter of its change: `M src/app.txt`.
fn listed(response: &TestResponse) -> Vec<String> {
    let files = response.json::<Value>()["files"].clone();
    let mut listed = Vec::new();
    for file in files.as_array().expect("a list of files") {
        let letter = match file["change"].as_str() {
            Some("added") => "A",
            Some("modified") => "M",
            Some("deleted") => "D",
            _ => " ",
        };
        listed.push(format!(
            "{letter} {}",
            file["path"].as_str().unwrap_or_default()
        ));
    }
    listed
}

/// Only the changed files of a listing.
fn changed(response: &TestResponse) -> Vec<String> {
    let mut changed = listed(response);
    changed.retain(|line| !line.starts_with(' '));
    changed
}

fn branches(response: &TestResponse) -> Vec<String> {
    let branches = response.json::<Value>()["branches"].clone();
    let mut names = Vec::new();
    for branch in branches.as_array().expect("a list of branches") {
        names.push(branch.as_str().unwrap_or_default().to_string());
    }
    names
}

/// A field of an answer as text, where a missing value reads `null`.
fn field(response: &TestResponse, name: &str) -> String {
    match &response.json::<Value>()[name] {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn kind(response: &TestResponse) -> String {
    response.json::<Value>()["content"]["kind"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The lines of a file: `2 two` unchanged, `+2 TWO` added, `- two` removed.
fn shown(response: &TestResponse) -> Vec<String> {
    let lines = response.json::<Value>()["content"]["lines"].clone();
    let mut shown = Vec::new();
    for line in lines.as_array().expect("lines") {
        let mark = match line["change"].as_str() {
            Some("added") => "+",
            Some("removed") => "-",
            _ => "",
        };
        let number = line["number"].as_u64().map(|number| number.to_string());
        shown.push(format!(
            "{mark}{} {}",
            number.unwrap_or_default(),
            line["text"].as_str().unwrap_or_default()
        ));
    }
    shown
}

lets_expect! { #tokio_test
    // requirement: the files of the project's directory are listed, each with its place in the repository
    story the_project_directory_is_listed {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;

        let response = files(&scene, &project, &[]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(listed(&response)) to equal(names(&["  .gitignore", "  README.md", "  src/app.txt", "  src/old.txt"]))
        expect(branches(&response)) to equal(names(&["main"]))
        expect(field(&response, "branch")) to equal("main".to_string())
        expect(field(&response, "base")) to equal("null".to_string())
    }

    // requirement: on the main branch, the changes are the work that is not committed yet
    // requirement: a file git ignores is not listed
    story work_that_is_not_committed_is_marked {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        put(folder.path(), "secret.env", "TOKEN=1\n");
        remove_file(folder.path().join("src/old.txt")).unwrap();

        let response = files(&scene, &project, &[]).await;
        expect(listed(&response)) to equal(names(&[
            "  .gitignore",
            "  README.md",
            "A notes/new file \u{17c}.txt",
            "M src/app.txt",
            "D src/old.txt",
        ]))

        git(folder.path(), &["rm", "-q", "README.md"]);
        git(folder.path(), &["add", "notes"]);
        let recorded = files(&scene, &project, &[]).await;
        expect(changed(&recorded)) to equal(names(&[
            "D README.md",
            "A notes/new file \u{17c}.txt",
            "M src/app.txt",
            "D src/old.txt",
        ]))
    }

    // requirement: on any other branch, the changes are counted from where it left the main branch
    story a_branch_in_the_project_directory_is_compared_with_main {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["switch", "-q", "-c", "feature/files"]);
        put(folder.path(), "src/app.txt", "one\ntwo\nthree\nfour\n");
        commit(folder.path());
        put(folder.path(), "draft.txt", "draft\n");

        let response = files(&scene, &project, &[]).await;
        expect(changed(&response)) to equal(names(&["A draft.txt", "M src/app.txt"]))
        expect(branches(&response)) to equal(names(&["feature/files", "main"]))
        expect(field(&response, "branch")) to equal("feature/files".to_string())
        expect(field(&response, "base")) to equal("main".to_string())

        let named = files(&scene, &project, &[("branch", "feature/files")]).await;
        expect(changed(&named)) to equal(names(&["A draft.txt", "M src/app.txt"]))

        let main = files(&scene, &project, &[("branch", "main")]).await;
        expect(changed(&main)) to equal(names(&[]))
        expect(field(&main, "branch")) to equal("main".to_string())
        expect(field(&main, "base")) to equal("null".to_string())
    }

    // requirement: a branch checked out in another directory is shown from that directory, with the work not committed there
    story a_branch_checked_out_elsewhere_shows_its_uncommitted_work {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        let elsewhere = project_folder();
        let checkout = elsewhere.path().join("checkout");
        git(folder.path(), &["worktree", "add", "-q", "-b", "feature", checkout.to_str().unwrap()]);
        put(&checkout, "src/app.txt", "one\ntwo\n");
        put(&checkout, "draft.txt", "draft\n");

        let response = files(&scene, &project, &[("branch", "feature")]).await;
        expect(changed(&response)) to equal(names(&["A draft.txt", "M src/app.txt"]))
        expect(field(&response, "branch")) to equal("feature".to_string())
        expect(field(&response, "base")) to equal("main".to_string())

        let shown_file = file(&scene, &project, &[("branch", "feature"), ("path", "src/app.txt")]).await;
        expect(shown(&shown_file)) to equal(names(&["1 one", "2 two", "- three"]))

        let own = files(&scene, &project, &[]).await;
        expect(changed(&own)) to equal(names(&[]))
    }

    // requirement: a branch checked out nowhere is shown as it was committed
    story a_branch_checked_out_nowhere_is_shown_as_committed {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        put(folder.path(), "added.txt", "added\n");
        git(folder.path(), &["rm", "-q", "src/old.txt"]);
        commit(folder.path());
        git(folder.path(), &["switch", "-q", "main"]);
        put(folder.path(), "README.md", "# Sample, moved on\n");
        commit(folder.path());

        let response = files(&scene, &project, &[("branch", "feature")]).await;
        expect(listed(&response)) to equal(names(&[
            "  .gitignore",
            "  README.md",
            "A added.txt",
            "M src/app.txt",
            "D src/old.txt",
        ]))
        expect(field(&response, "base")) to equal("main".to_string())

        let changed_file = file(&scene, &project, &[("branch", "feature"), ("path", "src/app.txt")]).await;
        expect(shown(&changed_file)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three"]))
        let untouched = file(&scene, &project, &[("branch", "feature"), ("path", "README.md")]).await;
        expect(shown(&untouched)) to equal(names(&["1 # Sample"]))
        let removed = file(&scene, &project, &[("branch", "feature"), ("path", "src/old.txt")]).await;
        expect(shown(&removed)) to equal(names(&["- old"]))
        let missing = file(&scene, &project, &[("branch", "feature"), ("path", "nothing.txt")]).await;
        expect(missing.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    // requirement: a directory outside the allowed folders is never read, even when a branch is checked out there
    story a_checkout_outside_the_allowed_folders_is_not_read {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        let outside = tempdir().unwrap();
        let checkout = outside.path().join("checkout");
        git(folder.path(), &["worktree", "add", "-q", "-b", "feature", checkout.to_str().unwrap()]);
        put(&checkout, "src/app.txt", "changed where Teryon may not look\n");
        put(&checkout, "draft.txt", "draft\n");

        let response = files(&scene, &project, &[("branch", "feature")]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(changed(&response)) to equal(names(&[]))

        let shown_file = file(&scene, &project, &[("branch", "feature"), ("path", "src/app.txt")]).await;
        expect(shown(&shown_file)) to equal(names(&["1 one", "2 two", "3 three"]))
        let draft = file(&scene, &project, &[("branch", "feature"), ("path", "draft.txt")]).await;
        expect(draft.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    // requirement: a directory that is not on a branch is still shown, compared with the main branch
    story a_directory_off_any_branch_is_listed {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["checkout", "-q", "--detach"]);
        put(folder.path(), "src/app.txt", "one\n");

        let response = files(&scene, &project, &[]).await;
        expect(changed(&response)) to equal(names(&["M src/app.txt"]))
        expect(field(&response, "branch")) to equal("null".to_string())
        expect(field(&response, "base")) to equal("main".to_string())
        expect(branches(&response)) to equal(names(&["main"]))
    }

    // requirement: without a main branch, the only changes are the ones not committed yet
    story a_repository_without_a_main_branch_shows_uncommitted_work_only {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["branch", "-q", "-m", "trunk"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\n");
        commit(folder.path());
        put(folder.path(), "draft.txt", "draft\n");

        let response = files(&scene, &project, &[]).await;
        expect(changed(&response)) to equal(names(&["A draft.txt"]))
        expect(field(&response, "base")) to equal("null".to_string())
        expect(branches(&response)) to equal(names(&["feature", "trunk"]))
    }

    // requirement: `master` counts as the main branch when there is no `main`
    story master_is_the_main_branch_of_an_older_repository {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["branch", "-q", "-m", "master"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\n");
        commit(folder.path());

        let response = files(&scene, &project, &[]).await;
        expect(changed(&response)) to equal(names(&["M src/app.txt"]))
        expect(field(&response, "base")) to equal("master".to_string())
    }

    // requirement: in a repository without a commit, every file is new
    story a_repository_without_a_commit_lists_everything_as_added {
        let scene = scene().await;
        let folder = project_folder();
        git(folder.path(), &["init", "-q", "-b", "main"]);
        put(folder.path(), "recorded.txt", "one\n");
        git(folder.path(), &["add", "recorded.txt"]);
        put(folder.path(), "loose.txt", "two\n");
        let project = scene.project(&folder).await;

        let response = files(&scene, &project, &[]).await;
        expect(listed(&response)) to equal(names(&["A loose.txt", "A recorded.txt"]))
        expect(branches(&response)) to equal(names(&["main"]))
        expect(field(&response, "branch")) to equal("main".to_string())
        expect(field(&response, "base")) to equal("null".to_string())

        let shown_file = file(&scene, &project, &[("path", "recorded.txt")]).await;
        expect(shown(&shown_file)) to equal(names(&["+1 one"]))
        let named = files(&scene, &project, &[("branch", "main")]).await;
        expect(listed(&named)) to equal(names(&["A loose.txt", "A recorded.txt"]))
    }

    // requirement: a commit can be shown in place of a branch, as it was committed and compared with the main branch
    story a_commit_is_shown_as_it_was_committed {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        let first = git(folder.path(), &["rev-parse", "HEAD"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        commit(folder.path());
        let second = git(folder.path(), &["rev-parse", "HEAD"]);
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "added.txt", "added\n");
        commit(folder.path());
        put(folder.path(), "draft.txt", "not committed\n");

        let response = files(&scene, &project, &[("commit", &second)]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(listed(&response)) to equal(names(&["  .gitignore", "  README.md", "M src/app.txt", "  src/old.txt"]))
        expect(field(&response, "commit")) to equal(second.clone())
        expect(field(&response, "branch")) to equal("null".to_string())
        expect(field(&response, "base")) to equal("main".to_string())
        expect(branches(&response)) to equal(names(&["feature", "main"]))

        let shown_file = file(&scene, &project, &[("commit", &second), ("path", "src/app.txt")]).await;
        expect(shown(&shown_file)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three"]))
        let later = file(&scene, &project, &[("commit", &second), ("path", "added.txt")]).await;
        expect(later.status_code()) to equal(StatusCode::NOT_FOUND)

        let shortened = files(&scene, &project, &[("commit", &second[..7])]).await;
        expect(field(&shortened, "commit")) to equal(second.clone())
        let over_a_branch = files(&scene, &project, &[("branch", "main"), ("commit", &second)]).await;
        expect(field(&over_a_branch, "commit")) to equal(second.clone())

        let on_main = files(&scene, &project, &[("commit", &first)]).await;
        expect(changed(&on_main)) to equal(names(&[]))
        expect(field(&on_main, "base")) to equal("main".to_string())
        let directory = files(&scene, &project, &[]).await;
        expect(field(&directory, "commit")) to equal("null".to_string())
    }

    // requirement: a commit that does not exist, or a name that is no commit's, is refused
    story an_unknown_commit_is_refused {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;

        let listing = files(&scene, &project, &[("commit", "0123456789abcdef0123456789abcdef01234567")]).await;
        expect(listing.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&listing, "error")) to equal("There is no such commit.".to_string())
        expect(files(&scene, &project, &[("commit", "main")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(files(&scene, &project, &[("commit", "HEAD")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(files(&scene, &project, &[("commit", "--all")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(files(&scene, &project, &[("commit", "")]).await.status_code()) to equal(StatusCode::NOT_FOUND)

        let shown_file = file(&scene, &project, &[("commit", "0123456789abcdef"), ("path", "README.md")]).await;
        expect(shown_file.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&shown_file, "error")) to equal("There is no such commit.".to_string())
    }

    // requirement: a branch that does not exist is refused
    story an_unknown_branch_is_refused {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;

        let listing = files(&scene, &project, &[("branch", "nothing")]).await;
        expect(listing.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&listing, "error")) to equal("There is no such branch.".to_string())

        let option = files(&scene, &project, &[("branch", "--all")]).await;
        expect(option.status_code()) to equal(StatusCode::NOT_FOUND)

        let shown_file = file(&scene, &project, &[("branch", "nothing"), ("path", "README.md")]).await;
        expect(shown_file.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&shown_file, "error")) to equal("There is no such branch.".to_string())
    }

    // requirement: a project whose directory is not a git repository has no file browser
    //
    // The folder made here sits inside Teryon's own repository, which must not
    // be mistaken for the project's.
    story a_directory_that_is_not_a_repository_is_refused {
        let scene = scene().await;
        let folder = project_folder();
        put(folder.path(), "README.md", "# Sample\n");
        let project = scene.project(&folder).await;
        let refusal = "This project's directory is not a git repository, so there are no files or history to show.".to_string();

        let listing = files(&scene, &project, &[]).await;
        expect(listing.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&listing, "error")) to equal(refusal.clone())

        let shown_file = file(&scene, &project, &[("path", "README.md")]).await;
        expect(shown_file.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&shown_file, "error")) to equal(refusal)
    }

    // requirement: a user sees only the files of their own projects
    story files_are_shown_to_the_owner_only {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        let stranger = scene.someone_else().await;
        let listing = format!("/api/projects/{}/files", project.id);
        let shown_file = format!("/api/projects/{}/file", project.id);

        expect(scene.get_as(&stranger, &listing, &[]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.get_as(&stranger, &shown_file, &[("path", "README.md")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.get_as("", &listing, &[]).await.status_code()) to equal(StatusCode::UNAUTHORIZED)
        expect(scene.get_as("", &shown_file, &[("path", "README.md")]).await.status_code()) to equal(StatusCode::UNAUTHORIZED)

        scene.delete(&format!("/api/projects/{}", project.id)).await;
        expect(scene.get(&listing, &[]).await.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    // requirement: a file is shown line by line, each line with its number
    story an_unchanged_file_is_shown {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;

        let response = file(&scene, &project, &[("path", "src/app.txt")]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(field(&response, "path")) to equal("src/app.txt".to_string())
        expect(kind(&response)) to equal("text".to_string())
        expect(shown(&response)) to equal(names(&["1 one", "2 two", "3 three"]))
    }

    // requirement: a changed file shows its removed lines where they were and marks the added ones
    story a_changed_file_shows_what_changed {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        remove_file(folder.path().join("src/old.txt")).unwrap();

        let modified = file(&scene, &project, &[("path", "src/app.txt")]).await;
        expect(shown(&modified)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three", "+4 four"]))
        let added = file(&scene, &project, &[("path", "notes/new file \u{17c}.txt")]).await;
        expect(shown(&added)) to equal(names(&["+1 hello"]))
        let deleted = file(&scene, &project, &[("path", "src/old.txt")]).await;
        expect(shown(&deleted)) to equal(names(&["- old"]))
    }

    // requirement: a file that is not text, or is larger than a mebibyte, is named as such instead of shown
    story a_file_that_cannot_be_shown_says_why {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "picture.bin", [137, 80, 78, 71, 0, 1, 2]);
        put(folder.path(), "largest.txt", "a".repeat(1024 * 1024));
        put(folder.path(), "too-large.txt", "a".repeat(1024 * 1024 + 1));
        put(folder.path(), "empty.txt", "");

        let picture = file(&scene, &project, &[("path", "picture.bin")]).await;
        expect(picture.status_code()) to equal(StatusCode::OK)
        expect(kind(&picture)) to equal("binary".to_string())
        let largest = file(&scene, &project, &[("path", "largest.txt")]).await;
        expect(kind(&largest)) to equal("text".to_string())
        let too_large = file(&scene, &project, &[("path", "too-large.txt")]).await;
        expect(kind(&too_large)) to equal("too_large".to_string())
        let empty = file(&scene, &project, &[("path", "empty.txt")]).await;
        expect(kind(&empty)) to equal("text".to_string())
        expect(shown(&empty)) to equal(names(&[]))

        commit(folder.path());
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        git(folder.path(), &["switch", "-q", "main"]);
        let committed = file(&scene, &project, &[("branch", "feature"), ("path", "too-large.txt")]).await;
        expect(kind(&committed)) to equal("too_large".to_string())
        let committed_picture = file(&scene, &project, &[("branch", "feature"), ("path", "picture.bin")]).await;
        expect(kind(&committed_picture)) to equal("binary".to_string())
    }

    // requirement: only files of the repository can be read: nothing outside it, nothing git ignores, nothing of git's own
    story a_path_that_leads_elsewhere_is_refused {
        let scene = scene().await;
        let folder = repository();
        let project = scene.project(&folder).await;
        let outside = tempdir().unwrap();
        put(outside.path(), "private.txt", "private\n");
        put(folder.path(), "secret.env", "TOKEN=1\n");
        symlink(outside.path().join("private.txt"), folder.path().join("way-out.txt")).unwrap();
        symlink(folder.path().join("secret.env"), folder.path().join("way-to-secret.txt")).unwrap();
        symlink(folder.path().join(".git/config"), folder.path().join("way-to-settings.txt")).unwrap();
        symlink(folder.path().join("README.md"), folder.path().join("way-inside.txt")).unwrap();
        let above = format!("../{}/README.md", folder.path().file_name().unwrap().to_str().unwrap());
        let whole = folder.path().canonicalize().unwrap().join("README.md");

        expect(file(&scene, &project, &[("path", "secret.env")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "nothing.txt")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "src")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", &above)]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "src/../README.md")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", whole.to_str().unwrap())]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", ".git/config")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "way-out.txt")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "way-to-secret.txt")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(file(&scene, &project, &[("path", "way-to-settings.txt")]).await.status_code()) to equal(StatusCode::NOT_FOUND)

        let refused = file(&scene, &project, &[("path", "way-out.txt")]).await;
        expect(text_field(&refused, "error")) to equal("There is no such file.".to_string())
        let inside = file(&scene, &project, &[("path", "way-inside.txt")]).await;
        expect(shown(&inside)) to equal(names(&["+1 # Sample"]))
        let unnamed = file(&scene, &project, &[]).await;
        expect(unnamed.status_code()) to equal(StatusCode::BAD_REQUEST)
    }
}
