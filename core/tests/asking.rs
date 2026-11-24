//! Specs for a thread's agent asking the person questions.
//!
//! These drive the workspace directly. The agent is the fake one, which notes
//! the turn Teryon started it for, so a test can ask as that turn, the way
//! `teryon thread ask` does. The command itself is covered by the service's specs.

mod common;

use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lets_expect::lets_expect;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::time::sleep;
use uuid::Uuid;

use common::{invocations, noted_turn, project_folder, put, repository, unknown, Scene};
use teryon_core::agents::asking::SHAPE;
use teryon_core::protocol::Event;
use teryon_core::store::Thread;

/// Questions as an agent writes them: one taking one answer, one taking several.
const ASKED: &str = r#"{"questions": [
    {"question": "Where should answers be kept?", "answers": [{"label": "In the thread"}, {"label": "On the record", "description": "Lets the list show it"}]},
    {"question": "Which screens?", "answers": [{"label": "List"}, {"label": "Title"}, {"label": "Notification"}], "several": true, "recommended": [2, 0]}
]}"#;

/// The questions of [`ASKED`] as they are stored.
fn stored(turn: Uuid) -> Value {
    json!({
        "turn": turn,
        "questions": [
            {
                "question": "Where should answers be kept?",
                "answers": [{ "label": "In the thread", "description": null }, { "label": "On the record", "description": "Lets the list show it" }],
                "choosing": { "kind": "one", "recommended": 0 },
            },
            {
                "question": "Which screens?",
                "answers": [{ "label": "List", "description": null }, { "label": "Title", "description": null }, { "label": "Notification", "description": null }],
                "choosing": { "kind": "several", "recommended": [0, 2] },
            },
        ],
    })
}

/// A thread whose agent is in a turn it noted, waiting to be stopped.
async fn waiting(scene: &Scene, folder: &TempDir) -> (Thread, Uuid) {
    let project = scene.project(folder);
    let thread = scene.thread(&project, "claude");
    scene.say(&thread, "remember the turn and wait").unwrap();
    let turn = noted_turn(folder).await;
    (thread, turn)
}

/// The turn the fake agent noted in `directory`, such as a reviewer in a worktree.
async fn noted_in(directory: &Path) -> Uuid {
    for _ in 0..200 {
        if let Ok(text) = read_to_string(directory.join(".fake-agent-turn")) {
            if let Some(turn) = text.split(' ').nth(1) {
                return Uuid::parse_str(turn).expect("the turn is an identifier");
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the fake agent did not note its turn in time");
}

/// A thread in a repository whose change has one changed file, its agent stopped.
async fn changed(scene: &Scene, folder: &TempDir) -> (Thread, PathBuf) {
    repository(folder);
    let (thread, turn) = waiting(scene, folder).await;
    let path = scene
        .workspace
        .start_change(thread.id, turn, "fix-login", None)
        .await
        .expect("the change starts");
    scene.workspace.stop(thread.id).unwrap();
    scene.wait_until_idle(&thread).await;
    put(&path, "login.txt", "fixed\n");
    scene.workspace.refresh_change(thread.id).await;
    (thread, path)
}

lets_expect! { #tokio_test
    // requirement: every turn of the thread's own agent is told how and when to ask
    story the_agent_is_told_how_to_ask {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let claude = scene.thread(&project, "claude");
        let codex = scene.thread(&project, "codex");

        scene.say(&claude, "hello").unwrap();
        scene.wait_until_idle(&claude).await;
        scene.say(&codex, "hello").unwrap();
        scene.wait_until_idle(&codex).await;
        scene.say(&claude, "and again").unwrap();
        scene.wait_until_idle(&claude).await;

        let runs = invocations(&folder);
        expect(runs[0].contains("Run `teryon thread ask` with a JSON object on standard input")) to be_true
        expect(runs[1].contains("Run `teryon thread ask` with a JSON object on standard input")) to be_true
        expect(runs[2].contains("Run `teryon thread ask` with a JSON object on standard input")) to be_true
    }

    // requirement: the reviewer is not told how to ask
    story the_reviewer_is_not_told_how_to_ask {
        let scene = Scene::new();
        let folder = project_folder();
        let (thread, _) = changed(&scene, &folder).await;

        scene.workspace.review_change(thread.id, "grok", None, None).unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs.last().is_some_and(|run| run.contains("Review the change."))) to be_true
        expect(runs.last().is_some_and(|run| run.contains("teryon thread ask"))) to be_false
    }

    // requirement: the agent's questions are kept in the thread with their turn, announced, and mark the thread as waiting until the person's next message
    story the_agent_asks {
        let scene = Scene::new();
        let folder = project_folder();
        let (thread, turn) = waiting(&scene, &folder).await;
        let mut changes = scene.workspace.subscribe();

        expect(scene.workspace.ask_questions(thread.id, turn, ASKED).is_ok()) to be_true
        let entries = scene.entries(&thread);
        let last = entries.last().cloned().expect("an entry");
        expect(last.kind.clone()) to equal("questions".to_string())
        expect(last.body.clone()) to equal(stored(turn))
        expect(scene.thread_now(&thread).asking) to be_true
        let first = changes.try_recv().ok().map(|event| matches!(event, Event::EntryAdded { entry } if entry.kind == "questions"));
        expect(first) to equal(Some(true))
        let second = changes.try_recv().ok().map(|event| matches!(event, Event::ThreadChanged { thread } if thread.asking));
        expect(second) to equal(Some(true))

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        let scene = scene.restart(|_| {});
        expect(scene.thread_now(&thread).asking) to be_true

        scene.say(&thread, "Answers to your questions: ...").unwrap();
        expect(scene.thread_now(&thread).asking) to be_false
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).asking) to be_false
    }

    // requirement: asking again in the same turn adds the newer questions under the same turn, which replace the older ones
    story the_agent_asks_twice_in_a_turn {
        let scene = Scene::new();
        let folder = project_folder();
        let (thread, turn) = waiting(&scene, &folder).await;

        scene.workspace.ask_questions(thread.id, turn, ASKED).unwrap();
        scene.workspace.ask_questions(thread.id, turn, ASKED).unwrap();

        let asked: Vec<Value> = scene.entries(&thread).into_iter().filter(|entry| entry.kind == "questions").map(|entry| entry.body["turn"].clone()).collect();
        expect(asked) to equal(vec![json!(turn), json!(turn)])
    }

    // requirement: questions are refused with a message the agent can act on, and leave the thread as it was
    story asking_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let (thread, turn) = waiting(&scene, &folder).await;
        let ask = |thread_id, turn, asked: &str| scene.workspace.ask_questions(thread_id, turn, asked).map_err(|problem| problem.to_string());

        expect(ask(unknown(), turn, ASKED)) to equal(Err("There is no such thread.".to_string()))
        expect(ask(thread.id, unknown(), ASKED)) to equal(Err("`teryon thread ask` is for agents working in Teryon, during their turn.".to_string()))
        expect(ask(thread.id, turn, r#"{"questions": []}"#)) to equal(Err(format!("Ask between one and four questions at once; there are 0.\n\n{SHAPE}")))
        let unreadable = ask(thread.id, turn, "not json").unwrap_err();
        expect(unreadable.starts_with("Teryon couldn't read the questions: ")) to be_true
        expect(unreadable.ends_with(SHAPE)) to be_true
        expect(scene.thread_now(&thread).asking) to be_false
        expect(scene.entries(&thread).iter().any(|entry| entry.kind == "questions")) to be_false

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(ask(thread.id, turn, ASKED)) to equal(Err("`teryon thread ask` is for agents working in Teryon, during their turn.".to_string()))
        expect(scene.thread_now(&thread).asking) to be_false
    }

    // requirement: a reviewer may not ask; it reports its findings to the author instead
    story the_reviewer_may_not_ask {
        let scene = Scene::new();
        let folder = project_folder();
        let (thread, path) = changed(&scene, &folder).await;
        scene.workspace.review_change(thread.id, "grok", None, None).unwrap();
        scene.wait_until_idle(&thread).await;

        scene.ask_reviewer(&thread, "remember the turn and wait").unwrap();
        let turn = noted_in(&path).await;

        let refused = scene.workspace.ask_questions(thread.id, turn, ASKED).map_err(|problem| problem.to_string());
        expect(refused) to equal(Err("`teryon thread ask` is for the thread's own agent; a reviewer reports its findings instead.".to_string()))
        expect(scene.thread_now(&thread).asking) to be_false
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
    }
}
