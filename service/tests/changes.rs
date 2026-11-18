//! Request specs for a thread's change through the service: `teryon worktree create`, opening a copy, and refreshing.
//!
//! The fake agent runs the real `teryon` program cargo built, found the way a
//! real agent finds it: through the `PATH` the service gives it. The rules
//! for starting a change are covered by the core library's specs; these
//! cover the command, the environment that makes it work, and the requests.

mod common;

use std::fs::{canonicalize, read_to_string, write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use lets_expect::lets_expect;
use tempfile::{tempdir, TempDir};
use tokio::process::Command;
use tokio::time::sleep;
use uuid::Uuid;

use common::{git, program, project_id, repository, scene, scene_with, thread_id, wait_for};
use teryon::client::{Client, ClientError};
use teryon::session::{Origin, Session};
use teryon_core::protocol::{Answer, Event, Opener, Opens, Request};
use teryon_core::store::{Status, Thread};
use teryon_core::workspace::{Setup, Workspace};

/// A project in `folder` with one Claude thread, through `client`.
async fn thread_in(client: &mut Client, folder: &TempDir) -> Uuid {
    project_and_thread(client, folder).await.1
}

/// A project in `folder`, and one Claude thread in it.
async fn project_and_thread(client: &mut Client, folder: &TempDir) -> (Uuid, Uuid) {
    let project = project_id(
        client
            .call(Request::AddProject {
                name: "Sample".to_string(),
                directory: folder.path().to_string_lossy().into_owned(),
            })
            .await
            .unwrap(),
    );
    (project, another_thread(client, project).await)
}

/// Another Claude thread in a project.
async fn another_thread(client: &mut Client, project: Uuid) -> Uuid {
    thread_id(
        client
            .call(Request::StartThread {
                project_id: project,
                agent: "claude".to_string(),
                title: String::new(),
            })
            .await
            .unwrap(),
    )
}

/// Sends a message and waits for the turn to end, giving the thread as it was then.
async fn turn(client: &mut Client, thread: Uuid, text: &str) -> Thread {
    client
        .call(Request::SendMessage {
            thread_id: thread,
            text: text.to_string(),
        })
        .await
        .unwrap();
    match wait_for(client, |event| {
        // A finished turn leaves the thread unread, unlike the idle thread before it.
        matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.status == Status::Idle && changed.unread)
    })
    .await
    {
        Event::ThreadChanged { thread } => thread,
        _ => unreachable!(),
    }
}

/// What a file the fake agent wrote in a project folder holds, once it is there.
async fn written_by_agent(folder: &TempDir, name: &str) -> String {
    for _ in 0..200 {
        if let Ok(text) = read_to_string(folder.path().join(name)) {
            if text.ends_with('\n') {
                return text.trim().to_string();
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the fake agent did not write {name} in time");
}

fn copy_of(folder: &TempDir, name: &str) -> PathBuf {
    canonicalize(folder.path())
        .unwrap()
        .join(".teryon/worktrees")
        .join(name)
}

/// Runs `teryon worktree create` as a person might in a terminal, and gives what it said and whether it succeeded.
async fn create_by_hand(variables: &[(&str, String)]) -> (bool, String) {
    let mut command = Command::new(program());
    command
        .args(["worktree", "create", "by-hand"])
        .env_remove("TERYON_THREAD")
        .env_remove("TERYON_TURN")
        .env_remove("TERYON_DATA");
    for (name, value) in variables {
        command.env(name, value);
    }
    let output = command.output().await.expect("teryon runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    )
}

fn same_place(left: &Path, right: &Path) -> bool {
    canonicalize(left).ok() == canonicalize(right).ok()
}

lets_expect! { #tokio_test
    // requirement: an agent starts a change by running `teryon worktree create <name>`, which prints the copy's path
    // requirement: the thread then has the change, with the files the agent changed counted
    story an_agent_starts_a_change_through_teryon {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        let after = turn(&mut client, thread, "start a change fix-login").await;

        expect(written_by_agent(&folder, ".fake-agent-worktree").await) to equal(copy_of(&folder, "fix-login").to_string_lossy().into_owned())
        let change = after.change.expect("the thread has a change");
        expect(change.name.clone()) to equal("fix-login".to_string())
        expect(change.base.clone()) to equal(Some("main".to_string()))
        expect(change.state.map(|state| state.files_changed)) to equal(Some(1))
    }

    // requirement: a refused `teryon worktree create` tells the agent why, and ends in failure
    story an_agent_is_told_why_a_change_is_refused {
        let scene = scene().await;
        let folder = repository();
        git(folder.path(), &["branch", "taken"]);
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        let after = turn(&mut client, thread, "start a change taken").await;

        expect(written_by_agent(&folder, ".fake-agent-worktree").await) to equal("There is already a worktree or a branch called `taken`. Pick another name, such as `taken-2`.".to_string())
        expect(after.change) to equal(None)
    }

    // requirement: the agent gets its thread, its turn, the data folder and the running `teryon` first on its PATH
    story the_agent_gets_what_teryon_needs {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        client.call(Request::SendMessage { thread_id: thread, text: "remember the turn and wait".to_string() }).await.unwrap();
        let noted = written_by_agent(&folder, ".fake-agent-turn").await;
        let parts: Vec<&str> = noted.split(' ').collect();
        expect(parts[0].to_string()) to equal(thread.to_string())
        expect(Uuid::parse_str(parts[1]).is_ok()) to be_true
        expect(same_place(Path::new(parts[2]), &scene.data())) to be_true
        expect(same_place(Path::new(parts[3]), &program())) to be_true
        client.call(Request::Stop { thread_id: thread }).await.unwrap();
    }

    // requirement: `teryon worktree create` outside an agent's turn is refused
    story the_command_is_only_for_agents {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        expect(create_by_hand(&[]).await) to equal((false, "This command is for agents working in Teryon, which sets TERYON_THREAD and TERYON_TURN for them.".to_string()))
        let data = scene.data().to_string_lossy().into_owned();
        let pretending = [("TERYON_THREAD", thread.to_string()), ("TERYON_TURN", Uuid::new_v4().to_string()), ("TERYON_DATA", data)];
        expect(create_by_hand(&pretending).await) to equal((false, "`teryon worktree create` is for agents working in Teryon, during their turn.".to_string()))
        let empty = tempdir().unwrap();
        let nowhere = [("TERYON_THREAD", thread.to_string()), ("TERYON_TURN", Uuid::new_v4().to_string()), ("TERYON_DATA", empty.path().to_string_lossy().into_owned())];
        expect(create_by_hand(&nowhere).await) to equal((false, "Teryon is not running.".to_string()))
    }

    // requirement: following a thread reads where its change stands again, so work done outside Teryon shows up
    story following_a_thread_refreshes_its_change {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;
        turn(&mut client, thread, "start a change fix-login").await;

        write(copy_of(&folder, "fix-login").join("by-hand.txt"), "made outside Teryon\n").unwrap();
        client.call(Request::FollowThread { thread_id: thread }).await.unwrap();
        let refreshed = wait_for(&mut client, |event| matches!(event, Event::ThreadChanged { thread: changed }
            if changed.id == thread && changed.change.as_ref().and_then(|change| change.state.as_ref()).is_some_and(|state| state.files_changed == 2))).await;
        let check = matches!(refreshed, Event::ThreadChanged { .. });
        expect(check) to be_true
    }

    // requirement: a copy opens in the editor set up in config.toml; an opener that is not set up is refused
    // requirement: the snapshot says which openers are set up
    story a_copy_is_opened {
        let scene = scene_with("[open]\neditor = \"touch {path}/opened-by-editor\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;
        let snapshot = client.event().await.unwrap();
        let opens = match snapshot { Event::Snapshot { opens, .. } => Some(opens), _ => None };
        expect(opens) to equal(Some(Opens { editor: true, terminal: false }))

        let thread = thread_in(&mut client, &folder).await;
        let before = client.call(Request::OpenCopy { thread_id: thread, with: Opener::Editor }).await;
        let check_none = matches!(before, Err(ClientError::Refused(ref message)) if message == "This thread has no changes to open.");
        expect(check_none) to be_true

        turn(&mut client, thread, "start a change fix-login").await;
        client.call(Request::OpenCopy { thread_id: thread, with: Opener::Editor }).await.unwrap();
        let marker = copy_of(&folder, "fix-login").join("opened-by-editor");
        let mut opened = false;
        for _ in 0..100 {
            if marker.exists() { opened = true; break; }
            sleep(Duration::from_millis(50)).await;
        }
        expect(opened) to be_true

        let terminal = client.call(Request::OpenCopy { thread_id: thread, with: Opener::Terminal }).await;
        let check_terminal = matches!(terminal, Err(ClientError::Refused(ref message)) if message == "Nothing is set up to open it with. Add it under [open] in Teryon's config.toml.");
        expect(check_terminal) to be_true
    }

    // requirement: a change is drafted, applied, caught up and discarded over the connection
    story a_change_is_finished_through_the_service {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let (project, thread) = project_and_thread(&mut client, &folder).await;
        turn(&mut client, thread, "start a change fix-login").await;

        let draft = client.call(Request::DraftDescription { thread_id: thread }).await.unwrap();
        let check_draft = matches!(draft, Answer::Draft { ref text } if text == "start a change fix-login");
        expect(check_draft) to be_true
        let up_to_date = client.call(Request::CatchUp { thread_id: thread }).await;
        let check_up_to_date = matches!(up_to_date, Err(ClientError::Refused(ref message)) if message == "This change already has all the newer work in the project.");
        expect(check_up_to_date) to be_true

        let applied = client.call(Request::ApplyChange { thread_id: thread, description: "Fix the login".to_string() }).await;
        let check_applied = matches!(applied, Ok(Answer::Nothing));
        expect(check_applied) to be_true
        expect(git(folder.path(), &["log", "-1", "--format=%s"])) to equal("Fix the login".to_string())
        expect(folder.path().join("fix-login.txt").exists()) to be_true

        let other = another_thread(&mut client, project).await;
        turn(&mut client, other, "start a change second").await;
        write(folder.path().join("README.md"), "newer work\n").unwrap();
        git(folder.path(), &["commit", "-q", "-am", "newer"]);
        let caught_up = client.call(Request::CatchUp { thread_id: other }).await;
        let check_caught_up = matches!(caught_up, Ok(Answer::CaughtUp { overlaps: false }));
        expect(check_caught_up) to be_true
        let discarded = client.call(Request::DiscardChange { thread_id: other }).await;
        let check_discarded = matches!(discarded, Ok(Answer::Nothing));
        expect(check_discarded) to be_true
        expect(copy_of(&folder, "second").exists()) to be_false
    }

    // requirement: a copy cannot be opened through a Teryon server, only on the machine itself
    story a_copy_is_not_opened_from_afar {
        let home = tempdir().unwrap();
        let workspace = Workspace::open(home.path(), Setup::default()).unwrap();
        let mut session = Session::new(workspace, Origin::Relayed);
        let (answer, _) = session.answer(Request::OpenCopy { thread_id: Uuid::new_v4(), with: Opener::Folder }).await;
        expect(answer) to equal(Err("This can only be done on the machine itself.".to_string()))
    }
}
