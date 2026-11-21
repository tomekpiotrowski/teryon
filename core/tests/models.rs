//! Specs for choosing the model a thread's agent runs on, and for the models on offer.
//!
//! A wrong model wastes part of a usage limit or gives weaker work, so these
//! check what each program was started with and what the thread records.

mod common;

use std::time::Duration;

use lets_expect::lets_expect;
use serde_json::{json, Value};
use tokio::time::sleep;

use common::{fake_agents, invocations, project_folder, unknown, Scene};
use teryon_core::agents::models::ModelLists;
use teryon_core::agents::Voice;
use teryon_core::protocol::Event;
use teryon_core::settings::{AgentSettings, GrokSettings};
use teryon_core::store::Status;
use teryon_core::workspace::Workspace;

/// The models a snapshot offers.
fn offered(workspace: &Workspace) -> Option<ModelLists> {
    match workspace.snapshot() {
        Event::Snapshot { models, .. } => Some(models),
        _ => None,
    }
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// Fake agents, with Grok's program and models as given.
fn grok_with(program: &str, models: Option<&[&str]>) -> AgentSettings {
    let agents = fake_agents();
    AgentSettings {
        grok: GrokSettings {
            program: program.to_string(),
            models: models.map(names),
            ..agents.grok.clone()
        },
        ..agents
    }
}

lets_expect! { #tokio_test
    // requirement: a new thread starts on the model chosen for it, and the program is told
    // requirement: each turn's summary names the model that ran, as the program reports it
    story a_thread_runs_on_its_chosen_model {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, "claude", Some(" sonnet "), "").unwrap();
        expect(thread.model.clone()) to equal(Some("sonnet".to_string()))

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        let runs = invocations(&folder);
        expect(runs[0].contains("--model sonnet --")) to be_true
        // The recording names the model it ran, which is what the thread shows.
        expect(scene.entries(&thread)[2].body["model"].clone()) to equal(json!("claude-fable-5-2"))
    }

    // requirement: "default" passes no model, leaving the choice to the program
    // requirement: Codex never names the model it ran, so on its default the summary names none
    story a_thread_on_the_default_model {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, "codex", None, "").unwrap();
        expect(thread.model.clone()) to equal(None)

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[0].contains(" -m ")) to be_false
        let summary = scene.entries(&thread).into_iter().find(|entry| entry.kind == "turn_summary").unwrap();
        expect(summary.body["model"].clone()) to equal(Value::Null)
    }

    // requirement: Codex's summary names the model asked for, since that is all that is known
    story a_codex_thread_on_a_chosen_model {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, "codex", Some("gpt-6-astra"), "").unwrap();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[0].contains("-m gpt-6-astra")) to be_true
        let summary = scene.entries(&thread).into_iter().find(|entry| entry.kind == "turn_summary").unwrap();
        expect(summary.body["model"].clone()) to equal(json!("gpt-6-astra"))
    }

    // requirement: a model name that would reach the program as an option, or as several words, is refused
    story a_malformed_model_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let option = scene.workspace.start_thread(project.id, "claude", Some("--help"), "").map_err(|problem| problem.to_string());
        expect(option.err()) to equal(Some("A model's name has no spaces in it, and does not start with a dash.".to_string()))
        let blank = scene.workspace.start_thread(project.id, "claude", Some("  "), "").map_err(|problem| problem.to_string());
        expect(blank.err()) to equal(Some("A model needs a name.".to_string()))
        expect(scene.threads().len()) to equal(0)
    }

    // requirement: the model can change between messages, and the conversation continues on the new one
    story the_model_changes_between_messages {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread(project.id, "claude", Some("sonnet"), "").unwrap();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("opus")).unwrap();
        expect(changed.model.clone()) to equal(Some("opus".to_string()))
        expect(scene.thread_now(&thread).model) to equal(Some("opus".to_string()))
        scene.say(&thread, "again").unwrap();
        scene.wait_until_idle(&thread).await;

        scene.workspace.change_agent(thread.id, Voice::Author, "claude", None).unwrap();
        scene.say(&thread, "and again").unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs[1].contains("--resume")) to be_true
        expect(runs[1].contains("--model opus --")) to be_true
        expect(runs[2].contains("--model")) to be_false
    }

    // requirement: changing the model while the agent works applies to the next message, and is kept when the turn ends
    story the_model_changes_while_the_agent_works {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread(project.id, "claude", Some("sonnet"), "").unwrap();
        scene.say(&thread, "be slow").unwrap();

        // The program has started, with the model the thread had then.
        for _ in 0..200 {
            if !invocations(&folder).is_empty() {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("opus"));
        expect(changed.is_ok()) to be_true
        expect(scene.thread_now(&thread).status) to equal(Status::Running)
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).model) to equal(Some("opus".to_string()))
        expect(invocations(&folder)[0].contains("--model sonnet --")) to be_true
    }

    // requirement: a thread without a reviewer has no reviewer's model to change
    story the_model_of_a_missing_reviewer_or_thread_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        let reviewer = scene.workspace.change_agent(thread.id, Voice::Reviewer, "codex", Some("opus")).map_err(|problem| problem.to_string());
        expect(reviewer.err()) to equal(Some("Nobody reviews this thread yet. Ask for a review first.".to_string()))
        let elsewhere = scene.workspace.change_agent(unknown(), Voice::Author, "claude", Some("opus")).map_err(|problem| problem.to_string());
        expect(elsewhere.err()) to equal(Some("There is no such thread.".to_string()))
        let malformed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("-x")).map_err(|problem| problem.to_string());
        expect(malformed.is_err()) to be_true
        expect(scene.thread_now(&thread).model) to equal(None)
    }

    // requirement: the snapshot offers each agent's models: Claude's short names, nothing for Codex, and Grok's once listed
    story the_snapshot_offers_the_models {
        let scene = Scene::new();
        expect(offered(&scene.workspace)) to equal(Some(ModelLists { claude: names(&["opus", "sonnet", "haiku"]), codex: Vec::new(), grok: Vec::new() }))
    }

    // requirement: Grok's models are read from the program, and clients are told when they arrive
    story grok_lists_its_models {
        let scene = Scene::with(fake_agents());
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        let grok = names(&["grok-4.7", "grok-4.7-build-fast", "grok-4.6", "grok-4.5"]);
        expect(offered(&scene.workspace).map(|models| models.grok)) to equal(Some(grok.clone()))
        let told = matches!(changes.try_recv(), Ok(Event::ModelsChanged { models }) if models.grok == grok);
        expect(told) to be_true
    }

    // requirement: Grok's models named in config.toml are offered as they are, without asking Grok
    story grok_models_named_in_settings_are_kept {
        let scene = Scene::with(grok_with("a-program-nobody-has", Some(&["grok-4.6"])));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| models.grok)) to equal(Some(names(&["grok-4.6"])))
        expect(changes.try_recv().is_err()) to be_true
    }

    // requirement: when Grok cannot list its models, it offers only its default
    story grok_that_cannot_list_offers_its_default {
        let scene = Scene::with(grok_with("a-program-nobody-has", None));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| models.grok)) to equal(Some(Vec::new()))
        expect(changes.try_recv().is_err()) to be_true
    }
}
