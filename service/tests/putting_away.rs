//! Request specs for taking quiet threads off the board, and for a finish leaving after it was shown.
//!
//! The rules for who is quiet live in the core library. These cover the two
//! requests: putting away only the quiet threads in scope, and marking a
//! finish seen only when the board showed that finish.

mod common;

use std::fs;

use chrono::{Duration, Utc};
use lets_expect::lets_expect;
use tempfile::TempDir;
use uuid::Uuid;

use common::{project_id, repository, scene, thread_id, wait_for, Scene};
use teryon::client::Client;
use teryon_core::agents::Voice;
use teryon_core::protocol::{Answer, DoneSeen, Event, Request};
use teryon_core::store::{Status, Thread};
use teryon_core::workflow::Stage;

/// A project in `folder` named `name`, and one Claude thread in it.
async fn project_and_thread(client: &mut Client, folder: &TempDir, name: &str) -> (Uuid, Uuid) {
    let project = project_id(
        client
            .call(Request::AddProject {
                name: name.to_string(),
                directory: folder.path().to_string_lossy().into_owned(),
            })
            .await
            .unwrap(),
    );
    (project, another_thread(client, project).await)
}

async fn another_thread(client: &mut Client, project: Uuid) -> Uuid {
    thread_id(
        client
            .call(Request::StartThread {
                project_id: project,
                agent: Some("claude".to_string()),
                model: None,
                title: String::new(),
                workflow: None,
            })
            .await
            .unwrap(),
    )
}

/// Sends a message and waits until that turn has ended.
///
/// Moving back from Done is announced before the thread is marked running,
/// and that announcement is still idle. Waiting for running first skips it,
/// so the idle turn afterwards is the one this message started.
async fn turn(client: &mut Client, thread: Uuid, text: &str) -> Thread {
    client
        .call(Request::SendMessage {
            thread_id: thread,
            text: text.to_string(),
            to: Voice::Author,
        })
        .await
        .unwrap_or_else(|problem| panic!("sending {text:?}: {problem}"));
    wait_for(client, |event| {
        matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.status == Status::Running)
    })
    .await;
    match wait_for(client, |event| {
        matches!(event, Event::ThreadChanged { thread: changed } if changed.id == thread && changed.status == Status::Idle && changed.unread)
    })
    .await
    {
        Event::ThreadChanged { thread } => thread,
        _ => unreachable!(),
    }
}

async fn put_away(client: &mut Client, project: Option<Uuid>) -> u32 {
    match client
        .call(Request::PutAwayQuiet {
            project_id: project,
        })
        .await
        .unwrap()
    {
        Answer::PutAway { count } => count,
        other => panic!("not a count: {other:?}"),
    }
}

/// Every thread, read by a client that has not seen the earlier events.
async fn threads_of(scene: &Scene) -> Vec<Thread> {
    let mut client = scene.client().await;
    match client.event().await.unwrap() {
        Event::Snapshot { threads, .. } => threads,
        other => panic!("not a snapshot: {other:?}"),
    }
}

fn thread_of(threads: &[Thread], id: Uuid) -> Thread {
    threads
        .iter()
        .find(|thread| thread.id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .clone()
}

lets_expect! { #tokio_test
    // requirement: put away applies only to quiet threads in the given project, or in every project when no project is given
    story putting_away_marks_only_quiet_threads {
        let scene = scene().await;
        let sample_folder = repository();
        let other_folder = tempfile::tempdir().unwrap();
        let mut client = scene.client().await;
        let (sample, fresh) = project_and_thread(&mut client, &sample_folder, "Sample").await;
        let hello = another_thread(&mut client, sample).await;
        let slow = another_thread(&mut client, sample).await;
        let asking = another_thread(&mut client, sample).await;
        let changed = another_thread(&mut client, sample).await;
        let planning = another_thread(&mut client, sample).await;
        let (other, elsewhere) = project_and_thread(&mut client, &other_folder, "Other").await;
        let planned = another_thread(&mut client, other).await;

        let unread = turn(&mut client, hello, "hello").await;
        client.call(Request::SendMessage { thread_id: slow, text: "be slow".to_string(), to: Voice::Author }).await.unwrap();
        let running = match wait_for(&mut client, |event| {
            matches!(event, Event::ThreadChanged { thread } if thread.id == slow && thread.status == Status::Running)
        }).await {
            Event::ThreadChanged { thread } => thread,
            _ => unreachable!(),
        };
        let waiting = turn(&mut client, asking, "ask questions").await;
        let with_change = turn(&mut client, changed, "start a change fix-login").await;
        let moved = turn(&mut client, planning, "report stage planning").await;
        client.call(Request::ChooseWorkflow { thread_id: planned, turn: None, workflow: "small".to_string() }).await.unwrap();
        // A plan on a workflow without planning stays in Talking. The agent is not told where to write it until one exists, so the file is written directly and noticed when the thread is opened.
        let plan_file = other_folder.path().join(".teryon/threads").join(planned.to_string()).join("plan.md");
        fs::write(&plan_file, "# Login\n\nSteps for Login.\n").unwrap();
        client.call(Request::FollowThread { thread_id: planned }).await.unwrap();
        let with_plan = match wait_for(&mut client, |event| {
            matches!(event, Event::ThreadChanged { thread } if thread.id == planned && thread.plan.is_some())
        }).await {
            Event::ThreadChanged { thread } => thread,
            _ => unreachable!(),
        };

        expect(unread.unread) to be_true
        expect(unread.stage) to equal(Stage::Talking)
        expect(running.status) to equal(Status::Running)
        expect(waiting.asking) to be_true
        expect(with_change.change.is_some()) to be_true
        expect(with_change.stage) to equal(Stage::Talking)
        expect(moved.stage) to equal(Stage::Planning)
        expect(with_plan.stage) to equal(Stage::Talking)
        expect(with_plan.plan.is_some()) to be_true

        let in_sample = put_away(&mut client, Some(sample)).await;
        let again = put_away(&mut client, Some(sample)).await;
        let everywhere = put_away(&mut client, None).await;
        let refused = client.call(Request::PutAwayQuiet { project_id: Some(Uuid::new_v4()) }).await;

        expect(in_sample) to equal(2)
        expect(again) to equal(0)
        expect(everywhere) to equal(1)
        expect(refused.map(|_| ()).map_err(|problem| problem.to_string())) to equal(Err("There is no such project.".to_string()))

        client.call(Request::Stop { thread_id: slow }).await.unwrap();
        let stopped = match wait_for(&mut client, |event| {
            matches!(event, Event::ThreadChanged { thread } if thread.id == slow && thread.status == Status::Idle && thread.held)
        }).await {
            Event::ThreadChanged { thread } => thread,
            _ => unreachable!(),
        };
        let stopped_away = put_away(&mut client, Some(sample)).await;
        expect(stopped.held) to be_true
        expect(stopped_away) to equal(1)

        let threads = threads_of(&scene).await;
        let flags = |id| thread_of(&threads, id).put_away;
        expect(flags(fresh)) to be_true
        expect(flags(hello)) to be_true
        expect(flags(slow)) to be_true
        expect(flags(asking)) to be_false
        expect(flags(changed)) to be_false
        expect(flags(planning)) to be_false
        expect(flags(elsewhere)) to be_true
        expect(flags(planned)) to be_false
    }

    // requirement: a person writing, or choosing a workflow, clears put away
    story a_person_brings_a_put_away_thread_back {
        let scene = scene().await;
        let folder = tempfile::tempdir().unwrap();
        let mut client = scene.client().await;
        let (project, written) = project_and_thread(&mut client, &folder, "Sample").await;
        let chosen = another_thread(&mut client, project).await;
        expect(put_away(&mut client, Some(project)).await) to equal(2)

        let after_writing = turn(&mut client, written, "hello again").await;
        let answer = client.call(Request::ChooseWorkflow { thread_id: chosen, turn: None, workflow: "small".to_string() }).await.unwrap();
        let Answer::Thread { thread: after_choosing } = answer else { panic!("not a thread") };

        expect(after_writing.put_away) to be_false
        expect(after_writing.stage) to equal(Stage::Talking)
        expect(after_choosing.put_away) to be_false
        expect(after_choosing.stage) to equal(Stage::Talking)
        expect(after_choosing.workflow.clone()) to equal("small".to_string())
    }

    // requirement: acknowledging a finish marks it seen only when the thread is still Done and the finish is at or before the time the board showed it
    story acknowledging_a_finish_marks_only_what_was_shown {
        let scene = scene().await;
        let folder = repository();
        let mut client = scene.client().await;
        let (_project, finished) = project_and_thread(&mut client, &folder, "Sample").await;
        let talking = another_thread(&mut client, _project).await;
        turn(&mut client, finished, "start a change fix-login").await;
        client.call(Request::DiscardChange { thread_id: finished }).await.unwrap();

        let first = thread_of(&threads_of(&scene).await, finished);
        let entered = first.stage_since.unwrap();
        expect(first.stage) to equal(Stage::Done)
        expect(first.done_seen) to be_false

        client.call(Request::AcknowledgeDone { seen: vec![DoneSeen { thread_id: finished, at: entered - Duration::seconds(1) }] }).await.unwrap();
        expect(thread_of(&threads_of(&scene).await, finished).done_seen) to be_false

        client.call(Request::AcknowledgeDone { seen: vec![DoneSeen { thread_id: finished, at: entered }] }).await.unwrap();
        expect(thread_of(&threads_of(&scene).await, finished).done_seen) to be_true
        client.call(Request::AcknowledgeDone { seen: vec![DoneSeen { thread_id: finished, at: entered }] }).await.unwrap();
        expect(thread_of(&threads_of(&scene).await, finished).done_seen) to be_true

        client.call(Request::AcknowledgeDone { seen: vec![DoneSeen { thread_id: talking, at: Utc::now() }] }).await.unwrap();
        let left = thread_of(&threads_of(&scene).await, talking);
        expect(left.stage) to equal(Stage::Talking)
        expect(left.done_seen) to be_false

        let restarted = turn(&mut client, finished, "hello again").await;
        expect(restarted.stage) to equal(Stage::Talking)
        expect(restarted.done_seen) to be_false

        turn(&mut client, finished, "start a change fix-again").await;
        client.call(Request::DiscardChange { thread_id: finished }).await.unwrap();
        let second = thread_of(&threads_of(&scene).await, finished);
        let again = second.stage_since.unwrap();
        expect(second.stage) to equal(Stage::Done)
        expect(second.done_seen) to be_false

        client.call(Request::AcknowledgeDone { seen: vec![
            DoneSeen { thread_id: Uuid::new_v4(), at: again },
            DoneSeen { thread_id: finished, at: entered },
        ] }).await.unwrap();
        expect(thread_of(&threads_of(&scene).await, finished).done_seen) to be_false

        client.call(Request::AcknowledgeDone { seen: vec![
            DoneSeen { thread_id: Uuid::new_v4(), at: again },
            DoneSeen { thread_id: finished, at: again },
        ] }).await.unwrap();
        expect(thread_of(&threads_of(&scene).await, finished).done_seen) to be_true

        let planning = another_thread(&mut client, _project).await;
        turn(&mut client, planning, "report stage planning").await;
        client.call(Request::AcknowledgeDone { seen: vec![DoneSeen { thread_id: planning, at: Utc::now() }] }).await.unwrap();
        let stayed = thread_of(&threads_of(&scene).await, planning);
        expect(stayed.stage) to equal(Stage::Planning)
        expect(stayed.done_seen) to be_false
    }
}
