//! Specs for what turns use: the usage log and each agent's limits.
//!
//! The fake agents replay recorded turns. Claude's recording reports its
//! limits; Codex's limits are read from a session file the test puts in a
//! Codex home folder of its own.

mod common;

use std::fs::{create_dir_all, write};
use std::path::Path;
use std::time::Duration;

use lets_expect::lets_expect;
use tempfile::{tempdir, TempDir};
use tokio::time::sleep;

use common::{fake_agents, project_folder, setup, Scene, CODEX_CONVERSATION};

/// The model the recorded Claude turn names on its first line.
const CLAUDE_MODEL: &str = "claude-fable-5-2";
use teryon_core::protocol::Event;
use teryon_core::usage::{Limits, UsageLine, UsageReport};

/// The recorded Codex session, which names its limits on its last `token_count` line.
const SESSION: &str = include_str!("samples/agents/codex-session.jsonl");

/// A scene whose Codex keeps its files in `home`.
fn scene_with_codex_home(home: &Path) -> Scene {
    let mut settings = fake_agents();
    settings.codex.home = Some(home.to_path_buf());
    Scene::with_setup(setup(settings))
}

/// A Codex home holding the session file of the recorded conversation.
fn codex_home_with_session() -> TempDir {
    let home = tempdir().unwrap();
    let day = home.path().join("sessions/2026/11/20");
    create_dir_all(&day).unwrap();
    write(
        day.join(format!(
            "rollout-2026-11-20T23-01-09-{CODEX_CONVERSATION}.jsonl"
        )),
        SESSION,
    )
    .unwrap();
    home
}

fn report(scene: &Scene) -> UsageReport {
    scene.workspace.usage().expect("the usage")
}

/// Waits for an agent's limits, which Grok's arrive with after its thread has finished.
async fn limits_of(scene: &Scene, agent: &str) -> Limits {
    for _ in 0..200 {
        if let Some(limits) = report(scene)
            .limits
            .into_iter()
            .find(|limits| limits.agent == agent)
        {
            return limits;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("{agent}'s limits did not arrive in time");
}

/// The agents with limits, in order.
fn agents(report: &UsageReport) -> Vec<String> {
    report
        .limits
        .iter()
        .map(|limits| limits.agent.clone())
        .collect()
}

fn window_names(limits: &Limits) -> Vec<String> {
    limits
        .windows
        .iter()
        .map(|window| window.name.clone())
        .collect()
}

/// The agent, project name and tokens of each line.
fn lines(report: &UsageReport) -> Vec<(String, String, u64, u64)> {
    report
        .lines
        .iter()
        .map(|line: &UsageLine| {
            (
                line.agent.clone(),
                line.project_name.clone(),
                line.read,
                line.written,
            )
        })
        .collect()
}

fn line(agent: &str, read: u64, written: u64) -> (String, String, u64, u64) {
    (agent.to_string(), "Sample".to_string(), read, written)
}

/// A window's name and whether it is used up.
fn pair(name: &str, reached: bool) -> (String, bool) {
    (name.to_string(), reached)
}

fn reached(limits: &Limits) -> Vec<(String, bool)> {
    limits
        .windows
        .iter()
        .map(|window| (window.name.clone(), window.reached))
        .collect()
}

/// Gemini's limits once `ready`, which arrive after the thread has finished.
async fn gemini_until(scene: &Scene, ready: impl Fn(&Limits) -> bool) -> Limits {
    for _ in 0..200 {
        if let Some(limits) = report(scene)
            .limits
            .into_iter()
            .find(|limits| limits.agent == "gemini")
        {
            if ready(&limits) {
                return limits;
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "gemini's limits did not arrive in time: {:?}",
        report(scene).limits
    );
}

lets_expect! { #tokio_test
    // requirement: with nothing run yet there are no limits and no turns
    story usage_before_any_turn {
        let scene = Scene::new();
        let report = report(&scene);
        expect(report.limits.len()) to equal(0)
        expect(report.lines.len()) to equal(0)
    }

    // requirement: a Claude turn adds a line to the log and keeps Claude's limits from its rate_limit_event
    story usage_after_a_claude_turn {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let report = report(&scene);
        expect(lines(&report)) to equal(vec![line("claude", 24528, 4)])
        expect(report.lines[0].thread_id) to equal(thread.id)
        expect(report.lines[0].project_id) to equal(project.id)
        expect(report.lines[0].model.clone()) to equal(Some(CLAUDE_MODEL.to_string()))
        expect(agents(&report)) to equal(vec!["claude".to_string()])
        expect(window_names(&report.limits[0])) to equal(vec!["five_hour".to_string(), "seven_day".to_string(), "seven_day_overage_included".to_string()])
    }

    // requirement: Codex's limits are read from its session file after the turn, with its plan
    story usage_after_a_codex_turn {
        let codex_home = codex_home_with_session();
        let scene = scene_with_codex_home(codex_home.path());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "codex");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let report = report(&scene);
        expect(lines(&report)) to equal(vec![line("codex", 86322, 229)])
        expect(agents(&report)) to equal(vec!["codex".to_string()])
        expect(report.limits[0].plan.clone()) to equal(Some("Plus".to_string()))
        expect(window_names(&report.limits[0])) to equal(vec!["primary".to_string(), "secondary".to_string()])
        expect(report.limits[0].failed) to be_false
    }

    // requirement: when Codex's session file cannot be found, its limits are marked as unreadable
    story usage_after_a_codex_turn_without_its_session_file {
        let codex_home = tempdir().unwrap();
        let scene = scene_with_codex_home(codex_home.path());
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "codex");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let report = report(&scene);
        expect(lines(&report)) to equal(vec![line("codex", 86322, 229)])
        expect(report.limits[0].failed) to be_true
        expect(report.limits[0].windows.len()) to equal(0)
    }

    // requirement: Grok's turns are logged; without Grok's sign-in its limits cannot be read, and the screen says to sign in
    story usage_after_a_grok_turn {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "grok");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let report = report(&scene);
        let limits = limits_of(&scene, "grok").await;
        expect(lines(&report)) to equal(vec![line("grok", 18779, 31)])
        expect(limits.failed) to be_true
        expect(limits.windows.len()) to equal(0)
        expect(limits.advice.as_deref()) to equal(Some("Run grok login in a terminal so Teryon can read Grok's limits."))
    }

    // requirement: a turn that never reported its end is not logged
    story usage_after_a_failed_turn {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "please fail").unwrap();
        scene.wait_until_idle(&thread).await;

        expect(report(&scene).lines.len()) to equal(0)
    }

    // requirement: after each turn, clients hear of the new log line and the new limits, before the thread is free again
    story usage_is_announced {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        let mut changes = scene.workspace.subscribe();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let mut names = Vec::new();
        while let Ok(event) = changes.try_recv() {
            match event {
                Event::UsageRecorded { line } => names.push(format!("usage_recorded {}", line.written)),
                Event::LimitsChanged { limits } => names.push(format!("limits_changed {}", limits.agent)),
                Event::ThreadChanged { .. } => names.push("thread_changed".to_string()),
                _ => {}
            }
        }
        expect(names.iter().rev().take(3).rev().cloned().collect::<Vec<_>>()) to equal(vec![
            "usage_recorded 4".to_string(),
            "limits_changed claude".to_string(),
            "thread_changed".to_string(),
        ])
    }

    // requirement: after a Gemini turn, Antigravity's limits are read; a short rate limit leaves them, and a quota error marks the 5-hour window used up until the next reading
    story gemini_limits_are_read_after_a_turn {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "gemini");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        let limits = gemini_until(&scene, |limits| limits.windows.len() == 2 && !limits.failed).await;
        expect(limits.failed) to be_false
        expect(reached(&limits)) to equal(vec![pair("gemini-5h", false), pair("gemini-weekly", false)])
        let seen = limits.taken_at;

        scene.say(&thread, "hit rate limit").unwrap();
        scene.wait_until_idle(&thread).await;
        let limits = gemini_until(&scene, |limits| limits.taken_at > seen && limits.windows.len() == 2 && limits.windows.iter().all(|window| !window.reached)).await;
        expect(limits.failed) to be_false
        expect(reached(&limits)) to equal(vec![pair("gemini-5h", false), pair("gemini-weekly", false)])

        scene.say(&thread, "hit quota").unwrap();
        scene.wait_until_idle(&thread).await;
        let limits = gemini_until(&scene, |limits| limits.windows.first().is_some_and(|window| window.reached)).await;
        expect(limits.failed) to be_false
        expect(reached(&limits)) to equal(vec![pair("gemini-5h", true), pair("gemini-weekly", false)])

        scene.say(&thread, "hello again").unwrap();
        scene.wait_until_idle(&thread).await;
        let limits = gemini_until(&scene, |limits| limits.windows.first().is_some_and(|window| !window.reached)).await;
        expect(limits.failed) to be_false
        expect(reached(&limits)) to equal(vec![pair("gemini-5h", false), pair("gemini-weekly", false)])
    }

    // requirement: limits and the log are kept in the data folder, so they survive a restart
    story usage_is_kept_across_restarts {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let scene = scene.restart(|_| {});
        let report = report(&scene);
        expect(agents(&report)) to equal(vec!["claude".to_string()])
        expect(lines(&report)) to equal(vec![line("claude", 24528, 4)])
    }
}
