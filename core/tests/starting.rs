//! Specs for choosing who the next new thread starts with before its message is sent.
//!
//! The box for a new thread says who will answer, so the service chooses once,
//! holds that choice, and starts the thread on exactly what was shown.

mod common;

use lets_expect::lets_expect;
use uuid::Uuid;

use common::{fake_agents, project_folder, setup, Scene};
use teryon_core::agents::Agent;
use teryon_core::protocol::{Event, Starting};
use teryon_core::settings::{
    AgentSettings, ClaudeSettings, ProviderChoice, StageSetting, StageSettings, Strength,
};
use teryon_core::workspace::Workspace;

/// A scene whose first stage, `talking`, is set as given, and whose Claude runs `sonnet` at medium strength.
fn scene_talking(talking: Option<StageSetting>, agents: AgentSettings) -> Scene {
    let agents = AgentSettings {
        claude: ClaudeSettings {
            medium: Some("sonnet".to_string()),
            ..agents.claude.clone()
        },
        ..agents
    };
    Scene::with_setup(teryon_core::workspace::Setup {
        stages: StageSettings {
            talking,
            ..StageSettings::default()
        },
        ..setup(agents)
    })
}

fn choosing(provider: ProviderChoice) -> Option<StageSetting> {
    Some(StageSetting {
        provider: Some(provider),
        strength: Some(Strength::Medium),
        ..StageSetting::default()
    })
}

/// Who a snapshot says new threads start with.
fn starting(workspace: &Workspace) -> Option<Starting> {
    match workspace.snapshot() {
        Event::Snapshot { starting, .. } => starting,
        _ => None,
    }
}

/// Fake agents, but Grok's program is not on the machine.
fn without_grok() -> AgentSettings {
    let mut agents = fake_agents();
    agents.grok.program = "/nonexistent/grok".to_string();
    agents
}

/// What a snapshot says for a provider and model.
fn provider(agent: &str, model: Option<&str>) -> Option<Starting> {
    Some(Starting::Provider {
        agent: agent.to_string(),
        model: model.map(str::to_string),
    })
}

/// Whether nobody can work.
fn unavailable(workspace: &Workspace) -> bool {
    matches!(starting(workspace), Some(Starting::Unavailable { .. }))
}

/// Whether twenty looks in a row see the same choice as the first.
fn held(workspace: &Workspace) -> bool {
    let first = starting(workspace);
    (0..20).all(|_| starting(workspace) == first)
}

/// Whether starting a thread on a named provider announced a new choice.
fn announced(workspace: &Workspace, project: Uuid) -> bool {
    let mut changes = workspace.subscribe();
    workspace
        .start_thread(project, Some("grok"), None, "")
        .unwrap();
    while let Ok(event) = changes.try_recv() {
        if matches!(event, Event::StartingChanged { .. }) {
            return true;
        }
    }
    false
}

lets_expect! { #tokio_test
    // requirement: nobody is named while the first stage does not choose
    story no_choice_without_a_choosing_stage {
        let scene = scene_talking(None, fake_agents());
        expect(starting(&scene.workspace)) to equal(None)
    }

    // requirement: the provider and the model of the stage's strength are named before sending
    story the_stage_names_a_provider_and_model {
        let scene = scene_talking(choosing(ProviderChoice::Only(Agent::Claude)), fake_agents());
        expect(starting(&scene.workspace)) to equal(provider("claude", Some("sonnet")))
    }

    // requirement: a provider without a model for the strength runs its default
    story a_missing_model_is_the_default {
        let scene = scene_talking(choosing(ProviderChoice::Only(Agent::Codex)), fake_agents());
        expect(starting(&scene.workspace)) to equal(provider("codex", None))
    }

    // requirement: when nobody can work the line says so
    story a_missing_program_leaves_nobody {
        let scene = scene_talking(choosing(ProviderChoice::Only(Agent::Grok)), without_grok());
        expect(unavailable(&scene.workspace)) to be_true
    }

    // requirement: the shown choice stays until it can no longer run
    story the_choice_is_held_between_looks {
        let scene = scene_talking(choosing(ProviderChoice::Any), fake_agents());
        expect(held(&scene.workspace)) to be_true
    }

    // requirement: sending without a pick starts the thread on the shown pair and remembers it
    story a_thread_starts_on_the_shown_choice {
        let scene = scene_talking(choosing(ProviderChoice::Any), fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let shown = starting(&scene.workspace);
        let thread = scene.workspace.start_thread(project.id, None, None, "").unwrap();
        expect(Some(Starting::Provider { agent: thread.agent.clone(), model: thread.model.clone() })) to equal(shown)
        expect(thread.picks.get(&thread.stage).cloned()) to equal(Some(thread.agent.clone()))
    }

    // requirement: a provider the person named does not use up the held choice
    story naming_a_provider_keeps_the_choice {
        let scene = scene_talking(choosing(ProviderChoice::Any), fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let shown = starting(&scene.workspace);
        let told = announced(&scene.workspace, project.id);
        expect(told) to be_false
        expect(starting(&scene.workspace)) to equal(shown)
    }
}
