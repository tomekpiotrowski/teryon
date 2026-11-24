//! Request specs for thread stages: agents reporting them through `teryon thread stage`, choosing workflows, and Teryon starting the next stage's agent by itself.
//!
//! The fake agent runs the real `teryon` program, as in the other request
//! specs. The rules themselves are covered by the core library's unit specs
//! (`workflow`, `workflow::picking`); these cover what a turn ending does with
//! them, which is where a mistake costs most: an agent started by mistake, or
//! a loop between an author and a reviewer.

mod common;

use std::fs::{read_to_string, write};
use std::time::Duration;

use lets_expect::lets_expect;
use tempfile::TempDir;
use tokio::time::sleep;
use uuid::Uuid;

use common::{project_id, repository, scene_with, wait_for, Scene};

use tokio::time::timeout;

use teryon::client::Client;
use teryon_core::agents::models::Effort;
use teryon_core::agents::Voice;
use teryon_core::protocol::{Answer, Event, Request, Starting};
use teryon_core::store::{Entry, Status, Thread};
use teryon_core::workflow::Stage;

/// Every agent program is a fake one, so a stage that may pick any provider never starts a real program.
const FAKES: &str = "[agents.codex]\nprogram = \"../core/tests/samples/fake-codex.sh\"\nhome = \"../core/tests/samples/no-codex-home\"\n\n";

/// Implementing goes to Claude on its fast model.
const FAST_IMPLEMENTING: &str =
    "[stages.implementing]\nprovider = \"only claude\"\nstrength = \"fast\"\n";

/// Reviews go to Grok.
const GROK_REVIEWS: &str = "[stages.review]\nprovider = \"only grok\"\nstrength = \"strong\"\n";

async fn stages_scene(more: &str) -> Scene {
    scene_with(&format!("{FAKES}{more}")).await
}

/// A project in `folder` with one Claude thread.
async fn thread_in(client: &mut Client, folder: &TempDir) -> Uuid {
    started_in(client, folder, Some("claude"), None).await.id
}

/// A project in `folder` with one thread, on the given agent and model, or as the first stage chooses.
async fn started_in(
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
            effort: None,
            title: String::new(),
            workflow: None,
        })
        .await
        .unwrap()
    {
        Answer::Thread { thread } => *thread,
        other => panic!("not a thread: {other:?}"),
    }
}

async fn say(client: &mut Client, thread: Uuid, text: &str) {
    client
        .call(Request::SendMessage {
            thread_id: thread,
            text: text.to_string(),
            to: Voice::Author,
        })
        .await
        .unwrap();
}

/// Waits until the thread is idle after `rounds` automatic starts, and gives it.
async fn settled(client: &mut Client, thread: Uuid, rounds: u32) -> Thread {
    match wait_for(client, |event| {
        matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.status == Status::Idle && changed.unread && changed.rounds == rounds)
    })
    .await
    {
        Event::ThreadChanged { thread } => thread,
        _ => unreachable!(),
    }
}

async fn entries(client: &mut Client, thread: Uuid) -> Vec<Entry> {
    match client
        .call(Request::FollowThread { thread_id: thread })
        .await
        .unwrap()
    {
        Answer::Entries { entries } => entries,
        other => panic!("not entries: {other:?}"),
    }
}

fn texts(entries: &[Entry], kind: &str) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .map(|entry| entry.body["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn invocations(folder: &TempDir) -> Vec<String> {
    read_to_string(folder.path().join(".fake-agent-invocations"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// What `teryon` told the fake agent about the stage or workflow it reported, once it is there.
async fn told(folder: &TempDir) -> String {
    for _ in 0..200 {
        if let Ok(text) = read_to_string(folder.path().join(".fake-agent-stage")) {
            if text.ends_with('\n') {
                return text.trim().to_string();
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the fake agent did not report a stage in time");
}

/// Who a fresh snapshot says new threads start with.
async fn starting_for(scene: &Scene) -> Option<Starting> {
    let mut client = scene.client().await;
    match client.event().await.unwrap() {
        Event::Snapshot { starting, .. } => starting,
        other => panic!("not a snapshot: {other:?}"),
    }
}

/// The next `StartingChanged` among the events that follow, within a moment. Nothing when there is none.
async fn next_starting(client: &mut Client) -> Option<Option<Starting>> {
    timeout(Duration::from_millis(500), async {
        loop {
            if let Event::StartingChanged { starting } = client.event().await.unwrap() {
                return starting;
            }
        }
    })
    .await
    .ok()
}

/// The settings screen's choices with Claude's medium model replaced.
async fn with_medium(client: &mut Client, model: &str) -> Request {
    let mut configuration = match client.call(Request::Configuration).await.unwrap() {
        Answer::Configuration { configuration } => *configuration,
        other => panic!("not the settings: {other:?}"),
    };
    configuration.claude.medium.model = model.to_string();
    Request::SaveConfiguration {
        configuration: Box::new(configuration),
    }
}

/// Claude on a model, as a snapshot says it.
fn claude_on(model: &str) -> Starting {
    Starting::Provider {
        agent: "claude".to_string(),
        model: Some(model.to_string()),
        effort: None,
    }
}

/// The first stage goes to Claude on its medium model.
const ONLY_CLAUDE_MEDIUM: &str =
    "[stages.talking]\nprovider = \"only claude\"\nstrength = \"medium\"\n";

/// What a snapshot says for the provider and model a thread runs on.
fn on(thread: &Thread) -> Starting {
    Starting::Provider {
        agent: thread.agent.clone(),
        model: thread.model.clone(),
        effort: None,
    }
}

lets_expect! { #tokio_test
    // requirement: saving settings that change the choice tells clients, and saving them again changes nothing
    story saving_settings_updates_the_shown_choice {
        let scene = stages_scene(ONLY_CLAUDE_MEDIUM).await;
        let mut watcher = scene.client().await;
        watcher.event().await.unwrap();
        next_starting(&mut watcher).await;
        let mut client = scene.client().await;

        let save = with_medium(&mut client, "other-model").await;
        client.call(save.clone()).await.unwrap();
        expect(next_starting(&mut watcher).await) to equal(Some(Some(claude_on("other-model"))))
        client.call(save).await.unwrap();
        expect(next_starting(&mut watcher).await) to equal(None)
    }

    // requirement: the provider and model of a new thread are chosen before its message, and the thread starts on them
    story a_new_thread_starts_on_the_shown_choice {
        let scene = stages_scene("[stages.talking]\nprovider = \"any\"\nstrength = \"medium\"\n").await;
        let folder = repository();
        let shown = starting_for(&scene).await;
        let mut client = scene.client().await;
        let thread = started_in(&mut client, &folder, None, None).await;

        expect(shown) to equal(Some(on(&thread)))
        expect(starting_for(&scene).await.is_some()) to be_true
    }

    // requirement: a new thread starts on the default workflow, in its first stage
    story a_new_thread_starts_talking {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "hello").await;
        let after = settled(&mut client, thread, 0).await;

        expect(after.workflow.clone()) to equal("default".to_string())
        expect(after.stage) to equal(Stage::Talking)
        expect(invocations(&folder)[0].contains("It is in `talking` now.")) to be_true
        expect(invocations(&folder)[0].contains("teryon thread workflow <name>")) to be_true
    }

    // requirement: the agent reports a stage with `teryon thread stage`, and the thread says who moved it
    // requirement: a stage without settings keeps the thread's agent, which carries on in its own turn, and is given the new stage's instructions
    // requirement: reporting the stage the thread is already in prints nothing
    story an_agent_reports_a_stage {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage planning").await;
        let after = settled(&mut client, thread, 0).await;

        let reply = told(&folder).await;
        expect(reply.starts_with("You carry on with `planning`. Work out how, write the plan, and ask the person to approve it with `teryon thread ask`.")) to be_true
        expect(reply.contains("plan.md`, not in the project and not in a worktree. Don't create a worktree before the person approves the plan.")) to be_true
        expect(after.stage) to equal(Stage::Planning)
        expect(texts(&entries(&mut client, thread).await, "stage")) to equal(vec!["Talking → Planning · by Claude".to_string()])
        expect(invocations(&folder).len()) to equal(1)

        say(&mut client, thread, "report stage planning").await;
        settled(&mut client, thread, 0).await;
        expect(told(&folder).await) to equal(String::new())
    }

    // requirement: when the stage changes, the stage's agent and model start by themselves once the turn ends
    story the_next_stage_starts_on_its_own_model {
        let scene = stages_scene(FAST_IMPLEMENTING).await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage implementing").await;
        let after = settled(&mut client, thread, 1).await;

        expect(told(&folder).await) to equal("Another agent takes `implementing` once your turn ends. Don't do its work: end your turn now.".to_string())
        expect(after.stage) to equal(Stage::Implementing)
        expect(after.model.clone()) to equal(Some("haiku".to_string()))
        let runs = invocations(&folder);
        expect(runs.len()) to equal(2)
        expect(runs[1].contains("--model haiku")) to be_true
        expect(runs[1].contains("Implement what was discussed above.")) to be_true
        expect(texts(&entries(&mut client, thread).await, "notice").contains(&"Claude · haiku takes Implementing (only Claude).".to_string())) to be_true
    }

    // requirement: when nobody can take the next stage, `teryon thread stage` tells the agent to end its turn and the refusal appears in the thread
    story when_nobody_can_take_the_stage_the_agent_ends_its_turn {
        let scene = scene_with(
            "[agents.codex]\nprogram = \"a-program-nobody-has\"\n\n[stages.planning]\nprovider = \"only codex\"\n",
        )
        .await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage planning").await;
        let after = settled(&mut client, thread, 0).await;

        expect(told(&folder).await) to equal("End your turn now.".to_string())
        expect(after.stage) to equal(Stage::Planning)
        expect(invocations(&folder).len()) to equal(1)
        expect(texts(&entries(&mut client, thread).await, "error").is_empty()) to be_false
    }

    // requirement: reporting review without stage settings tells the agent the person picks it up after the turn
    story reporting_review_without_stage_settings_waits_for_the_person {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage review").await;
        let after = settled(&mut client, thread, 0).await;

        expect(told(&folder).await) to equal("The person picks this up after your turn. End your turn now.".to_string())
        expect(after.stage) to equal(Stage::Review)
        expect(invocations(&folder).len()) to equal(1)
    }

    // requirement: reaching review starts the review stage's agent; findings go back to the author by themselves
    story findings_go_back_to_the_author {
        let scene = stages_scene(GROK_REVIEWS).await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "start a change fix-login and report stage review").await;
        let after = settled(&mut client, thread, 2).await;

        expect(after.stage) to equal(Stage::Implementing)
        expect(after.reviewer.as_ref().map(|reviewer| reviewer.agent.clone())) to equal(Some("grok".to_string()))
        let runs = invocations(&folder);
        expect(runs.len()) to equal(3)
        expect(runs[1].contains("Review the change.")) to be_true
        expect(runs[2].contains("A reviewer looked at the change and found")) to be_true
        expect(texts(&entries(&mut client, thread).await, "stage")) to equal(vec![
            "Talking → In review · by Claude".to_string(),
            "In review → Implementing · 2 findings".to_string(),
        ])
    }

    // requirement: a review that finds nothing moves the thread to Ready to apply, where nobody starts
    story a_clean_review_waits_for_the_person {
        let scene = stages_scene(GROK_REVIEWS).await;
        let folder = repository();
        write(folder.path().join(".fake-review-clean"), "").unwrap();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "start a change fix-login and report stage review").await;
        let after = settled(&mut client, thread, 1).await;
        sleep(Duration::from_millis(300)).await;

        expect(after.stage) to equal(Stage::Ready)
        expect(invocations(&folder).len()) to equal(2)
    }

    // requirement: after `automatic_rounds` starts without the person, Teryon stops and says so
    story the_round_limit_stops_a_loop {
        let scene = stages_scene(&format!("[workflow]\nautomatic_rounds = 1\n\n{GROK_REVIEWS}")).await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "start a change fix-login and report stage review").await;
        settled(&mut client, thread, 1).await;
        sleep(Duration::from_millis(300)).await;

        expect(invocations(&folder).len()) to equal(2)
        let all = entries(&mut client, thread).await;
        expect(texts(&all, "notice").last().cloned()) to equal(Some("Stopped after 1 round without you. Send a message to carry on.".to_string()))
    }

    // requirement: a thread the person stopped starts nothing by itself until they write again
    story a_stopped_thread_starts_nothing {
        let scene = stages_scene(FAST_IMPLEMENTING).await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage implementing and wait").await;
        told(&folder).await;
        client.call(Request::Stop { thread_id: thread }).await.unwrap();
        let after = settled(&mut client, thread, 0).await;
        sleep(Duration::from_millis(300)).await;

        expect(after.stage) to equal(Stage::Implementing)
        expect(after.held) to be_true
        expect(invocations(&folder).len()) to equal(1)
    }

    // requirement: the agent may choose another workflow at the start, and is told when a stage is not in it
    story an_agent_chooses_a_workflow {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "choose workflow small and report stage planning").await;
        let after = settled(&mut client, thread, 0).await;

        expect(after.workflow.clone()) to equal("small".to_string())
        expect(after.stage) to equal(Stage::Talking)
        expect(told(&folder).await) to equal("The `small` workflow skips `planning`. Its stages are `talking`, `implementing`, `ready`, `done`.".to_string())
    }

    // requirement: only finishing a change reaches done
    story an_agent_cannot_report_done {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage done").await;
        let after = settled(&mut client, thread, 0).await;

        expect(after.stage) to equal(Stage::Talking)
        expect(told(&folder).await) to equal("A thread reaches `done` only when its change is applied or discarded.".to_string())
    }

    // requirement: the person can put a thread on another workflow; a stage it skips moves on
    story the_person_chooses_a_workflow {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;
        say(&mut client, thread, "report stage planning").await;
        settled(&mut client, thread, 0).await;

        let answer = client.call(Request::ChooseWorkflow { thread_id: thread, turn: None, workflow: "small".to_string() }).await.unwrap();

        let Answer::Thread { thread: chosen } = answer else { panic!("not a thread") };
        expect(chosen.workflow.clone()) to equal("small".to_string())
        expect(chosen.stage) to equal(Stage::Implementing)
    }

    // requirement: `teryon thread stage` outside an agent's turn is refused
    story a_stage_reported_outside_a_turn_is_refused {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        let refused = client.call(Request::ReportStage { thread_id: thread, turn: Uuid::new_v4(), stage: "planning".to_string() }).await;

        expect(refused.map(|_| ()).map_err(|problem| problem.to_string())) to equal(Err("`teryon thread stage` is for agents working in Teryon, during their turn.".to_string()))
    }

    // requirement: shutting down stops agents without their turns starting the next stage's agent
    story shutting_down_starts_nothing {
        let scene = stages_scene(FAST_IMPLEMENTING).await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage implementing and wait").await;
        told(&folder).await;
        client.call(Request::Shutdown).await.unwrap();
        let stopped = tokio::time::timeout(Duration::from_secs(15), scene.stopped).await;
        sleep(Duration::from_millis(300)).await;

        expect(stopped.is_ok()) to be_true
        expect(invocations(&folder).len()) to equal(1)
    }

    // requirement: a new thread the person picked no agent for gets its first stage's agent and model
    story a_new_thread_starts_on_its_first_stage_agent {
        let scene = stages_scene("[stages.talking]\nprovider = \"only claude\"\nstrength = \"fast\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;

        let thread = started_in(&mut client, &folder, None, None).await;

        expect(thread.agent.clone()) to equal("claude".to_string())
        expect(thread.model.clone()) to equal(Some("haiku".to_string()))
        expect(thread.picks.get(&Stage::Talking).cloned()) to equal(Some("claude".to_string()))
    }

    // requirement: an agent the person picked for a new thread wins over the first stage's
    story a_picked_agent_wins_over_the_first_stage {
        let scene = stages_scene("[stages.talking]\nprovider = \"only claude\"\nstrength = \"fast\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;

        let thread = started_in(&mut client, &folder, Some("grok"), None).await;

        expect(thread.agent.clone()) to equal("grok".to_string())
        expect(thread.model.clone()) to equal(None)
    }

    // requirement: without stage settings a new thread starts on Claude's default model
    story a_new_thread_without_stage_settings_starts_on_claude {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;

        let thread = started_in(&mut client, &folder, None, None).await;

        expect((thread.agent.clone(), thread.model.clone())) to equal(("claude".to_string(), None))
    }

    // requirement: the person moving an idle thread into another stage by choosing a workflow starts that stage's agent
    story a_workflow_chosen_while_idle_starts_the_next_stage {
        let scene = stages_scene(&format!(
            "[workflows.default]\nstages = [\"planning\", \"implementing\", \"ready\", \"done\"]\n\n[workflows.small]\nstages = [\"implementing\", \"ready\", \"done\"]\n\n{FAST_IMPLEMENTING}"
        ))
        .await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        client.call(Request::ChooseWorkflow { thread_id: thread, turn: None, workflow: "small".to_string() }).await.unwrap();
        let after = settled(&mut client, thread, 1).await;

        expect(after.stage) to equal(Stage::Implementing)
        expect(after.model.clone()) to equal(Some("haiku".to_string()))
        let runs = invocations(&folder);
        expect(runs.len()) to equal(1)
        expect(runs[0].contains("Implement what was discussed above.")) to be_true
    }

    // requirement: a stage that picks the agent already working remembers it, without starting a turn
    story keeping_the_agent_remembers_it_for_the_stage {
        let scene = stages_scene("[stages.implementing]\nprovider = \"any\"\namong = [\"claude\"]\nstrength = \"medium\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = started_in(&mut client, &folder, Some("claude"), Some("sonnet")).await.id;

        say(&mut client, thread, "report stage implementing").await;
        settled(&mut client, thread, 0).await;
        let after = wait_for(&mut client, |event| {
            matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.picks.contains_key(&Stage::Implementing))
        })
        .await;

        let Event::ThreadChanged { thread: after } = after else { unreachable!() };
        expect(after.picks.get(&Stage::Implementing).cloned()) to equal(Some("claude".to_string()))
        expect(invocations(&folder).len()) to equal(1)
    }

    // requirement: a thread started from the chooser keeps its agent, versioned model, and effort when the next stage matches by alias with no effort
    story a_chooser_thread_keeps_its_model_and_effort_when_a_stage_matches_by_alias {
        let scene = stages_scene("[stages.implementing]\nprovider = \"only claude\"\nstrength = \"strong\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;
        let project = project_id(
            client
                .call(Request::AddProject {
                    name: "Sample".to_string(),
                    directory: folder.path().to_string_lossy().into_owned(),
                })
                .await
                .unwrap(),
        );
        let Answer::Thread { thread } = client
            .call(Request::StartThread {
                project_id: project,
                agent: Some("claude".to_string()),
                model: Some("claude-opus-5-5".to_string()),
                effort: Some(Effort::Medium),
                title: String::new(),
                workflow: None,
            })
            .await
            .unwrap()
        else {
            panic!("not a thread");
        };

        say(&mut client, thread.id, "report stage implementing").await;
        settled(&mut client, thread.id, 0).await;
        let after = wait_for(&mut client, |event| {
            matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread.id && changed.picks.contains_key(&Stage::Implementing))
        })
        .await;

        let Event::ThreadChanged { thread: after } = after else { unreachable!() };
        expect(after.model.clone()) to equal(Some("claude-opus-5-5".to_string()))
        expect(after.effort) to equal(Some(Effort::Medium))
        expect(invocations(&folder).len()) to equal(1)
        expect(texts(&entries(&mut client, thread.id).await, "notice").is_empty()) to be_true
    }

    // requirement: a stage that configures a different effort for the same model hands over and runs on that effort
    story a_stage_with_a_different_effort_starts_on_that_effort {
        let scene = stages_scene("[stages.implementing]\nprovider = \"only claude\"\nstrength = \"strong\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;
        let Answer::Configuration { mut configuration } = client.call(Request::Configuration).await.unwrap() else {
            panic!("not the settings");
        };
        configuration.claude.strong.effort = Some(Effort::High);
        client
            .call(Request::SaveConfiguration { configuration })
            .await
            .unwrap();
        let project = project_id(
            client
                .call(Request::AddProject {
                    name: "Sample".to_string(),
                    directory: folder.path().to_string_lossy().into_owned(),
                })
                .await
                .unwrap(),
        );
        let Answer::Thread { thread } = client
            .call(Request::StartThread {
                project_id: project,
                agent: Some("claude".to_string()),
                model: Some("claude-opus-5-5".to_string()),
                effort: Some(Effort::Medium),
                title: String::new(),
                workflow: None,
            })
            .await
            .unwrap()
        else {
            panic!("not a thread");
        };

        say(&mut client, thread.id, "report stage implementing").await;
        let after = settled(&mut client, thread.id, 1).await;

        expect(after.stage) to equal(Stage::Implementing)
        expect(after.model.clone()) to equal(Some("claude-opus-5-5".to_string()))
        expect(after.effort) to equal(Some(Effort::High))
        let runs = invocations(&folder);
        expect(runs.len()) to equal(2)
        expect(runs[1].contains("--model claude-opus-5-5 --effort high")) to be_true
    }

    // requirement: a plan written in the thread's plan file shows as a line, and moves a talking thread to planning
    // requirement: a changed plan shows again, an unchanged one doesn't, and the Plan request gives its text
    story a_plan_is_written_and_changed {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "write a plan \"Login fix\"").await;
        let after = settled(&mut client, thread, 0).await;
        expect(after.stage) to equal(Stage::Planning)
        expect(after.plan.as_ref().map(|plan| plan.title.clone())) to equal(Some("Login fix".to_string()))
        let path = folder.path().join(".teryon/threads").join(thread.to_string()).join("plan.md");
        expect(path.is_file()) to be_true

        say(&mut client, thread, "hello").await;
        settled(&mut client, thread, 0).await;
        say(&mut client, thread, "write a plan \"Login fix, second take\"").await;
        settled(&mut client, thread, 0).await;

        let all = entries(&mut client, thread).await;
        expect(texts(&all, "plan")) to equal(vec!["Plan written · Login fix".to_string(), "Plan changed · Login fix, second take".to_string()])
        expect(texts(&all, "stage")) to equal(vec!["Talking → Planning · a plan was written".to_string()])
        let Answer::Plan { text } = client.call(Request::Plan { thread_id: thread }).await.unwrap() else { panic!("not a plan") };
        expect(text) to equal("# Login fix, second take\n\nSteps for Login fix, second take.\n".to_string())
    }

    // requirement: the agent is told where to write the plan only in planning, and not to start a worktree before it is approved
    story the_agent_is_told_where_the_plan_goes {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "report stage planning").await;
        settled(&mut client, thread, 0).await;
        say(&mut client, thread, "hello").await;
        settled(&mut client, thread, 0).await;

        let runs = invocations(&folder);
        expect(runs[0].contains("plan.md")) to be_false
        expect(runs[1].contains("plan.md`, not in the project and not in a worktree. Don't create a worktree before the person approves the plan.")) to be_true
        expect(runs[1].contains(&thread.to_string())) to be_true
    }

    // requirement: a worktree started while planning leaves the thread in planning
    story a_worktree_while_planning_leaves_the_stage {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;
        say(&mut client, thread, "report stage planning").await;
        settled(&mut client, thread, 0).await;

        say(&mut client, thread, "start a change fix-login").await;
        let after = settled(&mut client, thread, 0).await;

        expect(after.change.is_some()) to be_true
        expect(after.stage) to equal(Stage::Planning)
    }

    // requirement: on a workflow without planning, a worktree still moves a talking thread to implementing
    story a_worktree_without_planning_moves_on {
        let scene = stages_scene("[workflow]\ndefault = \"small\"\n").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;

        say(&mut client, thread, "start a change fix-login").await;
        let after = settled(&mut client, thread, 0).await;

        expect(after.stage) to equal(Stage::Implementing)
    }

    // requirement: a plan edited by hand shows once the person opens the thread
    story a_plan_edited_by_hand_shows_on_opening {
        let scene = stages_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;
        say(&mut client, thread, "write a plan \"Login fix\"").await;
        settled(&mut client, thread, 0).await;

        let path = folder.path().join(".teryon/threads").join(thread.to_string()).join("plan.md");
        write(&path, "# Login fix by hand\n").unwrap();
        client.call(Request::UnfollowThread { thread_id: thread }).await.unwrap();
        entries(&mut client, thread).await;
        let changed = wait_for(&mut client, |event| {
            matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.plan.as_ref().is_some_and(|plan| plan.title == "Login fix by hand"))
        })
        .await;
        let shown = matches!(changed, Event::ThreadChanged { .. });

        expect(shown) to be_true
    }

    // requirement: the agent starting implementing by itself is pointed at the plan's file
    story implementing_names_the_plan {
        let scene = stages_scene(FAST_IMPLEMENTING).await;
        let folder = repository();
        let mut client = scene.client().await;
        let thread = thread_in(&mut client, &folder).await;
        say(&mut client, thread, "write a plan \"Login fix\"").await;
        settled(&mut client, thread, 0).await;

        say(&mut client, thread, "report stage implementing").await;
        settled(&mut client, thread, 1).await;

        let runs = invocations(&folder);
        expect(runs.last().unwrap().contains("plan.md` is approved. Implement it.")) to be_true
    }
}
