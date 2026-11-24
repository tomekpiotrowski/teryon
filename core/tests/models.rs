//! Specs for choosing the model a thread's agent runs on, and for the models on offer.
//!
//! A wrong model wastes part of a usage limit or gives weaker work, so these
//! check what each program was started with and what the thread records.

mod common;

use std::fs::{create_dir_all, metadata, set_permissions, write, Permissions};
use std::time::Duration;

use tempfile::{tempdir, TempDir};

use lets_expect::lets_expect;
use serde_json::{json, Value};
use tokio::time::sleep;

use common::{fake_agents, invocations, project_folder, unknown, Scene};
use teryon_core::agents::models::{
    list_claude_models, list_codex_models, Effort, ListingError, ModelLists, ModelOption,
};
use teryon_core::agents::Voice;
use teryon_core::protocol::Event;
use teryon_core::settings::{
    AgentSettings, ClaudeSettings, CodexSettings, GeminiSettings, GrokSettings,
};
use teryon_core::store::Status;
use teryon_core::workspace::Workspace;

/// The models a snapshot offers.
fn offered(workspace: &Workspace) -> Option<ModelLists> {
    match workspace.snapshot() {
        Event::Snapshot { models, .. } => Some(models),
        _ => None,
    }
}

fn ids(models: Vec<ModelOption>) -> Vec<String> {
    models.into_iter().map(|model| model.id).collect()
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// Fake agents, with Grok's program and models as given.
///
/// Codex and Gemini are given empty lists, which are explicit settings, so
/// these stories do not also ask them and then mistake their notices for Grok's.
fn grok_with(program: &str, models: Option<&[&str]>) -> AgentSettings {
    let agents = fake_agents();
    AgentSettings {
        grok: GrokSettings {
            program: program.to_string(),
            models: models.map(names),
            ..agents.grok.clone()
        },
        codex: CodexSettings {
            models: Some(Vec::new()),
            ..agents.codex
        },
        gemini: GeminiSettings {
            models: Some(Vec::new()),
            ..agents.gemini
        },
        ..agents
    }
}

/// Fake agents, with Gemini's program and models as given, and Codex and Grok silenced.
fn gemini_with(program: &str, models: Option<&[&str]>) -> AgentSettings {
    let agents = fake_agents();
    AgentSettings {
        gemini: GeminiSettings {
            program: program.to_string(),
            models: models.map(names),
            ..agents.gemini.clone()
        },
        codex: CodexSettings {
            models: Some(Vec::new()),
            ..agents.codex
        },
        grok: GrokSettings {
            models: Some(Vec::new()),
            ..agents.grok
        },
        ..agents
    }
}

/// A disposable Codex app server with a prescribed response or failure.
fn codex_program(script: &str) -> (TempDir, CodexSettings) {
    let folder = tempdir().unwrap();
    let program = folder.path().join("codex");
    write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
    let permissions = program_permissions();
    set_permissions(&program, permissions).unwrap();
    (
        folder,
        CodexSettings {
            program: program.to_string_lossy().into_owned(),
            ..CodexSettings::default()
        },
    )
}

fn program_permissions() -> Permissions {
    metadata("tests/samples/fake-codex.sh")
        .unwrap()
        .permissions()
}

lets_expect! { #tokio_test
    // requirement: a new thread starts on the model chosen for it, and the program is told
    // requirement: each turn's summary names the model that ran, as the program reports it
    story a_thread_runs_on_its_chosen_model {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, Some("claude"), Some(" sonnet "), "").unwrap();
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

        let thread = scene.workspace.start_thread(project.id, Some("codex"), None, "").unwrap();
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

        let thread = scene.workspace.start_thread_on(project.id, Some("codex"), Some("gpt-6-astra"), Some(Effort::High), "", None).unwrap();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[0].contains("-m gpt-6-astra")) to be_true
        expect(invocations(&folder)[0].contains("model_reasoning_effort=\"high\"")) to be_true
        let summary = scene.entries(&thread).into_iter().find(|entry| entry.kind == "turn_summary").unwrap();
        expect(summary.body["model"].clone()) to equal(json!("gpt-6-astra"))
        expect(summary.body["effort"].clone()) to equal(json!("high"))
    }

    // requirement: a model name that would reach the program as an option, or as several words, is refused
    story a_malformed_model_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let option = scene.workspace.start_thread(project.id, Some("claude"), Some("--help"), "").map_err(|problem| problem.to_string());
        expect(option.err()) to equal(Some("A model's name has no spaces in it, and does not start with a dash.".to_string()))
        let blank = scene.workspace.start_thread(project.id, Some("claude"), Some("  "), "").map_err(|problem| problem.to_string());
        expect(blank.err()) to equal(Some("A model needs a name.".to_string()))
        expect(scene.threads().len()) to equal(0)
    }

    // requirement: the model can change between messages, and the conversation continues on the new one
    story the_model_changes_between_messages {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread(project.id, Some("claude"), Some("sonnet"), "").unwrap();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("opus"), None).unwrap();
        expect(changed.model.clone()) to equal(Some("opus".to_string()))
        expect(scene.thread_now(&thread).model) to equal(Some("opus".to_string()))
        scene.say(&thread, "again").unwrap();
        scene.wait_until_idle(&thread).await;

        scene.workspace.change_agent(thread.id, Voice::Author, "claude", None, None).unwrap();
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
        let thread = scene.workspace.start_thread(project.id, Some("claude"), Some("sonnet"), "").unwrap();
        scene.say(&thread, "be slow").unwrap();

        // The program has started, with the model the thread had then.
        for _ in 0..200 {
            if !invocations(&folder).is_empty() {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("opus"), Some(Effort::Xhigh));
        expect(changed.is_ok()) to be_true
        expect(scene.thread_now(&thread).status) to equal(Status::Running)
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).model) to equal(Some("opus".to_string()))
        expect(scene.thread_now(&thread).effort) to equal(Some(Effort::Xhigh))
        expect(invocations(&folder)[0].contains("--model sonnet --")) to be_true
        scene.say(&thread, "again").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[1].contains("--model opus --effort xhigh")) to be_true
    }

    // requirement: a thread without a reviewer has no reviewer's model to change
    story the_model_of_a_missing_reviewer_or_thread_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        let reviewer = scene.workspace.change_agent(thread.id, Voice::Reviewer, "codex", Some("opus"), None).map_err(|problem| problem.to_string());
        expect(reviewer.err()) to equal(Some("Nobody reviews this thread yet. Ask for a review first.".to_string()))
        let elsewhere = scene.workspace.change_agent(unknown(), Voice::Author, "claude", Some("opus"), None).map_err(|problem| problem.to_string());
        expect(elsewhere.err()) to equal(Some("There is no such thread.".to_string()))
        let malformed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("-x"), None).map_err(|problem| problem.to_string());
        expect(malformed.is_err()) to be_true
        expect(scene.thread_now(&thread).model) to equal(None)
    }

    // requirement: before discovery, the snapshot offers Claude's short names and the other agents' defaults
    story the_snapshot_offers_the_models {
        let scene = Scene::new();
        expect(offered(&scene.workspace).map(|models| ids(models.claude))) to equal(Some(names(&["opus", "sonnet", "haiku"])))
    }

    story claude_discovers_the_current_models_with_a_configured_default {
        let home = tempdir().unwrap();
        let catalog = home.path().join("cache/model-catalog");
        create_dir_all(&catalog).unwrap();
        write(catalog.join("catalog.json"), include_str!("samples/claude-model-catalog.json")).unwrap();
        let settings_file = home.path().join("settings.json");
        let mut settings = ClaudeSettings { home: Some(home.path().to_path_buf()), ..ClaudeSettings::default() };

        write(&settings_file, json!({ "model": "sonnet" }).to_string()).unwrap();
        let options = list_claude_models(&settings).await;
        expect(ids(options.clone())) to equal(names(&["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]))
        expect(options[0].label.as_str()) to equal("Opus 5.5")
        expect(options[0].efforts.clone()) to equal(Effort::ALL[..5].to_vec())
        expect(options[1].efforts.clone()) to equal(Effort::ALL[..3].to_vec())
        expect(options[1].is_default) to be_true
        expect(options[2].default_effort) to equal(Some(Effort::Low))

        write(&settings_file, json!({ "model": "claude-haiku-5-5" }).to_string()).unwrap();
        let options = list_claude_models(&settings).await;
        expect(options[2].is_default) to be_true

        write(catalog.join("catalog.json"), "broken").unwrap();
        expect(ids(list_claude_models(&settings).await)) to equal(names(&["opus", "sonnet", "haiku"]))
        settings.models = Some(names(&["claude-opus-5-5"]));
        let explicit = list_claude_models(&settings).await;
        expect(explicit[0].label.as_str()) to equal("Opus 5.5")
        expect(explicit[0].default_effort) to equal(Some(Effort::Medium))
    }

    story gemini_runs_a_grouped_model_with_its_default_or_a_chosen_effort {
        let scene = Scene::with(gemini_with("tests/samples/fake-gemini.sh", None));
        scene.workspace.list_models().await;
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread(project.id, Some("gemini"), Some("gemini-4-neon"), "").unwrap();
        expect(thread.model.clone()) to equal(Some("gemini-4-neon-high".to_string()))
        expect(thread.effort) to equal(Some(Effort::High))
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "gemini", Some("gemini-4-neon"), Some(Effort::Medium)).unwrap();
        expect(changed.model.clone()) to equal(Some("gemini-4-neon-medium".to_string()))
        expect(changed.effort) to equal(Some(Effort::Medium))
        scene.say(&thread, "again").unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs[0].contains("--model gemini-4-neon-high")) to be_true
        expect(runs[1].contains("--model gemini-4-neon-medium")) to be_true
        expect(runs.iter().any(|run| run.contains("--effort"))) to be_false
    }

    // requirement: a Gemini thread works immediately after a restart, before `agy models` finishes
    story gemini_thread_runs_before_model_discovery_after_restart {
        let home = tempdir().unwrap();
        let folder = project_folder();
        let setup = common::setup(gemini_with("tests/samples/fake-gemini.sh", None));

        let (thread_id, stored_model) = {
            let workspace = Workspace::open(home.path(), setup.clone()).unwrap();
            workspace.list_models().await;
            let project = workspace
                .add_project("Sample", folder.path().to_str().unwrap())
                .unwrap();
            let thread = workspace
                .start_thread_on(project.id, Some("gemini"), Some("gemini-4-neon"), Some(Effort::High), "", None)
                .unwrap();
            (thread.id, thread.model)
        };
        expect(stored_model) to equal(Some("gemini-4-neon-high".to_string()))

        // Reopen without calling `list_models()`, simulating the startup window before `agy models` answers.
        let workspace = Workspace::open(home.path(), setup).unwrap();
        expect(offered(&workspace).unwrap().gemini.is_empty()) to be_true
        workspace.send_message(thread_id, "hello", Voice::Author).unwrap();
        for _ in 0..200 {
            let idle = match workspace.snapshot() {
                Event::Snapshot { threads, .. } => threads
                    .iter()
                    .find(|thread| thread.id == thread_id)
                    .is_some_and(|thread| thread.status == Status::Idle),
                _ => false,
            };
            if idle {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        let runs = invocations(&folder);
        expect(runs.len()) to equal(1)
        expect(runs[0].contains("--model gemini-4-neon-high")) to be_true
        expect(runs[0].contains("--effort")) to be_false
    }

    // requirement: Grok's models are read from the program, and clients are told when they arrive
    story grok_lists_its_models {
        let scene = Scene::with(grok_with("tests/samples/fake-grok.sh", None));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        let grok = names(&["grok-4.7", "grok-4.7-build-fast", "grok-4.6", "grok-4.5"]);
        expect(offered(&scene.workspace).map(|models| ids(models.grok))) to equal(Some(grok.clone()))
        let told = matches!(changes.try_recv(), Ok(Event::ModelsChanged { models }) if ids(models.grok.clone()) == grok);
        expect(told) to be_true
    }

    // requirement: Grok's models named in config.toml are offered as they are, without asking Grok
    story grok_models_named_in_settings_are_kept {
        let scene = Scene::with(grok_with("a-program-nobody-has", Some(&["grok-4.6"])));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| ids(models.grok))) to equal(Some(names(&["grok-4.6"])))
        expect(changes.try_recv().is_err()) to be_true
    }

    // requirement: when Grok cannot list its models, it offers only its default
    story grok_that_cannot_list_offers_its_default {
        let scene = Scene::with(grok_with("a-program-nobody-has", None));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| ids(models.grok))) to equal(Some(Vec::new()))
        expect(changes.try_recv().is_err()) to be_true
    }

    // requirement: Gemini's models are read from Antigravity, keeping only Gemini models, and clients are told when they arrive
    story gemini_lists_its_models {
        let scene = Scene::with(gemini_with("tests/samples/fake-gemini.sh", None));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        let gemini = names(&["gemini-4-neon", "gemini-3.8-flash"]);
        expect(offered(&scene.workspace).map(|models| ids(models.gemini))) to equal(Some(gemini.clone()))
        let told = matches!(changes.try_recv(), Ok(Event::ModelsChanged { models }) if ids(models.gemini.clone()) == gemini);
        expect(told) to be_true
    }

    // requirement: Gemini's models named in config.toml are offered as they are, without asking Antigravity
    story gemini_models_named_in_settings_are_kept {
        let scene = Scene::with(gemini_with("a-program-nobody-has", Some(&["gemini-4-neon-high"])));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| ids(models.gemini))) to equal(Some(names(&["gemini-4-neon"])))
        expect(changes.try_recv().is_err()) to be_true
    }

    // requirement: when Antigravity cannot list its models, Gemini offers only its default
    story gemini_that_cannot_list_offers_its_default {
        let scene = Scene::with(gemini_with("a-program-nobody-has", None));
        let mut changes = scene.workspace.subscribe();

        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| ids(models.gemini))) to equal(Some(Vec::new()))
        expect(changes.try_recv().is_err()) to be_true
    }

    // requirement: a Gemini thread passes its chosen model and records the model Antigravity reports
    story a_gemini_thread_runs_on_its_chosen_model {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, Some("gemini"), Some("gemini-4-neon-high"), "").unwrap();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[0].contains("--model gemini-4-neon-high")) to be_true
        let summary = scene.entries(&thread).into_iter().find(|entry| entry.kind == "turn_summary").unwrap();
        expect(summary.body["model"].clone()) to equal(json!("gemini-3.8-flash"))
        expect(summary.body["effort"].clone()) to equal(json!("low"))
    }
    // requirement: Codex discovery follows initialization and pagination, hides hidden
    // models, uses model identifiers rather than display IDs, and removes duplicates
    // requirement: whichever list arrives later still offers the one that arrived first
    story codex_lists_its_models_and_announces_them {
        let scene = Scene::with(fake_agents());
        let mut changes = scene.workspace.subscribe();
        scene.workspace.list_models().await;
        let codex = names(&["gpt-6-astra", "gpt-6-sol"]);
        let catalog = offered(&scene.workspace).unwrap().codex;
        expect(catalog[0].label.as_str()) to equal("GPT-6 Astra")
        expect(catalog[0].efforts.clone()) to equal(vec![Effort::Low, Effort::Medium, Effort::High, Effort::Ultra])
        expect(catalog[0].default_effort) to equal(Some(Effort::Medium))
        expect(catalog[0].is_default) to be_true
        expect(catalog[1].is_default) to be_false
        expect(offered(&scene.workspace).map(|models| ids(models.codex))) to equal(Some(codex.clone()))
        // The later notice has to keep the earlier list. A copy taken before
        // the other agent answered would wipe that agent for whoever is listening.
        let mut last_has_all = false;
        while let Ok(event) = changes.try_recv() {
            if let Event::ModelsChanged { models } = event {
                last_has_all = ids(models.codex) == codex && models.grok.len() == 4 && models.gemini.len() == 2;
            }
        }
        expect(last_has_all) to be_true
        expect(offered(&scene.workspace).map(|models| models.grok.len())) to equal(Some(4))
        expect(offered(&scene.workspace).map(|models| models.gemini.len())) to equal(Some(2))
    }

    // requirement: explicit Codex models, including an empty list, bypass discovery
    story codex_models_in_settings_are_kept {
        let mut results = Vec::new();
        for models in [Vec::new(), names(&["custom-model"])] {
            let mut agents = fake_agents();
            agents.codex.program = "a-program-nobody-has".to_string();
            agents.codex.models = Some(models.clone());
            agents.grok.models = Some(Vec::new());
            agents.gemini.models = Some(Vec::new());
            let scene = Scene::with(agents);
            let mut changes = scene.workspace.subscribe();
            scene.workspace.list_models().await;
            results.push(offered(&scene.workspace).map(|models| ids(models.codex)) == Some(models) && changes.try_recv().is_err());
        }
        expect(results) to equal(vec![true, true])
    }

    // requirement: missing Codex does not prevent Grok discovery or startup
    story missing_codex_leaves_its_default_and_grok_still_lists {
        let mut agents = fake_agents();
        agents.codex.program = "a-program-nobody-has".to_string();
        let scene = Scene::with(agents);
        scene.workspace.list_models().await;
        expect(offered(&scene.workspace).map(|models| ids(models.codex))) to equal(Some(Vec::new()))
        expect(offered(&scene.workspace).map(|models| models.grok.len())) to equal(Some(4))
    }

    // requirement: protocol failures are reported instead of being mistaken for empty lists
    story invalid_codex_responses_are_reported {
        let mut results = Vec::new();
        for response in [
            "not json",
            r#"{"id":1,"error":{"code":-1,"message":"unavailable"}}"#,
            r#"{"id":1}"#,
            r#"{"id":1,"result":{}}"#,
            r#"{"id":1,"result":{"data":[{"model":"--bad"}]}}"#,
            r#"{"id":1,"result":{"data":[{"model":""}]}}"#,
            r#"{"id":1,"result":{"data":[{}]}}"#,
            r#"{"id":1,"result":{"data":[],"nextCursor":42}}"#,
        ] {
            let script = format!("read -r request\necho '{{\"id\":0,\"result\":{{}}}}'\nread -r request\nread -r request\necho '{response}'");
            let (_folder, settings) = codex_program(&script);
            results.push(matches!(list_codex_models(&settings).await, Err(ListingError::Failed { .. })));
        }
        expect(results.iter().all(|success| *success)) to be_true
    }

    story codex_initialization_failures_and_early_exit_are_reported {
        let mut results = Vec::new();
        for script in ["exit 1", "read -r request\necho '{\"id\":0,\"error\":{\"message\":\"refused\"}}'", "read -r request\necho '{\"id\":0,\"result\":{}}'"] {
            let (_folder, settings) = codex_program(script);
            results.push(matches!(list_codex_models(&settings).await, Err(ListingError::Failed { .. })));
        }
        expect(results.iter().all(|success| *success)) to be_true
    }

    story an_empty_codex_catalog_is_valid {
        let (_folder, settings) = codex_program("read -r request\necho '{\"id\":0,\"result\":{}}'\nread -r request\nread -r request\necho '{\"id\":1,\"result\":{\"data\":[],\"nextCursor\":null}}'");
        expect(list_codex_models(&settings).await.unwrap()) to equal(Vec::<ModelOption>::new())
    }

    // requirement: a stuck program cannot keep model discovery running forever
    story codex_discovery_times_out {
        let (_folder, settings) = codex_program("exec sleep 30");
        expect(matches!(list_codex_models(&settings).await, Err(ListingError::TooSlow { .. }))) to be_true
    }

}
