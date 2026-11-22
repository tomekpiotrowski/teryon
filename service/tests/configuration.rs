//! Request specs for the settings screen: reading and saving this machine's choices.
//!
//! The file update itself is covered by the core library. These cover what a
//! client is told, that the next thread uses a saved stage, and that a turn
//! already running finishes with the settings it was given.

mod common;

use std::fs::{read_to_string, write};
use std::path::Path;
use std::time::Duration;

use lets_expect::lets_expect;
use tempfile::TempDir;
use tokio::time::sleep;

use common::{project_id, repository, scene, wait_for};
use teryon::client::{Client, ClientError};
use teryon::session::{Origin, Session};
use teryon_core::agents::Voice;
use teryon_core::paths::Folders;
use teryon_core::protocol::{Answer, Event, Request};
use teryon_core::settings::{Configuration, Settings, StageProvider, Strength};
use teryon_core::store::{Status, Thread};
use teryon_core::workflow::Stage;
use teryon_core::workspace::{Setup, Workspace};

/// The choices the settings screen is showing.
async fn shown(client: &mut Client) -> Configuration {
    match client.call(Request::Configuration).await.unwrap() {
        Answer::Configuration { configuration } => configuration,
        other => panic!("not the settings: {other:?}"),
    }
}

/// `config.toml` as it is on disk.
fn config_text(home: &Path) -> String {
    read_to_string(Folders::inside(home).config.join("config.toml")).unwrap()
}

/// A project in `folder` with one thread, on the given agent or as the first stage chooses.
async fn started(
    client: &mut Client,
    folder: &TempDir,
    agent: Option<&str>,
    model: Option<&str>,
) -> Thread {
    let project = project_id(
        client
            .call(Request::AddProject {
                name: "Sample".to_string(),
                directory: folder.path().to_string_lossy().into_owned(),
            })
            .await
            .unwrap(),
    );
    match client
        .call(Request::StartThread {
            project_id: project,
            agent: agent.map(str::to_string),
            model: model.map(str::to_string),
            title: String::new(),
        })
        .await
        .unwrap()
    {
        Answer::Thread { thread } => *thread,
        other => panic!("not a thread: {other:?}"),
    }
}

/// The turn file once the same process has finished and recorded how it was started.
async fn finished_turn(folder: &TempDir) -> String {
    for _ in 0..200 {
        if let Ok(text) = read_to_string(folder.path().join(".fake-agent-turn")) {
            if text.contains("finished ") {
                return text;
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the fake agent did not finish the turn in time");
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

lets_expect! { #tokio_test

    // requirement: the settings screen reads the choices this machine is using
    story the_screen_reads_the_choices {
        let scene = scene().await;
        let mut client = scene.client().await;

        let configuration = shown(&mut client).await;

        expect(configuration.own_workflows) to be_false
        expect(configuration.editor) to equal("".to_string())
        expect(configuration.terminal) to equal("".to_string())
        expect(configuration.workflows.len()) to equal(2)
        expect(configuration.default_workflow) to equal("default".to_string())
        expect(configuration.automatic_rounds) to equal(3)
    }

    // requirement: saving an editor is announced, and the agent programs stay in the file
    story a_saved_editor_is_announced {
        let scene = scene().await;
        let mut client = scene.client().await;
        let mut configuration = shown(&mut client).await;
        configuration.editor = "zed {path}".to_string();

        client.call(Request::SaveConfiguration { configuration }).await.unwrap();
        let changed = wait_for(&mut client, |event| matches!(event, Event::SettingsChanged { .. })).await;

        let editor = matches!(changed, Event::SettingsChanged { opens, .. } if opens.editor && !opens.terminal);
        expect(editor) to be_true
        let text = config_text(scene.home.path());
        expect(text.contains("fake-claude.sh")) to be_true
        expect(text.contains("fake-grok.sh")) to be_true
        expect(text.contains("zed {path}")) to be_true
    }

    // requirement: a relayed session cannot read or save this machine's settings
    story settings_stay_on_the_machine {
        let home = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(home.path(), Setup::default()).unwrap();
        let mut session = Session::new(workspace, Origin::Relayed);
        let draft = Configuration::from_settings(&Settings::default());

        let (read, _) = session.answer(Request::Configuration).await;
        let (saved, _) = session.answer(Request::SaveConfiguration { configuration: draft }).await;

        expect(read) to equal(Err("This can only be done on the machine itself.".to_string()))
        expect(saved) to equal(Err("This can only be done on the machine itself.".to_string()))
    }

    // requirement: the next thread uses a stage and a model saved a moment ago
    story a_saved_stage_chooses_the_next_thread {
        let scene = scene().await;
        let mut client = scene.client().await;
        let mut configuration = shown(&mut client).await;
        for stage in &mut configuration.stages {
            if stage.stage == Stage::Talking {
                stage.provider = Some(StageProvider::Only { agent: "grok".to_string() });
                stage.strength = Some(Strength::Fast);
            }
        }
        configuration.grok.fast = "grok-test".to_string();
        client.call(Request::SaveConfiguration { configuration }).await.unwrap();

        let thread = started(&mut client, &repository(), None, None).await;

        expect(thread.agent) to equal("grok".to_string())
        expect(thread.model) to equal(Some("grok-test".to_string()))
    }

    // requirement: a turn already running stays running through a save, then finishes with the model it was started with
    story a_running_turn_keeps_going {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = started(&mut client, &folder, Some("claude"), Some("started-model")).await.id;
        client.call(Request::SendMessage { thread_id: thread, text: "remember the turn and wait".to_string(), to: Voice::Author }).await.unwrap();
        written_by_agent(&folder, ".fake-agent-turn").await;
        let mut configuration = shown(&mut client).await;
        configuration.claude.strong = "later-model".to_string();

        client.call(Request::SaveConfiguration { configuration }).await.unwrap();
        let mut other = scene.client().await;
        let snapshot = other.event().await.unwrap();
        let running = matches!(snapshot, Event::Snapshot { threads, .. } if threads.iter().any(|found| found.id == thread && found.status == Status::Running));
        expect(running) to be_true

        write(folder.path().join(".fake-agent-release"), "go\n").unwrap();
        let noted = finished_turn(&folder).await;
        expect(noted.contains("--model started-model")) to be_true
        expect(noted.contains("later-model")) to be_false
        let idle = wait_for(&mut other, |event| matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.status == Status::Idle)).await;
        let model = match idle { Event::ThreadChanged { thread } => thread.model, _ => None };
        expect(model) to equal(Some("started-model".to_string()))
    }

    // requirement: a save does not reuse config.toml.writing, so one save cannot truncate another's temporary file
    story a_save_uses_its_own_temporary_file {
        let scene = scene().await;
        let sentinel = Folders::inside(scene.home.path()).config.join("config.toml.writing");
        write(&sentinel, "leave this name alone\n").unwrap();
        let mut client = scene.client().await;
        let mut configuration = shown(&mut client).await;
        configuration.editor = "zed {path}".to_string();

        client.call(Request::SaveConfiguration { configuration }).await.unwrap();

        expect(read_to_string(&sentinel).unwrap()) to equal("leave this name alone\n".to_string())
        expect(config_text(scene.home.path()).contains("zed {path}")) to be_true
    }

    // requirement: a workflow the rules refuse is not written, and the file stays byte for byte
    story a_refused_workflow_is_not_written {
        let scene = scene().await;
        let before = config_text(scene.home.path());
        let mut client = scene.client().await;
        let mut configuration = shown(&mut client).await;
        let workflow = configuration.workflows.iter_mut().find(|workflow| workflow.name == "default").unwrap();
        workflow.stages = vec![Stage::Review, Stage::Ready, Stage::Done];

        let saved = client.call(Request::SaveConfiguration { configuration }).await;

        let refused = matches!(saved, Err(ClientError::Refused(ref message)) if message.contains("has to start with talking, planning or implementing"));
        expect(refused) to be_true
        expect(config_text(scene.home.path())) to equal(before)
    }
}
