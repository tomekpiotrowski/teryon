//! Request specs for managing threads and changes across projects through `teryon`.
//!
//! Each test runs a real service with the fake agents and invokes the compiled
//! `teryon` binary both from a terminal (without `TERYON_THREAD`) and on
//! behalf of another thread (with `TERYON_THREAD`).

mod common;

use std::fs::write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use lets_expect::lets_expect;
use serde_json::Value;
use tempfile::tempdir;
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use uuid::Uuid;

use common::{git, program, project_id, repository, scene_with, wait_for, Scene};
use teryon::client::Client;
use teryon_core::agents::Voice;
use teryon_core::protocol::{Answer, Event, Request};
use teryon_core::store::{Entry, RunEnd, Status, Thread};
use teryon_core::workflow::Stage;

const FAKES: &str = "[agents.codex]\nprogram = \"../core/tests/samples/fake-codex.sh\"\nhome = \"../core/tests/samples/no-codex-home\"\n\n";
const FAST_IMPLEMENTING: &str =
    "[stages.implementing]\nprovider = \"only claude\"\nstrength = \"fast\"\n";
const GROK_REVIEWS: &str = "[stages.review]\nprovider = \"only grok\"\nstrength = \"strong\"\n";

/// Runs the compiled `teryon` binary against `home`, optionally acting for `acting_thread`.
async fn cli(
    home: &Path,
    acting_thread: Option<Uuid>,
    args: &[&str],
    stdin_text: Option<&str>,
) -> Output {
    let home = home.to_path_buf();
    let owned_args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let stdin_owned = stdin_text.map(str::to_string);
    spawn_blocking(move || {
        let mut command = Command::new(program());
        command
            .arg("--home")
            .arg(&home)
            .args(&owned_args)
            .env_remove("TERYON_THREAD")
            .env_remove("TERYON_TURN")
            .env_remove("TERYON_DATA")
            .env_remove("TERYON_HOME")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(thread_id) = acting_thread {
            command.env("TERYON_THREAD", thread_id.to_string());
        }
        if stdin_owned.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        let mut child = command.spawn().expect("teryon starts");
        if let Some(text) = stdin_owned {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("stdin piped")
                .write_all(text.as_bytes())
                .expect("stdin written");
        }
        child.wait_with_output().expect("teryon finishes")
    })
    .await
    .expect("cli task completes")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

async fn add_project(client: &mut Client, name: &str, directory: &Path) -> Uuid {
    project_id(
        client
            .call(Request::AddProject {
                name: name.to_string(),
                directory: directory.to_string_lossy().into_owned(),
            })
            .await
            .unwrap(),
    )
}

async fn start_thread(client: &mut Client, project: Uuid, title: &str) -> Thread {
    match client
        .call(Request::StartThread {
            project_id: project,
            agent: Some("claude".to_string()),
            model: None,
            effort: None,
            title: title.to_string(),
            workflow: None,
        })
        .await
        .unwrap()
    {
        Answer::Thread { thread } => *thread,
        other => panic!("not a thread: {other:?}"),
    }
}

async fn wait_idle(client: &mut Client, thread_id: Uuid) -> Thread {
    match wait_for(client, |event| {
        matches!(event, Event::ThreadChanged { thread } if thread.id == thread_id && thread.status == Status::Idle && thread.unread)
    })
    .await
    {
        Event::ThreadChanged { thread } => thread,
        _ => unreachable!(),
    }
}

async fn entries_of(client: &mut Client, thread_id: Uuid) -> Vec<Entry> {
    match client
        .call(Request::FollowThread { thread_id })
        .await
        .unwrap()
    {
        Answer::Entries { entries } => entries,
        other => panic!("not entries: {other:?}"),
    }
}

fn notice_texts(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.kind == "notice")
        .filter_map(|entry| entry.body["text"].as_str().map(str::to_string))
        .collect()
}

async fn managing_scene(more: &str) -> Scene {
    scene_with(&format!("{FAKES}{more}")).await
}

lets_expect! { #tokio_test
    // requirement: listing and showing work across two projects, with short identifier beginnings and --json
    story listing_and_showing_across_two_projects {
        let scene = managing_scene("").await;
        let first = repository();
        let second = repository();
        let mut client = scene.client().await;
        let alpha = add_project(&mut client, "alpha", first.path()).await;
        let beta = add_project(&mut client, "beta", second.path()).await;
        let t1 = start_thread(&mut client, alpha, "Alpha task").await;
        let t2 = start_thread(&mut client, beta, "Beta task").await;

        client.call(Request::SendMessage { thread_id: t2.id, text: "hello".to_string(), to: Voice::Author }).await.unwrap();
        wait_idle(&mut client, t2.id).await;

        let projects_out = cli(scene.home.path(), None, &["projects"], None).await;
        expect(projects_out.status.success()) to be_true
        expect(stdout(&projects_out).contains("alpha")) to be_true
        expect(stdout(&projects_out).contains("beta")) to be_true

        let all_threads = cli(scene.home.path(), None, &["threads"], None).await;
        expect(all_threads.status.success()) to be_true
        expect(stdout(&all_threads).contains("Alpha task")) to be_true
        expect(stdout(&all_threads).contains("Beta task")) to be_true

        let beta_only = cli(scene.home.path(), None, &["threads", "--project", "beta"], None).await;
        expect(stdout(&beta_only).contains("Beta task")) to be_true
        expect(stdout(&beta_only).contains("Alpha task")) to be_false

        let prefix = &t2.id.to_string()[..6];
        let shown = cli(scene.home.path(), None, &["thread", "show", prefix], None).await;
        expect(shown.status.success()) to be_true
        expect(stdout(&shown).contains("Project: beta")) to be_true
        expect(stdout(&shown).contains("Agent: ready")) to be_true

        let json_out = cli(scene.home.path(), None, &["--json", "thread", "show", prefix], None).await;
        let parsed: Value = serde_json::from_str(&stdout(&json_out)).unwrap();
        expect(parsed["thread"]["id"].as_str()) to equal(Some(t2.id.to_string().as_str()))
        expect(parsed["entries"].as_array().unwrap().is_empty()) to be_false

        // Put away quiet thread t1, then confirm `--all` shows it again.
        let put = cli(scene.home.path(), None, &["threads", "put-away", "--project", "alpha"], None).await;
        expect(put.status.success()) to be_true
        expect(stdout(&put)) to equal("Put away 1 thread.".to_string())

        let visible = cli(scene.home.path(), None, &["threads"], None).await;
        expect(stdout(&visible).contains("Alpha task")) to be_false
        let with_all = cli(scene.home.path(), None, &["threads", "--all"], None).await;
        expect(stdout(&with_all).contains("Alpha task")) to be_true
        expect(t1.project_id) to equal(alpha)
    }

    // requirement: start and send from another thread add `from` attribution and increment rounds instead of resetting them
    story start_and_send_from_another_thread_attributes_and_counts_rounds {
        let scene = managing_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let project = add_project(&mut client, "teryon", folder.path()).await;
        let coordinator = start_thread(&mut client, project, "Coordinator").await;

        let started = cli(
            scene.home.path(),
            Some(coordinator.id),
            &["--json", "thread", "start", "--project", "teryon", "--title", "Worker", "--wait", "hello"],
            None,
        )
        .await;
        expect(started.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&started)).unwrap();
        let worker_id = Uuid::parse_str(payload["thread"]["id"].as_str().unwrap()).unwrap();
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(1))
        let first_msg = payload["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "user_message")
            .unwrap();
        expect(first_msg["body"]["from"]["thread_id"].as_str()) to equal(Some(coordinator.id.to_string().as_str()))
        expect(first_msg["body"]["from"]["project_id"].as_str()) to equal(Some(project.to_string().as_str()))
        expect(first_msg["body"]["from"]["title"].as_str()) to equal(Some("Coordinator"))

        // Second message from coordinator via stdin (`-`) increments rounds to 2.
        let prefix = &worker_id.to_string()[..8];
        let second = cli(
            scene.home.path(),
            Some(coordinator.id),
            &["--json", "thread", "send", prefix, "--wait", "-"],
            Some("follow up from stdin"),
        )
        .await;
        expect(second.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&second)).unwrap();
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(2))

        // A message from the person (no TERYON_THREAD) resets rounds to 0 and has no `from`.
        let person = cli(
            scene.home.path(),
            None,
            &["--json", "thread", "send", prefix, "--wait", "hello from person"],
            None,
        )
        .await;
        expect(person.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&person)).unwrap();
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(0))
        let last_user = payload["entries"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|entry| entry["kind"] == "user_message")
            .unwrap();
        expect(last_user["body"].get("from").is_none()) to be_true
    }

    // requirement: messaging a stopped thread lifts the stop; round limit and own-thread requests are refused
    story stopped_thread_round_limit_and_own_thread_rules {
        let scene = managing_scene("[workflow]\nautomatic_rounds = 1\n").await;
        let folder = repository();
        let mut client = scene.client().await;
        let project = add_project(&mut client, "teryon", folder.path()).await;
        let actor = start_thread(&mut client, project, "Manager").await;
        let target = start_thread(&mut client, project, "Worker").await;

        // Start a slow turn on target and stop it as the person (`held = true`).
        client
            .call(Request::SendMessage {
                thread_id: target.id,
                text: "be slow".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        sleep(Duration::from_millis(200)).await;
        client.call(Request::Stop { thread_id: target.id }).await.unwrap();
        let stopped = wait_idle(&mut client, target.id).await;
        expect(stopped.held) to be_true

        // Another thread messaging the stopped thread is allowed and lifts `held`.
        let target_short = &target.id.to_string()[..8];
        let resumed = cli(
            scene.home.path(),
            Some(actor.id),
            &["--json", "thread", "send", target_short, "--wait", "hello"],
            None,
        )
        .await;
        expect(resumed.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&resumed)).unwrap();
        expect(payload["thread"]["held"].as_bool()) to equal(Some(false))
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(1))

        // A second message from another thread hits `automatic_rounds = 1` and is refused.
        let refused_limit = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "send", target_short, "too many"],
            None,
        )
        .await;
        expect(refused_limit.status.success()) to be_false
        expect(stderr(&refused_limit).contains("limit of 1 automatic round")) to be_true

        // An unknown TERYON_THREAD (e.g. stale terminal variable) is treated as the person:
        // no round-limit refusal, rounds reset to 0, and no `from` attribution.
        let stale_env = cli(
            scene.home.path(),
            Some(Uuid::new_v4()),
            &["--json", "thread", "send", target_short, "--wait", "from person with stale env"],
            None,
        )
        .await;
        expect(stale_env.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&stale_env)).unwrap();
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(0))
        let last_user = payload["entries"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|entry| entry["kind"] == "user_message")
            .unwrap();
        expect(last_user["body"].get("from").is_none()) to be_true

        // Acting on its own thread is refused across all thread and change commands.
        client
            .call(Request::SendMessage {
                thread_id: actor.id,
                text: "start a change fix-self".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        wait_idle(&mut client, actor.id).await;
        let actor_short = &actor.id.to_string()[..8];
        let mut not_refused = Vec::new();
        for command in [
            vec!["thread", "send", actor_short, "hi self"],
            vec!["thread", "stop", actor_short],
            vec!["thread", "agent", actor_short, "--agent", "codex"],
            vec!["thread", "review", actor_short, "--agent", "grok"],
            vec!["thread", "workflow", "small", "--thread", actor_short],
            vec!["change", "apply", actor_short],
            vec!["change", "catch-up", actor_short],
            vec!["change", "discard", actor_short],
            vec!["change", "send-for-review", actor_short],
        ] {
            let refused = cli(scene.home.path(), Some(actor.id), &command, None).await;
            let message = stderr(&refused);
            if refused.status.success()
                || !message.contains("teryon thread title")
                || !message.contains("teryon thread ask")
            {
                not_refused.push(format!("{command:?}: {message}"));
            }
        }
        expect(not_refused) to equal(Vec::<String>::new())

        let refused_wait_self = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "wait", actor_short],
            None,
        )
        .await;
        expect(refused_wait_self.status.success()) to be_false
    }

    // requirement: stopping, applying, discarding and choosing a workflow from another thread leave attributed notices
    story notices_for_stop_apply_discard_and_workflow {
        let scene = managing_scene("").await;
        let folder = repository();
        let mut client = scene.client().await;
        let project = add_project(&mut client, "teryon", folder.path()).await;
        let actor = start_thread(&mut client, project, "Clean up logging").await;

        // 1. Stop notice
        let slow = start_thread(&mut client, project, "Slow job").await;
        client
            .call(Request::SendMessage {
                thread_id: slow.id,
                text: "be slow".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        sleep(Duration::from_millis(200)).await;
        let stop_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "stop", &slow.id.to_string()[..8]],
            None,
        )
        .await;
        expect(stop_out.status.success()) to be_true
        wait_idle(&mut client, slow.id).await;
        expect(notice_texts(&entries_of(&mut client, slow.id).await))
            to equal(vec!["«Clean up logging» stopped this thread.".to_string()])

        // 2. Choose workflow notice
        let wf_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "workflow", "small", "--thread", &slow.id.to_string()[..8]],
            None,
        )
        .await;
        expect(wf_out.status.success()) to be_true
        expect(notice_texts(&entries_of(&mut client, slow.id).await).last().cloned())
            to equal(Some("«Clean up logging» chose the small workflow.".to_string()))

        // 3. Apply notice (drafting description automatically)
        let to_apply = start_thread(&mut client, project, "Apply me").await;
        client
            .call(Request::SendMessage {
                thread_id: to_apply.id,
                text: "start a change fix-apply".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        wait_idle(&mut client, to_apply.id).await;
        let change_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["change", "show", &to_apply.id.to_string()[..8]],
            None,
        )
        .await;
        expect(change_out.status.success()) to be_true
        expect(stdout(&change_out).contains("Change: fix-apply")) to be_true

        let apply_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["change", "apply", &to_apply.id.to_string()[..8]],
            None,
        )
        .await;
        expect(apply_out.status.success()) to be_true
        expect(notice_texts(&entries_of(&mut client, to_apply.id).await).last().cloned())
            to equal(Some("«Clean up logging» applied this change.".to_string()))

        // 4. Discard notice
        let to_discard = start_thread(&mut client, project, "Discard me").await;
        client
            .call(Request::SendMessage {
                thread_id: to_discard.id,
                text: "start a change fix-discard".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        wait_idle(&mut client, to_discard.id).await;
        let discard_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["change", "discard", &to_discard.id.to_string()[..8]],
            None,
        )
        .await;
        expect(discard_out.status.success()) to be_true
        expect(notice_texts(&entries_of(&mut client, to_discard.id).await).last().cloned())
            to equal(Some("«Clean up logging» discarded this change.".to_string()))

        // 5. Stopping an idle thread reports failure and leaves no notice.
        let idle = start_thread(&mut client, project, "Already idle").await;
        let idle_stop = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "stop", &idle.id.to_string()[..8]],
            None,
        )
        .await;
        expect(idle_stop.status.success()) to be_false
        expect(stderr(&idle_stop).contains("is not running.")) to be_true
        expect(notice_texts(&entries_of(&mut client, idle.id).await).is_empty()) to be_true
    }

    // requirement: switching agent, asking for a review, catching up and sending for review leave attributed notices
    story notices_for_agent_review_catch_up_and_send_for_review {
        let github_config = "[github]\nprogram = \"../core/tests/samples/fake-gh.sh\"\n";
        let scene = managing_scene(github_config).await;
        let remote = tempdir().unwrap();
        git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
        let folder = repository();
        let github_url = "https://github.com/owner/app.git";
        git(folder.path(), &["remote", "add", "origin", github_url]);
        let instead = format!("url.{}.insteadOf", remote.path().display());
        git(folder.path(), &["config", &instead, github_url]);
        git(folder.path(), &["push", "-q", "origin", "main"]);

        let mut client = scene.client().await;
        let project = add_project(&mut client, "teryon", folder.path()).await;
        let actor = start_thread(&mut client, project, "Coordinator").await;
        let target = start_thread(&mut client, project, "Feature").await;
        let target_short = &target.id.to_string()[..8];

        // Start a change so review, catch-up and send-for-review can run on it.
        client
            .call(Request::SendMessage {
                thread_id: target.id,
                text: "start a change fix-notices".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        wait_idle(&mut client, target.id).await;

        // 1. Switch agent notice
        let agent_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "agent", target_short, "--agent", "codex"],
            None,
        )
        .await;
        expect(agent_out.status.success()) to be_true
        expect(
            notice_texts(&entries_of(&mut client, target.id).await)
                .iter()
                .any(|text| text == "«Coordinator» switched the agent to Codex.")
        ) to be_true

        // 2. Ask for review notice
        write(folder.path().join(".fake-review-clean"), "").unwrap();
        let review_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["thread", "review", target_short, "--agent", "grok"],
            None,
        )
        .await;
        expect(review_out.status.success()) to be_true
        let waited = cli(scene.home.path(), None, &["thread", "wait", target_short], None).await;
        expect(waited.status.success()) to be_true
        expect(
            notice_texts(&entries_of(&mut client, target.id).await)
                .iter()
                .any(|text| text == "«Coordinator» asked Grok to review this change.")
        ) to be_true

        // 3. Catch up notice
        write(folder.path().join("README.md"), "newer work on main\n").unwrap();
        git(folder.path(), &["commit", "-q", "-am", "newer work"]);
        let catch_up_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["change", "catch-up", target_short],
            None,
        )
        .await;
        expect(catch_up_out.status.success()) to be_true
        expect(
            notice_texts(&entries_of(&mut client, target.id).await)
                .iter()
                .any(|text| text == "«Coordinator» caught this change up.")
        ) to be_true

        // 4. Send for review notice
        let send_review_out = cli(
            scene.home.path(),
            Some(actor.id),
            &["change", "send-for-review", target_short],
            None,
        )
        .await;
        expect(send_review_out.status.success()) to be_true
        expect(
            notice_texts(&entries_of(&mut client, target.id).await)
                .iter()
                .any(|text| text == "«Coordinator» sent this change for review.")
        ) to be_true
    }

    // requirement: a thread at the automatic stage round limit can still be answered by another thread, and automatic_rounds = 0 still allows one message
    story cross_thread_limit_allows_answering_unattended_threads_and_zero_rounds {
        let scene = managing_scene(&format!("[workflow]\nautomatic_rounds = 1\n\n{FAST_IMPLEMENTING}\n{GROK_REVIEWS}")).await;
        let folder = repository();
        write(folder.path().join(".fake-review-clean"), "").unwrap();
        let mut client = scene.client().await;
        let project = add_project(&mut client, "teryon", folder.path()).await;
        let coordinator = start_thread(&mut client, project, "Coordinator").await;
        let worker = start_thread(&mut client, project, "Auto worker").await;
        let worker_short = &worker.id.to_string()[..8];

        // Worker runs an automatic review handover, reaching `rounds == 1 == automatic_rounds`.
        let handover = cli(
            scene.home.path(),
            None,
            &[
                "--json",
                "thread",
                "send",
                worker_short,
                "--wait",
                "start a change fix-unattended and report stage review",
            ],
            None,
        )
        .await;
        expect(handover.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&handover)).unwrap();
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(1))

        // Coordinator can still send one message to worker even though `rounds == automatic_rounds`.
        let answered = cli(
            scene.home.path(),
            Some(coordinator.id),
            &["thread", "send", worker_short, "--wait", "hello"],
            None,
        )
        .await;
        expect(answered.status.success()) to be_true

        // A second consecutive message from coordinator is refused.
        let refused = cli(
            scene.home.path(),
            Some(coordinator.id),
            &["thread", "send", worker_short, "second message"],
            None,
        )
        .await;
        expect(refused.status.success()) to be_false
        expect(stderr(&refused).contains("limit of 1 automatic round from other threads")) to be_true

        // With `automatic_rounds = 0`, one cross-thread message still succeeds and a second is refused.
        let zero_scene = managing_scene("[workflow]\nautomatic_rounds = 0\n").await;
        let zero_folder = repository();
        let mut zero_client = zero_scene.client().await;
        let zero_project = add_project(&mut zero_client, "teryon", zero_folder.path()).await;
        let zero_actor = start_thread(&mut zero_client, zero_project, "Helper").await;
        let zero_target = start_thread(&mut zero_client, zero_project, "Target").await;
        let zero_short = &zero_target.id.to_string()[..8];

        let first = cli(
            zero_scene.home.path(),
            Some(zero_actor.id),
            &["thread", "send", zero_short, "--wait", "hello"],
            None,
        )
        .await;
        expect(first.status.success()) to be_true

        let second = cli(
            zero_scene.home.path(),
            Some(zero_actor.id),
            &["thread", "send", zero_short, "again"],
            None,
        )
        .await;
        expect(second.status.success()) to be_false
    }

    // requirement: wait returns on finished, failed and asking; skips automatic handover; exits 2 on timeout
    story wait_handles_finished_failed_asking_handover_and_timeout {
        let scene = managing_scene(&format!("{FAST_IMPLEMENTING}\n{GROK_REVIEWS}")).await;
        let folder = repository();
        write(folder.path().join(".fake-review-clean"), "").unwrap();
        let mut client = scene.client().await;
        let project = add_project(&mut client, "teryon", folder.path()).await;

        // 1. Finished
        let finished = start_thread(&mut client, project, "Finishes").await;
        let out = cli(
            scene.home.path(),
            None,
            &["thread", "send", &finished.id.to_string()[..8], "--wait", "hello"],
            None,
        )
        .await;
        expect(out.status.success()) to be_true
        expect(stdout(&out).contains("Agent: ready")) to be_true

        // 2. Failed
        let failed = start_thread(&mut client, project, "Fails").await;
        let out = cli(
            scene.home.path(),
            None,
            &["--json", "thread", "send", &failed.id.to_string()[..8], "--wait", "please fail"],
            None,
        )
        .await;
        expect(out.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&out)).unwrap();
        expect(serde_json::from_value::<Option<RunEnd>>(payload["thread"]["ended"].clone()).unwrap())
            to equal(Some(RunEnd::Failed))

        // 3. Asking
        let asking = start_thread(&mut client, project, "Asks").await;
        let out = cli(
            scene.home.path(),
            None,
            &["thread", "send", &asking.id.to_string()[..8], "--wait", "please ask questions"],
            None,
        )
        .await;
        expect(out.status.success()) to be_true
        expect(stdout(&out).contains("waiting for answers")) to be_true
        expect(stdout(&out).contains("Where should answers be kept?")) to be_true

        // 4. Automatic handover to review: `wait` must NOT return at the brief moment between author and reviewer!
        let handover = start_thread(&mut client, project, "Auto review").await;
        let out = cli(
            scene.home.path(),
            None,
            &[
                "--json",
                "thread",
                "send",
                &handover.id.to_string()[..8],
                "--wait",
                "start a change fix-auto and report stage review",
            ],
            None,
        )
        .await;
        expect(out.status.success()) to be_true
        let payload: Value = serde_json::from_str(&stdout(&out)).unwrap();
        expect(serde_json::from_value::<Stage>(payload["thread"]["stage"].clone()).unwrap())
            to equal(Stage::Ready)
        expect(payload["thread"]["rounds"].as_u64()) to equal(Some(1))

        // 5. Timeout exits with code 2 and prints working threads
        let slow = start_thread(&mut client, project, "Slow runner").await;
        client
            .call(Request::SendMessage {
                thread_id: slow.id,
                text: "be slow".to_string(),
                to: Voice::Author,
            })
            .await
            .unwrap();
        let timed = cli(
            scene.home.path(),
            None,
            &["thread", "wait", &slow.id.to_string()[..8], "--timeout", "300ms"],
            None,
        )
        .await;
        expect(timed.status.code()) to equal(Some(2))
        expect(stdout(&timed).contains("Still working:")) to be_true
        expect(stdout(&timed).contains("Slow runner")) to be_true
        client.call(Request::Stop { thread_id: slow.id }).await.unwrap();
    }
}
