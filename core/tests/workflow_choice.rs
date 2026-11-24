//! Specs for starting a thread on a workflow chosen at the start.
//!
//! The workflow decides where a thread begins, so its first stage is also
//! what chooses the agent and the model when the person picked none.

mod common;

use std::collections::BTreeMap;

use lets_expect::lets_expect;

use common::{fake_agents, project_folder, setup, Scene};
use teryon_core::agents::Agent;
use teryon_core::protocol::{Event, Starting};
use teryon_core::settings::{
    AgentSettings, ClaudeSettings, ProviderChoice, Settings, StageSetting, StageSettings, Strength,
    WorkflowDefinition,
};
use teryon_core::workflow::{Stage, Workflows};
use teryon_core::workspace::{Setup, Workspace};

/// Two workflows starting in different stages: `default` in talking, `planned` in planning.
fn two_workflows() -> Workflows {
    let settings = Settings {
        workflows: Some(BTreeMap::from([
            (
                "default".to_string(),
                WorkflowDefinition {
                    description: String::new(),
                    stages: vec![
                        Stage::Talking,
                        Stage::Implementing,
                        Stage::Ready,
                        Stage::Done,
                    ],
                },
            ),
            (
                "planned".to_string(),
                WorkflowDefinition {
                    description: String::new(),
                    stages: vec![
                        Stage::Planning,
                        Stage::Implementing,
                        Stage::Ready,
                        Stage::Done,
                    ],
                },
            ),
        ])),
        ..Settings::default()
    };
    Workflows::from_settings(&settings)
}

fn only(agent: Agent, strength: Strength) -> Option<StageSetting> {
    Some(StageSetting {
        provider: Some(ProviderChoice::Only(agent)),
        strength: Some(strength),
        ..StageSetting::default()
    })
}

/// Claude talks and Codex plans, on their own strengths; Claude's models are `sonnet`, `haiku` and `opus`.
fn scene_with(
    talking: Option<StageSetting>,
    planning: Option<StageSetting>,
    agents: AgentSettings,
) -> Scene {
    let agents = AgentSettings {
        claude: ClaudeSettings {
            strong: Some("opus".to_string()),
            medium: Some("sonnet".to_string()),
            fast: Some("haiku".to_string()),
            ..agents.claude.clone()
        },
        ..agents
    };
    Scene::with_setup(Setup {
        stages: StageSettings {
            talking,
            planning,
            ..StageSettings::default()
        },
        workflows: two_workflows(),
        ..setup(agents)
    })
}

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

lets_expect! { #tokio_test
    // requirement: a thread can start on a workflow the person chose
    story a_thread_starts_on_the_named_workflow {
        let scene = scene_with(None, None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", Some("planned")).unwrap();
        expect((thread.workflow, thread.stage)) to equal(("planned".to_string(), Stage::Planning))
    }

    story a_thread_without_a_workflow_starts_on_the_default {
        let scene = scene_with(None, None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let plain = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", None).unwrap();
        let blank = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", Some("  ")).unwrap();
        expect((plain.workflow, blank.workflow)) to equal(("default".to_string(), "default".to_string()))
    }

    story a_workflow_nobody_set_up_is_refused {
        let scene = scene_with(None, None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let refused = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", Some("nonsense"))
            .map_err(|problem| problem.to_string());
        expect(refused.err().map(|text| text.starts_with("There is no workflow `nonsense`."))) to equal(Some(true))
        expect(scene.threads().len()) to equal(0)
    }

    // requirement: the first stage of the chosen workflow chooses the agent and model, not the default workflow's
    story the_chosen_workflows_first_stage_picks_the_agent {
        let scene = scene_with(only(Agent::Claude, Strength::Medium), only(Agent::Codex, Strength::Medium), fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let on_default = scene.workspace.start_thread_on(project.id, None, None, None, "", None).unwrap();
        let on_planned = scene.workspace.start_thread_on(project.id, None, None, None, "", Some("planned")).unwrap();
        expect((on_default.agent.as_str(), on_default.model)) to equal(("claude", Some("sonnet".to_string())))
        expect((on_planned.agent.as_str(), on_planned.model, on_planned.stage)) to equal(("codex", None, Stage::Planning))
        expect(on_planned.picks.get(&Stage::Planning).cloned()) to equal(Some("codex".to_string()))
    }

    // requirement: a default workflow whose provider is missing does not stop a thread on another workflow
    story another_workflow_starts_while_the_defaults_provider_is_missing {
        let scene = scene_with(only(Agent::Grok, Strength::Medium), only(Agent::Claude, Strength::Fast), without_grok());
        let folder = project_folder();
        let project = scene.project(&folder);
        let refused = scene.workspace.start_thread_on(project.id, None, None, None, "", None).map_err(|problem| problem.to_string());
        let started = scene.workspace.start_thread_on(project.id, None, None, None, "", Some("planned"));
        expect(refused.is_err()) to be_true
        expect(started.map(|thread| (thread.agent, thread.model)).ok()) to equal(Some(("claude".to_string(), Some("haiku".to_string()))))
    }

    // requirement: starting on another workflow leaves the choice shown for new threads as it was
    story another_workflow_leaves_the_shown_choice_alone {
        let scene = scene_with(only(Agent::Claude, Strength::Medium), only(Agent::Codex, Strength::Medium), fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let shown = starting(&scene.workspace);
        scene.workspace.start_thread_on(project.id, None, None, None, "", Some("planned")).unwrap();
        expect(starting(&scene.workspace)) to equal(shown)
    }

    // requirement: an agent the person picked runs the model its stage names for the stage's strength
    story a_picked_agent_runs_the_stages_model {
        let scene = scene_with(only(Agent::Claude, Strength::Fast), None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", None).unwrap();
        expect(thread.model) to equal(Some("haiku".to_string()))
    }

    story a_stage_without_a_strength_names_medium {
        let scene = scene_with(Some(StageSetting { provider: Some(ProviderChoice::Any), ..StageSetting::default() }), None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", None).unwrap();
        expect(thread.model) to equal(Some("sonnet".to_string()))
    }

    story a_picked_model_is_kept {
        let scene = scene_with(only(Agent::Claude, Strength::Fast), None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread_on(project.id, Some("claude"), Some("opus"), None, "", None).unwrap();
        expect(thread.model) to equal(Some("opus".to_string()))
    }

    story the_chosen_workflows_stage_names_the_model_of_a_picked_agent {
        let scene = scene_with(only(Agent::Claude, Strength::Fast), only(Agent::Claude, Strength::Strong), fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", Some("planned")).unwrap();
        expect(thread.model) to equal(Some("opus".to_string()))
    }

    story without_a_choosing_stage_a_picked_agent_runs_its_default {
        let scene = scene_with(None, None, fake_agents());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread_on(project.id, Some("claude"), None, None, "", None).unwrap();
        expect(thread.model) to equal(None)
    }
}
