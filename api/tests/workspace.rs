//! Request specs for projects, threads and running agents.
//!
//! The agent programs are replaced by `tests/samples/fake-agent.sh`, which
//! replays recordings of the real programs and misbehaves on request. Nothing
//! here reaches a model provider.

mod common;

use std::path::PathBuf;

use axum::http::StatusCode;
use lets_expect::lets_expect;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

use common::{invocations, project_folder, scene, text_field};
use teryon::agents::runner::{Runner, TurnRequest};
use teryon::settings::{AgentSettings, ClaudeSettings, GrokSettings};
use teryon::workspace::recovery::release_interrupted_threads;
use teryon::workspace::{project, thread};

fn kinds(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// Settings that start a given program in place of Claude.
fn settings_with(program: &str) -> AgentSettings {
    AgentSettings {
        allowed_folders: Vec::new(),
        stop_grace_seconds: 1,
        claude: ClaudeSettings {
            program: program.to_string(),
            use_subscription: true,
        },
        grok: GrokSettings::default(),
    }
}

lets_expect! { #tokio_test
    // requirement: a signed-in user can create a project with a name and a directory
    story a_project_is_created {
        let scene = scene().await;
        let folder = project_folder();

        let response = scene.post("/api/projects", json!({ "name": "  Teryon ", "directory": folder.path() })).await;
        expect(response.status_code()) to equal(StatusCode::CREATED)

        let project: project::Model = response.json();
        expect(project.name.as_str()) to equal("Teryon")
        expect(PathBuf::from(&project.directory)) to equal(folder.path().canonicalize().unwrap())
    }

    story a_project_needs_someone_signed_in {
        let scene = scene().await;
        let folder = project_folder();

        let response = scene.post_as("", "/api/projects", json!({ "name": "Teryon", "directory": folder.path() })).await;
        expect(response.status_code()) to equal(StatusCode::UNAUTHORIZED)
    }

    // requirement: an empty name is refused
    story a_project_without_a_name_is_refused {
        let scene = scene().await;
        let folder = project_folder();

        let response = scene.post("/api/projects", json!({ "name": "   ", "directory": folder.path() })).await;
        expect(response.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&response, "error")) to equal("A project needs a name.".to_string())
        expect(scene.synchronised(&scene.token, "projects").await.len()) to equal(0)
    }

    // requirement: the directory must exist
    story a_project_in_a_missing_directory_is_refused {
        let scene = scene().await;
        let folder = project_folder();

        let response = scene.post("/api/projects", json!({ "name": "Teryon", "directory": folder.path().join("nothing") })).await;
        expect(response.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&response, "error")) to equal("The directory does not exist.".to_string())
    }

    // requirement: the directory must be inside one of the allowed folders from the configuration
    story a_project_outside_the_allowed_folders_is_refused {
        let scene = scene().await;
        let outside = tempdir().unwrap();

        let response = scene.post("/api/projects", json!({ "name": "Teryon", "directory": outside.path() })).await;
        expect(response.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&response, "error")) to equal("The directory is not inside a folder Teryon is allowed to use.".to_string())
    }

    // requirement: a user sees only their own projects
    // requirement: a user sees only their own threads and their entries
    story a_user_sees_only_their_own_work {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "hello").await;
        scene.wait_until_idle(&thread).await;
        let stranger = scene.someone_else().await;

        expect(scene.synchronised(&scene.token, "projects").await.len()) to equal(1)
        expect(scene.synchronised(&scene.token, "threads").await.len()) to equal(1)
        expect(scene.synchronised(&scene.token, "thread-entries").await.len()) to equal(3)
        expect(scene.synchronised(&stranger, "projects").await.len()) to equal(0)
        expect(scene.synchronised(&stranger, "threads").await.len()) to equal(0)
        expect(scene.synchronised(&stranger, "thread-entries").await.len()) to equal(0)
    }

    // requirement: a user can remove a project; its directory is left untouched
    story a_project_is_removed_with_its_threads {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "hello").await;
        scene.wait_until_idle(&thread).await;

        let response = scene.delete(&format!("/api/projects/{}", project.id)).await;
        expect(response.status_code()) to equal(StatusCode::NO_CONTENT)
        expect(folder.path().is_dir()) to be_true

        let projects = scene.synchronised(&scene.token, "projects").await;
        let threads = scene.synchronised(&scene.token, "threads").await;
        let entries = scene.synchronised(&scene.token, "thread-entries").await;
        expect(projects.iter().filter(|row| row["deleted_at"].is_null()).count()) to equal(0)
        expect(threads.iter().filter(|row| row["deleted_at"].is_null()).count()) to equal(0)
        expect(entries.iter().filter(|row| row["deleted_at"].is_null()).count()) to equal(0)
        expect(entries.len()) to equal(3)

        let again = scene.delete(&format!("/api/projects/{}", project.id)).await;
        expect(again.status_code()) to equal(StatusCode::NOT_FOUND)
        let message = scene.say(&thread, "hello again").await;
        expect(message.status_code()) to equal(StatusCode::NOT_FOUND)
        let new_thread = scene.post(&format!("/api/projects/{}/threads", project.id), json!({ "agent": "claude" })).await;
        expect(new_thread.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    story someone_else_cannot_remove_a_project {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let stranger = scene.someone_else().await;

        let response = scene.delete_as(&stranger, &format!("/api/projects/{}", project.id)).await;
        expect(response.status_code()) to equal(StatusCode::NOT_FOUND)
        let unknown = scene.delete(&format!("/api/projects/{}", Uuid::new_v4())).await;
        expect(unknown.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.synchronised(&scene.token, "projects").await[0]["deleted_at"].is_null()) to be_true
    }

    // requirement: a project with a running agent cannot be removed
    story a_project_with_a_running_agent_cannot_be_removed {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "be slow").await;

        let response = scene.delete(&format!("/api/projects/{}", project.id)).await;
        expect(response.status_code()) to equal(StatusCode::CONFLICT)
        expect(text_field(&response, "error")) to equal("An agent is still running in this project.".to_string())

        scene.stop(&thread).await;
        scene.wait_until_idle(&thread).await;
    }

    // requirement: a user can start a thread in one of their projects, choosing Claude or Grok
    story a_thread_is_started {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;

        let response = scene.post(&format!("/api/projects/{}/threads", project.id), json!({ "agent": "grok", "title": " Tidy up " })).await;
        expect(response.status_code()) to equal(StatusCode::CREATED)

        let thread: thread::Model = response.json();
        expect(thread.agent.as_str()) to equal("grok")
        expect(thread.title.as_str()) to equal("Tidy up")
        expect(thread.status.as_str()) to equal("idle")
        expect(thread.project_id) to equal(project.id)
    }

    // requirement: an unknown agent name is refused
    story a_thread_with_an_unknown_agent_is_refused {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;

        let response = scene.post(&format!("/api/projects/{}/threads", project.id), json!({ "agent": "gemini" })).await;
        expect(response.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&response, "error")) to equal("Choose Claude or Grok as the agent.".to_string())
    }

    // requirement: a thread in someone else's project is refused
    story a_thread_in_someone_elses_project_is_refused {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let stranger = scene.someone_else().await;

        let response = scene.post_as(&stranger, &format!("/api/projects/{}/threads", project.id), json!({ "agent": "claude" })).await;
        expect(response.status_code()) to equal(StatusCode::NOT_FOUND)
        let unknown = scene.post(&format!("/api/projects/{}/threads", Uuid::new_v4()), json!({ "agent": "claude" })).await;
        expect(unknown.status_code()) to equal(StatusCode::NOT_FOUND)
    }

    // requirement: sending a message to an idle thread stores the message and starts the agent
    // requirement: the agent runs with the project directory as its working directory
    // requirement: when the program ends, the thread is marked as idle
    story a_message_starts_the_agent {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        let response = scene.say(&thread, "  hello there\nsecond line ").await;
        expect(response.status_code()) to equal(StatusCode::ACCEPTED)
        expect(text_field(&response, "kind")) to equal("user_message".to_string())
        expect(scene.thread_now(&thread).await.status) to equal("running".to_string())

        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&["user_message", "agent_message", "turn_summary"]))
        expect(scene.text_at(&thread, 1).await) to equal("hello there\nsecond line".to_string())
        expect(scene.text_at(&thread, 2).await) to equal("ready".to_string())
        expect(scene.entries(thread.id).await[2].body["duration_ms"].is_u64()) to be_true
        expect(scene.thread_now(&thread).await.title) to equal("hello there".to_string())

        let runs = invocations(&folder);
        expect(runs.len()) to equal(1)
        expect(runs[0].contains(&format!("--session-id {}", thread.conversation_id))) to be_true
    }

    // requirement: later messages resume that conversation
    story a_second_message_resumes_the_conversation {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "hello").await;
        scene.wait_until_idle(&thread).await;

        let response = scene.say(&thread, "and again").await;
        expect(response.status_code()) to equal(StatusCode::ACCEPTED)
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs.len()) to equal(2)
        expect(runs[1].contains(&format!("--resume {}", thread.conversation_id))) to be_true
        expect(scene.kinds(&thread).await.len()) to equal(6)
        expect(scene.thread_now(&thread).await.title) to equal("hello".to_string())
    }

    story a_title_that_was_given_is_kept {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread: thread::Model = scene.post(&format!("/api/projects/{}/threads", project.id), json!({ "agent": "claude", "title": "Named" })).await.json();

        scene.say(&thread, "hello").await;
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).await.title) to equal("Named".to_string())
    }

    // requirement: each tool the agent uses becomes an activity line
    story claude_reports_the_tools_it_uses {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        scene.say(&thread, "use tools").await;
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&[
            "user_message", "activity", "activity", "activity", "activity", "activity", "agent_message", "turn_summary",
        ]))
        expect(scene.entries(thread.id).await[1].body.clone()) to equal(json!({ "tool": "Bash", "summary": "Check repository state and existing worktrees" }))
    }

    story grok_runs_and_reports_the_tools_it_uses {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "grok").await;

        scene.say(&thread, "use tools").await;
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&[
            "user_message", "agent_message", "activity", "activity", "notice", "activity", "agent_message", "turn_summary",
        ]))
        expect(invocations(&folder)[0].contains(&format!("--session-id {}", thread.conversation_id))) to be_true
        expect(invocations(&folder)[0].contains("--single=use tools")) to be_true
    }

    // requirement: an empty message is refused
    story an_empty_message_is_refused {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        let response = scene.say(&thread, "  \n ").await;
        expect(response.status_code()) to equal(StatusCode::UNPROCESSABLE_ENTITY)
        expect(text_field(&response, "error")) to equal("Write a message first.".to_string())
        expect(scene.kinds(&thread).await.len()) to equal(0)
        expect(scene.thread_now(&thread).await.status) to equal("idle".to_string())
        expect(invocations(&folder).len()) to equal(0)
    }

    // requirement: sending a message while the agent is running is refused
    story a_message_while_the_agent_is_running_is_refused {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "be slow").await;

        let response = scene.say(&thread, "hurry up").await;
        expect(response.status_code()) to equal(StatusCode::CONFLICT)
        expect(text_field(&response, "error")) to equal("The agent is still working on the previous message.".to_string())
        expect(scene.kinds(&thread).await) to equal(kinds(&["user_message"]))

        scene.stop(&thread).await;
        scene.wait_until_idle(&thread).await;
    }

    story someone_else_cannot_use_a_thread {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        let stranger = scene.someone_else().await;

        let message = scene.post_as(&stranger, &format!("/api/threads/{}/messages", thread.id), json!({ "text": "hello" })).await;
        expect(message.status_code()) to equal(StatusCode::NOT_FOUND)
        let stop = scene.post_as(&stranger, &format!("/api/threads/{}/stop", thread.id), json!({})).await;
        expect(stop.status_code()) to equal(StatusCode::NOT_FOUND)
        let unknown = scene.post(&format!("/api/threads/{}/messages", Uuid::new_v4()), json!({ "text": "hello" })).await;
        expect(unknown.status_code()) to equal(StatusCode::NOT_FOUND)
        expect(scene.kinds(&thread).await.len()) to equal(0)
    }

    // requirement: a program that ends with a failure adds an error entry with its message
    story a_failing_agent_leaves_an_error {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        scene.say(&thread, "please fail").await;
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&["user_message", "error"]))
        expect(scene.text_at(&thread, 2).await) to equal("the fake agent was told to fail".to_string())
    }

    story an_agent_that_fails_silently_leaves_its_exit_code {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        scene.say(&thread, "end silently").await;
        scene.wait_until_idle(&thread).await;
        expect(scene.text_at(&thread, 2).await) to equal("The agent ended without finishing its turn (exit code 4).".to_string())
    }

    // requirement: a report line Teryon does not understand is skipped, and the turn continues
    story a_line_that_is_not_understood_is_skipped {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        scene.say(&thread, "say nonsense").await;
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&["user_message", "agent_message", "turn_summary"]))
    }

    // requirement: a user can stop a running agent
    // requirement: the next message after a stop resumes the same conversation
    story a_running_agent_is_stopped {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "be slow").await;
        scene.wait_until_reporting(&thread).await;

        let response = scene.stop(&thread).await;
        expect(response.status_code()) to equal(StatusCode::NO_CONTENT)
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&["user_message", "notice"]))
        expect(scene.text_at(&thread, 2).await) to equal("Stopped by you.".to_string())

        scene.say(&thread, "carry on").await;
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[1].contains(&format!("--resume {}", thread.conversation_id))) to be_true
    }

    // requirement: the program is asked to stop, and killed if it has not ended after a few seconds
    story an_agent_that_ignores_the_request_is_killed {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.say(&thread, "be stubborn").await;
        scene.wait_until_reporting(&thread).await;

        scene.stop(&thread).await;
        scene.wait_until_idle(&thread).await;
        expect(scene.text_at(&thread, 2).await) to equal("Stopped by you.".to_string())
    }

    // requirement: stopping an idle thread does nothing
    story stopping_an_idle_thread_does_nothing {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;

        let response = scene.stop(&thread).await;
        expect(response.status_code()) to equal(StatusCode::NO_CONTENT)
        expect(scene.kinds(&thread).await.len()) to equal(0)
        expect(scene.thread_now(&thread).await.status) to equal("idle".to_string())
    }

    // requirement: a program that cannot be started adds an error entry saying which program was not found
    story a_program_that_cannot_be_started_leaves_an_error {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.mark_running(&thread).await;

        Runner::default().start(scene.t.db.clone(), settings_with("/nowhere/claude"), TurnRequest {
            thread: thread.clone(),
            directory: folder.path().to_path_buf(),
            message: "hello".to_string(),
        });
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&["error"]))
        expect(scene.text_at(&thread, 1).await.starts_with("Could not start `/nowhere/claude`: No such file or directory")) to be_true
    }

    // requirement: agents still running when Teryon shuts down are stopped
    story agents_are_stopped_when_teryon_shuts_down {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let thread = scene.thread(&project, "claude").await;
        scene.mark_running(&thread).await;
        let runner = Runner::default();

        runner.start(scene.t.db.clone(), settings_with("tests/samples/fake-claude.sh"), TurnRequest {
            thread: thread.clone(),
            directory: folder.path().to_path_buf(),
            message: "be slow".to_string(),
        });
        runner.stop_all();
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread).await) to equal(kinds(&["notice"]))
        expect(scene.text_at(&thread, 1).await) to equal("Interrupted by a restart.".to_string())
    }

    // requirement: on start, every thread still marked as running is marked as idle and gets a notice
    story threads_left_running_are_released_on_start {
        let scene = scene().await;
        let folder = project_folder();
        let project = scene.project(&folder).await;
        let interrupted = scene.thread(&project, "claude").await;
        let untouched = scene.thread(&project, "grok").await;
        scene.mark_running(&interrupted).await;

        let released = release_interrupted_threads(&scene.t.db).await;
        expect(released) to equal(Ok(1))
        expect(scene.thread_now(&interrupted).await.status) to equal("idle".to_string())
        expect(scene.text_at(&interrupted, 1).await) to equal("Interrupted by a restart.".to_string())
        expect(scene.kinds(&untouched).await.len()) to equal(0)
    }
}
