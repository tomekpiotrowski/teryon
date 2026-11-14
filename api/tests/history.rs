//! Request specs for the history screen.
//!
//! Every project here is a real git repository made for the test, because the
//! answers come from the `git` program. Each commit is made at a moment the
//! test chooses, so that their order and who was working at the time are
//! known.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use axum_test::TestResponse;
use lets_expect::lets_expect;
use serde_json::Value;
use tempfile::TempDir;

use common::{git, git_at, project_folder, put, scene, text_field, Scene};
use teryon::workspace::project;

/// The moment of the first commit of every repository here. Later commits come a minute apart.
const START: i64 = 1_700_000_000;

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

/// A project folder holding a repository with one commit on `main`.
fn repository() -> (TempDir, String) {
    let folder = project_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    put(folder.path(), "README.md", "# Sample\n");
    put(folder.path(), "src/app.txt", "one\ntwo\nthree\n");
    let first = record(folder.path(), 0, "Start the sample");
    (folder, first)
}

async fn history(scene: &Scene, project: &project::Model, query: &[(&str, &str)]) -> TestResponse {
    scene
        .get(&format!("/api/projects/{}/history", project.id), query)
        .await
}

async fn changes(scene: &Scene, project: &project::Model, query: &[(&str, &str)]) -> TestResponse {
    scene
        .get(&format!("/api/projects/{}/changes", project.id), query)
        .await
}

async fn change(scene: &Scene, project: &project::Model, query: &[(&str, &str)]) -> TestResponse {
    scene
        .get(&format!("/api/projects/{}/change", project.id), query)
        .await
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// A field of an answer as text, where a missing value reads `null`.
fn field(response: &TestResponse, name: &str) -> String {
    as_text(&response.json::<Value>()[name])
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn commits(response: &TestResponse) -> Vec<Value> {
    response.json::<Value>()["commits"]
        .as_array()
        .cloned()
        .expect("a list of commits")
}

/// One field of every commit listed, in order.
fn each(response: &TestResponse, name: &str) -> Vec<String> {
    commits(response)
        .iter()
        .map(|commit| as_text(&commit[name]))
        .collect()
}

/// The commits listed, each as `subject` or, when the main branch has it too, `subject (main)`.
fn listed(response: &TestResponse) -> Vec<String> {
    commits(response)
        .iter()
        .map(|commit| {
            let subject = as_text(&commit["subject"]);
            if commit["on_base"] == Value::Bool(true) {
                format!("{subject} (main)")
            } else {
                subject
            }
        })
        .collect()
}

/// One commit listed, found by the first line of its message.
fn commit_named(response: &TestResponse, subject: &str) -> Value {
    commits(response)
        .into_iter()
        .find(|commit| commit["subject"] == subject)
        .expect("the commit is listed")
}

/// The files a commit changed: `M src/app.txt +1 -1`, without counts for a file that is not text.
fn changed(response: &TestResponse) -> Vec<String> {
    let files = response.json::<Value>()["files"].clone();
    let mut changed = Vec::new();
    for file in files.as_array().expect("a list of files") {
        let letter = match file["change"].as_str() {
            Some("added") => "A",
            Some("modified") => "M",
            Some("deleted") => "D",
            _ => "?",
        };
        let counts = match (file["added"].as_u64(), file["removed"].as_u64()) {
            (Some(added), Some(removed)) => format!(" +{added} -{removed}"),
            _ => String::new(),
        };
        changed.push(format!(
            "{letter} {}{counts}",
            file["path"].as_str().unwrap_or_default()
        ));
    }
    changed
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

lets_expect! { #tokio_test
    // requirement: the commits of the project's directory are listed newest first, each with who made it, when, on top of what, and the files it changed
    story the_commits_of_the_project_directory_are_listed {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        let second = record(folder.path(), 1, "Change the app");
        put(folder.path(), "draft.txt", "not committed\n");

        let response = history(&scene, &project, &[]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(each(&response, "subject")) to equal(names(&["Change the app", "Start the sample"]))
        expect(each(&response, "hash")) to equal(vec![second.clone(), first.clone()])
        expect(each(&response, "parents")) to equal(vec![format!("[\"{first}\"]"), "[]".to_string()])
        expect(each(&response, "author")) to equal(names(&["Tester", "Tester"]))
        expect(each(&response, "email")) to equal(names(&["tester@example.com", "tester@example.com"]))
        expect(each(&response, "time")) to equal(vec![(START + 60).to_string(), START.to_string()])
        expect(each(&response, "files")) to equal(names(&[
            "[\"notes/new file \u{17c}.txt\",\"src/app.txt\"]",
            "[\"README.md\",\"src/app.txt\"]",
        ]))
        expect(each(&response, "on_base")) to equal(names(&["false", "false"]))
        expect(each(&response, "thread")) to equal(names(&["null", "null"]))
        expect(field(&response, "branches")) to equal("[\"main\"]".to_string())
        expect(field(&response, "branch")) to equal("main".to_string())
        expect(field(&response, "base")) to equal("null".to_string())
        expect(field(&response, "ahead")) to equal("0".to_string())
        expect(field(&response, "more")) to equal("false".to_string())
    }

    // requirement: on a branch other than the main one, the commits the main branch has too are told apart, and the branch's own are counted
    // requirement: a merge lists the files it brought in
    story a_branch_is_told_apart_from_the_main_branch {
        let scene = scene().await;
        let folder = branched();
        let project = scene.project(&folder).await;

        let response = history(&scene, &project, &[]).await;
        expect(listed(&response)) to equal(names(&[
            "Merge main into feature",
            "Change the app",
            "Add notes",
            "Move main on (main)",
            "Start the sample (main)",
        ]))
        expect(field(&response, "branch")) to equal("feature".to_string())
        expect(field(&response, "base")) to equal("main".to_string())
        expect(field(&response, "ahead")) to equal("3".to_string())
        expect(field(&response, "branches")) to equal("[\"feature\",\"main\"]".to_string())
        expect(commit_named(&response, "Merge main into feature")["parents"].as_array().map(Vec::len)) to equal(Some(2))
        expect(as_text(&commit_named(&response, "Merge main into feature")["files"])) to equal("[\"README.md\"]".to_string())

        let named = history(&scene, &project, &[("branch", "feature")]).await;
        expect(listed(&named)) to equal(listed(&response))

        let main = history(&scene, &project, &[("branch", "main")]).await;
        expect(listed(&main)) to equal(names(&["Move main on", "Start the sample"]))
        expect(field(&main, "branch")) to equal("main".to_string())
        expect(field(&main, "base")) to equal("null".to_string())
        expect(field(&main, "ahead")) to equal("0".to_string())
    }

    // requirement: a branch is listed as it was committed, wherever it is checked out
    story a_branch_checked_out_nowhere_is_listed {
        let scene = scene().await;
        let folder = branched();
        let project = scene.project(&folder).await;
        git(folder.path(), &["switch", "-q", "main"]);

        let own = history(&scene, &project, &[]).await;
        expect(listed(&own)) to equal(names(&["Move main on", "Start the sample"]))

        let response = history(&scene, &project, &[("branch", "feature")]).await;
        expect(listed(&response)) to equal(names(&[
            "Merge main into feature",
            "Change the app",
            "Add notes",
            "Move main on (main)",
            "Start the sample (main)",
        ]))
        expect(field(&response, "branch")) to equal("feature".to_string())
    }

    // requirement: a directory that is not on a branch still has a history, told apart from the main branch
    story a_directory_off_any_branch_is_listed {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["checkout", "-q", "--detach"]);
        put(folder.path(), "notes.txt", "notes\n");
        record(folder.path(), 1, "Add notes");

        let response = history(&scene, &project, &[]).await;
        expect(listed(&response)) to equal(names(&["Add notes", "Start the sample (main)"]))
        expect(field(&response, "branch")) to equal("null".to_string())
        expect(field(&response, "base")) to equal("main".to_string())
        expect(field(&response, "ahead")) to equal("1".to_string())
    }

    // requirement: without a main branch no commit is told apart
    story a_repository_without_a_main_branch_tells_nothing_apart {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["branch", "-q", "-m", "trunk"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "notes.txt", "notes\n");
        record(folder.path(), 1, "Add notes");

        let response = history(&scene, &project, &[]).await;
        expect(listed(&response)) to equal(names(&["Add notes", "Start the sample"]))
        expect(field(&response, "base")) to equal("null".to_string())
        expect(field(&response, "ahead")) to equal("0".to_string())
    }

    // requirement: `master` counts as the main branch when there is no `main`
    story master_is_the_main_branch_of_an_older_repository {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;
        git(folder.path(), &["branch", "-q", "-m", "master"]);
        git(folder.path(), &["switch", "-q", "-c", "feature"]);
        put(folder.path(), "notes.txt", "notes\n");
        record(folder.path(), 1, "Add notes");

        let response = history(&scene, &project, &[]).await;
        expect(listed(&response)) to equal(names(&["Add notes", "Start the sample (main)"]))
        expect(field(&response, "base")) to equal("master".to_string())
    }

    // requirement: a repository without a commit has no history
    story a_repository_without_a_commit_has_no_history {
        let scene = scene().await;
        let folder = project_folder();
        git(folder.path(), &["init", "-q", "-b", "main"]);
        put(folder.path(), "loose.txt", "two\n");
        let project = scene.project(&folder).await;

        let response = history(&scene, &project, &[]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(listed(&response)) to equal(names(&[]))
        expect(field(&response, "branches")) to equal("[\"main\"]".to_string())
        expect(field(&response, "branch")) to equal("main".to_string())
        expect(field(&response, "more")) to equal("false".to_string())

        let named = history(&scene, &project, &[("branch", "main")]).await;
        expect(named.status_code()) to equal(StatusCode::OK)
        expect(listed(&named)) to equal(names(&[]))
    }

    // requirement: at most 200 commits are listed, and the answer says when there are older ones
    story a_long_history_is_cut_short {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;
        for number in 1..200 {
            record(folder.path(), number, &format!("Commit {number}"));
        }

        let whole = history(&scene, &project, &[]).await;
        expect(commits(&whole).len()) to equal(200)
        expect(field(&whole, "more")) to equal("false".to_string())

        record(folder.path(), 200, "Commit 200");
        let cut = history(&scene, &project, &[]).await;
        expect(commits(&cut).len()) to equal(200)
        expect(field(&cut, "more")) to equal("true".to_string())
        expect(each(&cut, "subject").first().cloned()) to equal(Some("Commit 200".to_string()))
        expect(each(&cut, "subject").last().cloned()) to equal(Some("Commit 1".to_string()))
    }

    // requirement: a commit made while an agent was working in one of the project's threads names that thread
    story a_commit_names_the_thread_whose_agent_was_working {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;
        let elsewhere = project_folder();
        let other_project = scene.project(&elsewhere).await;

        let other_thread = scene.thread(&other_project, "claude").await;
        scene.say(&other_thread, "hello").await;
        scene.wait_until_idle(&other_thread).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "hello").await;
        scene.wait_until_idle(&thread).await;
        let entries = scene.entries(thread.id).await;
        let started = entries.first().expect("a message").created_at.and_utc().timestamp();
        let ended = entries.last().expect("a report").created_at.and_utc().timestamp();
        let elsewhere_started = scene.entries(other_thread.id).await[0].created_at.and_utc().timestamp();

        git_at(folder.path(), started - 3600, &["commit", "-q", "--allow-empty", "-m", "Long before"]);
        git_at(folder.path(), elsewhere_started.min(started - 1), &["commit", "-q", "--allow-empty", "-m", "During another project's turn"]);
        git_at(folder.path(), started, &["commit", "-q", "--allow-empty", "-m", "As the turn started"]);
        git_at(folder.path(), ended, &["commit", "-q", "--allow-empty", "-m", "As the turn ended"]);
        git_at(folder.path(), ended + 3600, &["commit", "-q", "--allow-empty", "-m", "Long after"]);

        let response = history(&scene, &project, &[]).await;
        expect(as_text(&commit_named(&response, "Long before")["thread"])) to equal("null".to_string())
        expect(as_text(&commit_named(&response, "During another project's turn")["thread"])) to equal("null".to_string())
        expect(as_text(&commit_named(&response, "As the turn started")["thread"])) to equal(thread.id.to_string())
        expect(as_text(&commit_named(&response, "As the turn ended")["thread"])) to equal(thread.id.to_string())
        expect(as_text(&commit_named(&response, "Long after")["thread"])) to equal("null".to_string())

        // An agent still working may have made anything since it started.
        scene.mark_running(&thread).await;
        let running = history(&scene, &project, &[]).await;
        expect(as_text(&commit_named(&running, "Long after")["thread"])) to equal(thread.id.to_string())
        expect(as_text(&commit_named(&running, "Long before")["thread"])) to equal("null".to_string())
    }

    // requirement: a branch that does not exist is refused
    story an_unknown_branch_is_refused {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;

        let response = history(&scene, &project, &[("branch", "nothing")]).await;
        expect(response.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&response, "error")) to equal("There is no such branch.".to_string())
        expect(history(&scene, &project, &[("branch", "--all")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(history(&scene, &project, &[("branch", "HEAD")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    // requirement: a project whose directory is not a git repository has no history
    story a_directory_that_is_not_a_repository_is_refused {
        let scene = scene().await;
        let folder = project_folder();
        put(folder.path(), "README.md", "# Sample\n");
        let project = scene.project(&folder).await;
        let refusal = "This project's directory is not a git repository, so there are no files or history to show.".to_string();
        let commit = [("commit", "0123456789abcdef")];
        let file = [("commit", "0123456789abcdef"), ("path", "README.md")];

        let listing = history(&scene, &project, &[]).await;
        expect(listing.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&listing, "error")) to equal(refusal.clone())
        let changed_files = changes(&scene, &project, &commit).await;
        expect(changed_files.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&changed_files, "error")) to equal(refusal.clone())
        let changed_file = change(&scene, &project, &file).await;
        expect(changed_file.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&changed_file, "error")) to equal(refusal)
    }

    // requirement: a user sees only the history of their own projects
    story history_is_shown_to_the_owner_only {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;
        let stranger = scene.someone_else().await;
        let listing = format!("/api/projects/{}/history", project.id);
        let changed_files = format!("/api/projects/{}/changes", project.id);
        let changed_file = format!("/api/projects/{}/change", project.id);
        let commit = [("commit", first.as_str())];
        let file = [("commit", first.as_str()), ("path", "README.md")];

        expect(scene.get(&changed_files, &commit).await.status_code()) to equal(StatusCode::OK)
        expect(scene.get(&changed_file, &file).await.status_code()) to equal(StatusCode::OK)
        expect(scene.get_as(&stranger, &listing, &[]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.get_as(&stranger, &changed_files, &commit).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.get_as(&stranger, &changed_file, &file).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.get_as("", &listing, &[]).await.status_code()) to equal(StatusCode::UNAUTHORIZED)
        expect(scene.get_as("", &changed_files, &commit).await.status_code()) to equal(StatusCode::UNAUTHORIZED)
        expect(scene.get_as("", &changed_file, &file).await.status_code()) to equal(StatusCode::UNAUTHORIZED)

        scene.delete(&format!("/api/projects/{}", project.id)).await;
        expect(scene.get(&listing, &[]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.get(&changed_files, &commit).await.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    // requirement: a commit shows the rest of its message and the files it changed, each with how many lines were added and removed
    story a_commit_shows_what_it_changed {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "notes/new file \u{17c}.txt", "hello\n");
        put(folder.path(), "picture.bin", [137, 80, 78, 71, 0, 1, 2]);
        git(folder.path(), &["rm", "-q", "README.md"]);
        let second = record(folder.path(), 1, "Change the app\n\nThe second line is louder now.\n\nAnd there is a fourth.");

        let response = changes(&scene, &project, &[("commit", &second)]).await;
        expect(response.status_code()) to equal(StatusCode::OK)
        expect(changed(&response)) to equal(names(&[
            "D README.md +0 -1",
            "A notes/new file \u{17c}.txt +1 -0",
            "A picture.bin",
            "M src/app.txt +2 -1",
        ]))
        expect(field(&response, "commit")) to equal(second.clone())
        expect(field(&response, "from")) to equal(first.clone())
        expect(field(&response, "body")) to equal("The second line is louder now.\n\nAnd there is a fourth.".to_string())

        let shortened = changes(&scene, &project, &[("commit", &second[..7])]).await;
        expect(field(&shortened, "commit")) to equal(second.clone())
        expect(changed(&shortened)) to equal(changed(&response))
    }

    // requirement: a commit without a parent added everything it holds
    story a_first_commit_added_everything {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;

        let response = changes(&scene, &project, &[("commit", &first)]).await;
        expect(changed(&response)) to equal(names(&["A README.md +1 -0", "A src/app.txt +3 -0"]))
        expect(field(&response, "from")) to equal("null".to_string())
        expect(field(&response, "body")) to equal(String::new())

        let file = change(&scene, &project, &[("commit", &first), ("path", "src/app.txt")]).await;
        expect(shown(&file)) to equal(names(&["+1 one", "+2 two", "+3 three"]))
    }

    // requirement: a merge is compared with the branch that was merged into, so it shows what it brought in
    story a_merge_shows_what_it_brought_in {
        let scene = scene().await;
        let folder = branched();
        let project = scene.project(&folder).await;
        let merge = git(folder.path(), &["rev-parse", "HEAD"]);
        let continued = git(folder.path(), &["rev-parse", "HEAD^1"]);

        let response = changes(&scene, &project, &[("commit", &merge)]).await;
        expect(changed(&response)) to equal(names(&["M README.md +1 -1"]))
        expect(field(&response, "from")) to equal(continued)

        let file = change(&scene, &project, &[("commit", &merge), ("path", "README.md")]).await;
        expect(shown(&file)) to equal(names(&["- # Sample", "+1 # Sample, moved on"]))
    }

    // requirement: any two commits can be compared in place of a commit and the one before it
    story two_commits_are_compared {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\n");
        record(folder.path(), 1, "Change the app");
        put(folder.path(), "notes.txt", "notes\n");
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        let third = record(folder.path(), 2, "Add notes");

        let response = changes(&scene, &project, &[("commit", &third), ("from", &first)]).await;
        expect(changed(&response)) to equal(names(&["A notes.txt +1 -0", "M src/app.txt +2 -1"]))
        expect(field(&response, "commit")) to equal(third.clone())
        expect(field(&response, "from")) to equal(first.clone())

        let file = change(&scene, &project, &[("commit", &third), ("from", &first), ("path", "src/app.txt")]).await;
        expect(shown(&file)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three", "+4 four"]))
        let alone = change(&scene, &project, &[("commit", &third), ("path", "src/app.txt")]).await;
        expect(shown(&alone)) to equal(names(&["1 one", "2 TWO", "3 three", "+4 four"]))

        let backwards = changes(&scene, &project, &[("commit", &first), ("from", &third)]).await;
        expect(changed(&backwards)) to equal(names(&["D notes.txt +0 -1", "M src/app.txt +1 -2"]))
        let same = changes(&scene, &project, &[("commit", &third), ("from", &third)]).await;
        expect(changed(&same)) to equal(names(&[]))
    }

    // requirement: a file a commit changed is shown line by line, with what was removed where it was and what was added marked
    story a_changed_file_shows_what_changed {
        let scene = scene().await;
        let (folder, _) = repository();
        let project = scene.project(&folder).await;
        put(folder.path(), "src/app.txt", "one\nTWO\nthree\nfour\n");
        put(folder.path(), "notes.txt", "hello\n");
        put(folder.path(), "picture.bin", [137, 80, 78, 71, 0, 1, 2]);
        put(folder.path(), "too-large.txt", "a".repeat(1024 * 1024 + 1));
        git(folder.path(), &["rm", "-q", "README.md"]);
        let second = record(folder.path(), 1, "Change the app");
        put(folder.path(), "src/app.txt", "not committed\n");

        let modified = change(&scene, &project, &[("commit", &second), ("path", "src/app.txt")]).await;
        expect(modified.status_code()) to equal(StatusCode::OK)
        expect(field(&modified, "path")) to equal("src/app.txt".to_string())
        expect(kind(&modified)) to equal("text".to_string())
        expect(shown(&modified)) to equal(names(&["1 one", "- two", "+2 TWO", "3 three", "+4 four"]))
        let added = change(&scene, &project, &[("commit", &second), ("path", "notes.txt")]).await;
        expect(shown(&added)) to equal(names(&["+1 hello"]))
        let deleted = change(&scene, &project, &[("commit", &second), ("path", "README.md")]).await;
        expect(shown(&deleted)) to equal(names(&["- # Sample"]))
        let picture = change(&scene, &project, &[("commit", &second), ("path", "picture.bin")]).await;
        expect(kind(&picture)) to equal("binary".to_string())
        let too_large = change(&scene, &project, &[("commit", &second), ("path", "too-large.txt")]).await;
        expect(kind(&too_large)) to equal("too_large".to_string())
    }

    // requirement: a commit that does not exist, or a name that is no commit's, is refused
    story an_unknown_commit_is_refused {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;
        let missing = "0123456789abcdef0123456789abcdef01234567";

        let response = changes(&scene, &project, &[("commit", missing)]).await;
        expect(response.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&response, "error")) to equal("There is no such commit.".to_string())
        expect(changes(&scene, &project, &[("commit", "main")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[("commit", "HEAD")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[("commit", "--all")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[("commit", "")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[("commit", &format!("{first}^"))]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[("commit", &first), ("from", missing)]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[("commit", &first), ("from", "main")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(changes(&scene, &project, &[]).await.status_code()) to equal(StatusCode::BAD_REQUEST)

        let file = change(&scene, &project, &[("commit", missing), ("path", "README.md")]).await;
        expect(file.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&file, "error")) to equal("There is no such commit.".to_string())
        expect(change(&scene, &project, &[("commit", &first), ("from", missing), ("path", "README.md")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(change(&scene, &project, &[("commit", &first)]).await.status_code()) to equal(StatusCode::BAD_REQUEST)
    }

    // requirement: only files of the repository can be read through a commit: nothing outside it, nothing of git's own
    story a_path_that_leads_elsewhere_is_refused {
        let scene = scene().await;
        let (folder, first) = repository();
        let project = scene.project(&folder).await;
        let above = format!("../{}/README.md", folder.path().file_name().unwrap().to_str().unwrap());
        let whole = folder.path().canonicalize().unwrap().join("README.md");

        let missing = change(&scene, &project, &[("commit", &first), ("path", "nothing.txt")]).await;
        expect(missing.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(text_field(&missing, "error")) to equal("There is no such file.".to_string())
        expect(change(&scene, &project, &[("commit", &first), ("path", "src")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(change(&scene, &project, &[("commit", &first), ("path", "")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(change(&scene, &project, &[("commit", &first), ("path", &above)]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(change(&scene, &project, &[("commit", &first), ("path", "src/../README.md")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(change(&scene, &project, &[("commit", &first), ("path", whole.to_str().unwrap())]).await.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(change(&scene, &project, &[("commit", &first), ("path", ".git/config")]).await.status_code()) to equal(StatusCode::NOT_FOUND)
    }
}
