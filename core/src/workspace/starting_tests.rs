//! Specs for the held choice of who the next new thread starts with.
//!
//! They drive the hooks that keep it current, such as an agent's limits
//! changing, without taking a snapshot, because a snapshot refreshes the
//! choice by itself and would hide a missing hook.

use chrono::Utc;
use lets_expect::lets_expect;
use tempfile::{tempdir, TempDir};
use tokio::sync::broadcast::Receiver;
use uuid::Uuid;

use super::{Setup, Workspace};
use crate::agents::Agent;
use crate::protocol::{Event, Starting};
use crate::settings::{AgentSettings, ProviderChoice, StageSetting, StageSettings, Strength};
use crate::usage::{Limits, Reading, Window};

/// Fake programs, so every provider counts as installed.
fn agents() -> AgentSettings {
    let mut agents = AgentSettings::default();
    agents.claude.program = "tests/samples/fake-claude.sh".to_string();
    agents.codex.program = "tests/samples/fake-codex.sh".to_string();
    agents.grok.program = "tests/samples/fake-grok.sh".to_string();
    agents
}

/// A workspace whose first stage chooses as given.
fn workspace_choosing(provider: ProviderChoice) -> (Workspace, TempDir) {
    let home = tempdir().unwrap();
    let setup = Setup {
        agents: agents(),
        stages: StageSettings {
            talking: Some(StageSetting {
                provider: Some(provider),
                strength: Some(Strength::Medium),
                ..StageSetting::default()
            }),
            ..StageSettings::default()
        },
        ..Setup::default()
    };
    (Workspace::open(home.path(), setup).unwrap(), home)
}

/// The provider held for the next thread, without taking a snapshot.
fn held(workspace: &Workspace) -> Option<String> {
    match workspace.inner.starting().as_ref() {
        Some(Ok(pick)) => Some(pick.agent.name().to_string()),
        _ => None,
    }
}

/// Starts the workspace's holding, as the first snapshot does.
fn begun(workspace: &Workspace) -> Receiver<Event> {
    workspace.inner.refresh_starting();
    workspace.subscribe()
}

fn limits(agent: Agent, reached: bool) -> Reading {
    Reading::Fresh(Limits {
        agent: agent.name().to_string(),
        plan: None,
        windows: vec![Window {
            name: "five_hour".to_string(),
            label: "5 hours".to_string(),
            minutes: Some(300),
            used: if reached { 100.0 } else { 10.0 },
            resets_at: None,
            reached,
        }],
        taken_at: Utc::now(),
        failed: false,
        advice: None,
    })
}

fn note(workspace: &Workspace, agent: Agent, reached: bool) {
    workspace
        .inner
        .note_limits(agent, limits(agent, reached), false, Utc::now());
}

/// What the announced `StartingChanged` events said, in order.
fn announced(changes: &mut Receiver<Event>) -> Vec<Option<Starting>> {
    let mut said = Vec::new();
    while let Ok(event) = changes.try_recv() {
        if let Event::StartingChanged { starting } = event {
            said.push(starting);
        }
    }
    said
}

/// Whether the last announcement names the provider held.
fn latest_is_held(changes: &mut Receiver<Event>, workspace: &Workspace) -> bool {
    let said = announced(changes);
    let provider = held(workspace);
    matches!(said.last(), Some(Some(Starting::Provider { agent, .. })) if Some(agent) == provider.as_ref())
}

/// Starting a thread on the held choice in a folder that is no project.
fn start_in_unknown_project(workspace: &Workspace) -> bool {
    workspace
        .start_thread(Uuid::new_v4(), None, None, "")
        .is_err()
}

/// Starts threads on the held choice, `times` times.
///
/// Returns how many times the held choice changed afterwards, and whether
/// every change, and only a change, was announced.
fn starts_changing(
    workspace: &Workspace,
    changes: &mut Receiver<Event>,
    folder: &TempDir,
    times: usize,
) -> (usize, bool) {
    let project = workspace
        .add_project("Sample", &folder.path().to_string_lossy())
        .unwrap();
    announced(changes);
    let (mut changed, mut told) = (0, true);
    for _ in 0..times {
        let before = held(workspace);
        workspace.start_thread(project.id, None, None, "").unwrap();
        let said = announced(changes);
        match held(workspace) != before {
            true => {
                changed += 1;
                told &= said.len() == 1 && latest_matches(&said, workspace);
            }
            false => told &= said.is_empty(),
        }
    }
    (changed, told)
}

/// Whether the last announcement names the provider held.
fn latest_matches(said: &[Option<Starting>], workspace: &Workspace) -> bool {
    let provider = held(workspace);
    matches!(said.last(), Some(Some(Starting::Provider { agent, .. })) if Some(agent) == provider.as_ref())
}

/// A workspace that saves its settings in a folder of its own.
fn saving_workspace() -> (Workspace, TempDir, TempDir) {
    let home = tempdir().unwrap();
    let config = tempdir().unwrap();
    let setup = Setup {
        agents: agents(),
        config_folder: Some(config.path().to_path_buf()),
        ..Setup::default()
    };
    (Workspace::open(home.path(), setup).unwrap(), home, config)
}

/// Saves a different Claude medium model while a thread is being started, which holds the choice's lock.
///
/// Returns Claude's medium model as seen while the lock was held, and after it was released.
fn model_while_starting(workspace: &Workspace) -> (Option<String>, Option<String>) {
    let medium = |workspace: &Workspace| workspace.inner.setup().agents.claude.medium.clone();
    let mut configuration = workspace.configuration();
    configuration.claude.medium = "saved-model".to_string();
    let held = workspace.inner.starting();
    let saving = workspace.clone();
    let save = std::thread::spawn(move || saving.save_configuration(&configuration).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(300));
    let during = medium(workspace);
    drop(held);
    save.join().unwrap();
    (during, medium(workspace))
}

lets_expect! {
    // requirement: settings are applied under the choice's lock, so a thread starting never mixes two sets of settings
    story saving_settings_waits_for_a_thread_being_started {
        let (workspace, _home, _config) = saving_workspace();
        let (during, after) = model_while_starting(&workspace);
        expect(during.as_deref() == Some("saved-model")) to be_false
        expect(after) to equal(Some("saved-model".to_string()))
    }

    // requirement: a provider that runs out is replaced, and clients are told once
    story a_used_up_provider_is_replaced {
        let (workspace, _home) = workspace_choosing(ProviderChoice::Prefer(Agent::Claude));
        let mut changes = begun(&workspace);
        expect(held(&workspace)) to equal(Some("claude".to_string()))

        note(&workspace, Agent::Claude, true);
        expect(held(&workspace).is_some_and(|agent| agent != "claude")) to be_true
        expect(latest_is_held(&mut changes, &workspace)) to be_true
    }

    // requirement: the preferred provider returning replaces the fallback
    story the_preferred_provider_returns {
        let (workspace, _home) = workspace_choosing(ProviderChoice::Prefer(Agent::Claude));
        begun(&workspace);
        note(&workspace, Agent::Claude, true);
        let mut changes = workspace.subscribe();

        note(&workspace, Agent::Claude, false);
        expect(held(&workspace)) to equal(Some("claude".to_string()))
        expect(latest_is_held(&mut changes, &workspace)) to be_true
    }

    // requirement: a limits reading that leaves the choice usable neither replaces it nor announces anything
    story a_reading_that_changes_nothing_is_quiet {
        let (workspace, _home) = workspace_choosing(ProviderChoice::Any);
        let mut changes = begun(&workspace);
        let before = held(&workspace);

        note(&workspace, Agent::Claude, false);
        note(&workspace, Agent::Codex, false);
        note(&workspace, Agent::Grok, false);
        expect(held(&workspace)) to equal(before)
        expect(announced(&mut changes).len()) to equal(0)
    }

    // requirement: a thread that fails to start does not use up the choice
    story a_failed_start_keeps_the_choice {
        let (workspace, _home) = workspace_choosing(ProviderChoice::Any);
        let mut changes = begun(&workspace);
        let before = held(&workspace);

        expect(start_in_unknown_project(&workspace)) to be_true
        expect(held(&workspace)) to equal(before)
        expect(announced(&mut changes).len()) to equal(0)
    }

    // requirement: after a thread starts on the choice, a fresh one is chosen and announced when it differs
    story a_started_thread_is_followed_by_a_fresh_choice {
        let (workspace, _home) = workspace_choosing(ProviderChoice::Any);
        let mut changes = begun(&workspace);
        let folder = tempdir().unwrap();

        // Keeping the choice would never change it, and a fresh pick among three
        // differs almost every other time, so thirty starts leave no doubt.
        let (changed, told_every_time) = starts_changing(&workspace, &mut changes, &folder, 30);
        expect(changed > 0) to be_true
        expect(told_every_time) to be_true
    }
}
