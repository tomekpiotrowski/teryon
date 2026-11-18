//! Specs for projects, threads and running agents.
//!
//! These drive the workspace directly, the way the local service does for
//! every client. The service's own tests cover the connection on top.

mod common;

use std::fs::{create_dir_all, read_to_string, write};
use std::path::PathBuf;
use std::time::Duration;

use lets_expect::lets_expect;
use serde_json::json;
use tokio::time::sleep;

use common::{
    git, invocations, kinds, mark_running, project_folder, repository, settings_with, unknown,
    Scene,
};
use teryon_core::protocol::{Event, ProjectState};
use teryon_core::store::Status;

lets_expect! { #tokio_test
    // requirement: adding a project takes a name and a directory
    story a_project_is_added {
        let scene = Scene::new();
        let folder = project_folder();

        let project = scene.workspace.add_project("  Teryon ", &folder.path().to_string_lossy());
        expect(project.as_ref().ok().map(|project| project.name.clone())) to equal(Some("Teryon".to_string()))
        let project = project.unwrap();
        expect(project.directory.clone()) to equal(folder.path().canonicalize().unwrap())
        expect(project.state) to equal(ProjectState::Ready)
        expect(scene.projects().len()) to equal(1)
    }

    // requirement: projects are kept in projects.json in the person's data folder
    story projects_are_kept_across_restarts {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let scene = scene.restart(|_| {});
        expect(scene.projects().iter().map(|kept| kept.id).collect::<Vec<_>>()) to equal(vec![project.id])
        expect(scene.home.path().join("projects.json").is_file()) to be_true
    }

    // requirement: an empty name is refused
    story a_project_without_a_name_is_refused {
        let scene = Scene::new();
        let folder = project_folder();

        let added = scene.workspace.add_project("   ", &folder.path().to_string_lossy()).map_err(|problem| problem.to_string());
        expect(added.err()) to equal(Some("A project needs a name.".to_string()))
        expect(scene.projects().len()) to equal(0)
    }

    // requirement: the directory must exist and be a directory
    story a_project_in_a_missing_directory_is_refused {
        let scene = Scene::new();
        let folder = project_folder();

        let missing = scene.workspace.add_project("Teryon", &folder.path().join("nothing").to_string_lossy()).map_err(|problem| problem.to_string());
        expect(missing.err()) to equal(Some("The directory does not exist.".to_string()))
        write(folder.path().join("file.txt"), "text").unwrap();
        let file = scene.workspace.add_project("Teryon", &folder.path().join("file.txt").to_string_lossy()).map_err(|problem| problem.to_string());
        expect(file.err()) to equal(Some("That is a file, not a directory.".to_string()))
    }

    // requirement: a directory already added is refused
    story a_directory_already_added_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        scene.project(&folder);

        let again = scene.workspace.add_project("Again", &format!("{}/.", folder.path().display())).map_err(|problem| problem.to_string());
        expect(again.err()) to equal(Some("This directory is already a project.".to_string()))
    }

    // requirement: adding a directory that already has threads shows them
    // requirement: removing a project leaves its directory, its threads and its worktrees untouched
    story a_project_removed_and_added_again_keeps_its_threads {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let removed = scene.workspace.remove_project(project.id);
        expect(removed.is_ok()) to be_true
        expect(folder.path().is_dir()) to be_true
        expect(scene.projects().len()) to equal(0)
        expect(scene.threads().len()) to equal(0)

        let again = scene.project(&folder);
        let threads = scene.threads();
        expect(threads.len()) to equal(1)
        expect(threads[0].project_id) to equal(again.id)
        expect(scene.kinds(&threads[0])) to equal(kinds(&["user_message", "agent_message", "turn_summary"]))
    }

    story an_unknown_project_cannot_be_removed {
        let scene = Scene::new();
        let removed = scene.workspace.remove_project(unknown()).map_err(|problem| problem.to_string());
        expect(removed.err()) to equal(Some("There is no such project.".to_string()))
    }

    // requirement: a project with a running agent cannot be removed
    story a_project_with_a_running_agent_cannot_be_removed {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be slow").unwrap();

        let removed = scene.workspace.remove_project(project.id).map_err(|problem| problem.to_string());
        expect(removed.err()) to equal(Some("An agent is still running in this project.".to_string()))

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
    }

    // requirement: a project whose directory has gone is still listed, and marked as missing; it can be removed
    story a_project_whose_directory_has_gone_is_marked_missing {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let path = folder.path().to_path_buf();

        let scene = scene.restart(|_| { drop(folder); });
        expect(path.exists()) to be_false
        expect(scene.projects()[0].state) to equal(ProjectState::Missing)
        let thread = scene.workspace.start_thread(project.id, "claude", "").map_err(|problem| problem.to_string());
        expect(thread.err()) to equal(Some("This project's directory is gone.".to_string()))
        expect(scene.workspace.remove_project(project.id).is_ok()) to be_true
    }

    // requirement: a project another Teryon has open is shown as in use elsewhere
    story a_project_open_elsewhere_is_marked_in_use {
        let scene = Scene::new();
        let folder = project_folder();
        scene.project(&folder);

        let elsewhere = Scene::new();
        let project = elsewhere.project(&folder);
        expect(project.state) to equal(ProjectState::InUse)
    }

    // requirement: starting a thread, choosing Claude or Grok
    story a_thread_is_started {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, "grok", " Tidy up ").unwrap();
        expect(thread.agent.as_str()) to equal("grok")
        expect(thread.title.as_str()) to equal("Tidy up")
        expect(thread.status) to equal(Status::Idle)
        expect(thread.project_id) to equal(project.id)
        expect(read_to_string(folder.path().join(".teryon/.gitignore")).unwrap()) to equal("*\n".to_string())
    }

    // requirement: an unknown agent name is refused
    story a_thread_with_an_unknown_agent_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);

        let thread = scene.workspace.start_thread(project.id, "gemini", "").map_err(|problem| problem.to_string());
        expect(thread.err()) to equal(Some("Choose Claude or Grok as the agent.".to_string()))
        let elsewhere = scene.workspace.start_thread(unknown(), "claude", "").map_err(|problem| problem.to_string());
        expect(elsewhere.err()) to equal(Some("There is no such project.".to_string()))
    }

    // requirement: sending a message to an idle thread stores the message and starts the agent
    // requirement: the agent runs with the project directory as its working directory
    // requirement: when the program ends, the thread is marked as idle
    story a_message_starts_the_agent {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        let entry = scene.say(&thread, "  hello there\nsecond line ").unwrap();
        expect(entry.kind.as_str()) to equal("user_message")
        expect(scene.thread_now(&thread).status) to equal(Status::Running)

        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&["user_message", "agent_message", "turn_summary"]))
        expect(scene.text_at(&thread, 1)) to equal("hello there\nsecond line".to_string())
        expect(scene.text_at(&thread, 2)) to equal("ready".to_string())
        expect(scene.entries(&thread)[2].body["duration_ms"].is_u64()) to be_true
        expect(scene.thread_now(&thread).title) to equal("hello there".to_string())

        let runs = invocations(&folder);
        expect(runs.len()) to equal(1)
        expect(runs[0].contains(&format!("--session-id {}", thread.conversation_id))) to be_true
        expect(runs[0].contains("--append-system-prompt")) to be_false
        expect(scene.thread_now(&thread).branch) to equal(None)
    }

    // requirement: in a repository the agent is told where a worktree goes
    // requirement: a question creates no worktree
    story a_question_in_a_repository_creates_no_worktree {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        scene.say(&thread, "and again").unwrap();
        scene.wait_until_idle(&thread).await;

        expect(scene.thread_now(&thread).branch) to equal(None)
        let runs = invocations(&folder);
        expect(runs[0].contains(&format!(".teryon/worktrees/{}", thread.id))) to be_true
        expect(runs[0].contains("--append-system-prompt")) to be_true
        expect(runs[0].contains("the branch `main`")) to be_true
        expect(runs[1].contains("--append-system-prompt")) to be_true
        expect(git(folder.path(), &["status", "--porcelain", "--untracked-files=all"]).contains(".teryon")) to be_false
    }

    // requirement: grok is told the same thing, through its own option
    story grok_is_told_where_a_worktree_goes {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "grok");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs[0].contains("--rules")) to be_true
        expect(runs[0].contains(&format!(".teryon/worktrees/{}", thread.id))) to be_true
        expect(runs[0].contains("the branch `main`")) to be_true
    }

    // requirement: a worktree the agent creates at the instructed path is recorded, and its files can be read
    // requirement: once that checkout is gone, the thread records no branch
    story a_worktree_the_agent_creates_is_recorded {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "make a worktree").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).branch) to equal(Some("feature".to_string()))
        expect(git(folder.path(), &["status", "--porcelain", "--untracked-files=all"]).contains(".teryon")) to be_false

        let files = scene.workspace.files(project.id, Some("feature"), None).await.map(|listing| listing.branch);
        expect(files.ok().flatten()) to equal(Some("feature".to_string()))

        git(folder.path(), &["worktree", "remove", "--force", &format!(".teryon/worktrees/{}", thread.id)]);
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).branch) to equal(None)
    }

    // requirement: the branch is recorded while the agent is still working
    story a_worktree_is_recorded_while_the_agent_is_working {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "make a worktree and wait").unwrap();
        let mut running = false;
        for _ in 0..100 {
            let current = scene.thread_now(&thread);
            if current.status == Status::Running && current.branch.as_deref() == Some("feature") {
                running = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        expect(running) to be_true

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).branch) to equal(Some("feature".to_string()))
    }

    // requirement: later messages resume that conversation
    story a_second_message_resumes_the_conversation {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        expect(scene.say(&thread, "and again").is_ok()) to be_true
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs.len()) to equal(2)
        expect(runs[1].contains(&format!("--resume {}", thread.conversation_id))) to be_true
        expect(scene.kinds(&thread).len()) to equal(6)
        expect(scene.thread_now(&thread).title) to equal("hello".to_string())
    }

    // requirement: a conversation is kept across restarts and resumed
    story a_conversation_continues_after_a_restart {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let scene = scene.restart(|_| {});
        expect(scene.kinds(&thread).len()) to equal(3)
        scene.say(&thread, "and again").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[1].contains(&format!("--resume {}", thread.conversation_id))) to be_true
        expect(scene.entries(&thread).iter().map(|entry| entry.position).collect::<Vec<_>>()) to equal(vec![1, 2, 3, 4, 5, 6])
    }

    story a_title_that_was_given_is_kept {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread(project.id, "claude", "Named").unwrap();

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).title) to equal("Named".to_string())
    }

    // requirement: each tool the agent uses becomes an activity line
    story claude_reports_the_tools_it_uses {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "use tools").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&[
            "user_message", "activity", "activity", "activity", "activity", "activity", "agent_message", "turn_summary",
        ]))
        expect(scene.entries(&thread)[1].body.clone()) to equal(json!({ "tool": "Bash", "summary": "Check repository state and existing worktrees" }))
    }

    story grok_runs_and_reports_the_tools_it_uses {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "grok");

        scene.say(&thread, "use tools").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&[
            "user_message", "agent_message", "activity", "activity", "notice", "activity", "agent_message", "turn_summary",
        ]))
        expect(invocations(&folder)[0].contains(&format!("--session-id {}", thread.conversation_id))) to be_true
        expect(invocations(&folder)[0].contains("--single=use tools")) to be_true
    }

    // requirement: an empty message is refused
    story an_empty_message_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        expect(scene.say(&thread, "  \n ").err()) to equal(Some("Write a message first.".to_string()))
        expect(scene.kinds(&thread).len()) to equal(0)
        expect(scene.thread_now(&thread).status) to equal(Status::Idle)
        expect(invocations(&folder).len()) to equal(0)
    }

    // requirement: sending a message while the agent is running is refused
    story a_message_while_the_agent_is_running_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be slow").unwrap();

        expect(scene.say(&thread, "hurry up").err()) to equal(Some("The agent is still working on the previous message.".to_string()))
        expect(scene.kinds(&thread)) to equal(kinds(&["user_message"]))

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
    }

    story an_unknown_thread_is_refused {
        let scene = Scene::new();

        expect(scene.workspace.send_message(unknown(), "hello").map_err(|problem| problem.to_string()).err()) to equal(Some("There is no such thread.".to_string()))
        expect(scene.workspace.stop(unknown()).is_err()) to be_true
        expect(scene.workspace.mark_seen(unknown()).is_err()) to be_true
        expect(scene.workspace.entries(unknown()).is_err()) to be_true
    }

    // requirement: a program that ends with a failure adds an error entry with its message
    story a_failing_agent_leaves_an_error {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "please fail").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&["user_message", "error"]))
        expect(scene.text_at(&thread, 2)) to equal("the fake agent was told to fail".to_string())
    }

    story an_agent_that_fails_silently_leaves_its_exit_code {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "end silently").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.text_at(&thread, 2)) to equal("The agent ended without finishing its turn (exit code 4).".to_string())
    }

    // requirement: a report line Teryon does not understand is skipped, and the turn continues
    story a_line_that_is_not_understood_is_skipped {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "say nonsense").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&["user_message", "agent_message", "turn_summary"]))
    }

    // requirement: a user can stop a running agent
    // requirement: the next message after a stop resumes the same conversation
    story a_running_agent_is_stopped {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be slow").unwrap();
        scene.wait_until_reporting(&thread).await;

        expect(scene.workspace.stop(thread.id).is_ok()) to be_true
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&["user_message", "notice"]))
        expect(scene.text_at(&thread, 2)) to equal("Stopped by you.".to_string())

        scene.say(&thread, "carry on").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[1].contains(&format!("--resume {}", thread.conversation_id))) to be_true
    }

    // requirement: the program is asked to stop, and killed if it has not ended after a few seconds
    story an_agent_that_ignores_the_request_is_killed {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be stubborn").unwrap();
        scene.wait_until_reporting(&thread).await;

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.text_at(&thread, 2)) to equal("Stopped by you.".to_string())
    }

    // requirement: stopping an idle thread does nothing
    story stopping_an_idle_thread_does_nothing {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        expect(scene.workspace.stop(thread.id).is_ok()) to be_true
        expect(scene.kinds(&thread).len()) to equal(0)
        expect(scene.thread_now(&thread).status) to equal(Status::Idle)
    }

    // requirement: a program that cannot be started adds an error entry saying which program was not found
    story a_program_that_cannot_be_started_leaves_an_error {
        let scene = Scene::with(settings_with("/nowhere/claude"));
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.kinds(&thread)) to equal(kinds(&["user_message", "error"]))
        expect(scene.text_at(&thread, 2).starts_with("Could not start `/nowhere/claude`:")) to be_true
    }

    // requirement: agents still running when Teryon shuts down are stopped
    story agents_are_stopped_when_teryon_shuts_down {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be slow").unwrap();
        scene.wait_until_reporting(&thread).await;

        scene.workspace.shut_down(Duration::from_secs(5)).await;
        expect(scene.thread_now(&thread).status) to equal(Status::Idle)
        expect(scene.text_at(&thread, 2)) to equal("Interrupted by a restart.".to_string())
    }

    // requirement: when the service starts, every thread left marked as running is marked idle and unread, with a notice
    story threads_left_running_are_released_on_start {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let interrupted = scene.thread(&project, "claude");
        let untouched = scene.thread(&project, "grok");

        let scene = scene.restart(|_| mark_running(&folder, &interrupted));
        expect(scene.thread_now(&interrupted).status) to equal(Status::Idle)
        expect(scene.thread_now(&interrupted).unread) to be_true
        expect(scene.thread_now(&untouched).unread) to be_false
        expect(scene.text_at(&interrupted, 1)) to equal("Interrupted by a restart.".to_string())
        expect(scene.kinds(&untouched).len()) to equal(0)
    }

    // requirement: a thread is unread once its agent stops, however it stops, until the person has looked at it
    story a_thread_is_unread_once_its_agent_ends {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        expect(scene.thread_now(&thread).unread) to be_false

        scene.say(&thread, "hello").unwrap();
        expect(scene.thread_now(&thread).unread) to be_false
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).unread) to be_true

        expect(scene.workspace.mark_seen(thread.id).is_ok()) to be_true
        expect(scene.thread_now(&thread).unread) to be_false
    }

    story a_thread_whose_agent_failed_is_unread {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "please fail").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).unread) to be_true
    }

    // requirement: looking at a thread while its agent works does not hide what the agent does afterwards
    story a_thread_seen_while_its_agent_works_is_unread_when_it_ends {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be slow").unwrap();
        scene.wait_until_reporting(&thread).await;

        expect(scene.workspace.mark_seen(thread.id).is_ok()) to be_true
        expect(scene.thread_now(&thread).status) to equal(Status::Running)
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).unread) to be_true
    }

    // requirement: looking at a thread that is not unread changes nothing, so no client is told of a change
    story seeing_a_thread_that_is_not_unread_changes_nothing {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        let mut changes = scene.workspace.subscribe();

        expect(scene.workspace.mark_seen(thread.id).is_ok()) to be_true
        expect(changes.try_recv().is_err()) to be_true
    }

    // requirement: the slash commands an agent program names are kept with the thread
    story a_thread_keeps_the_commands_its_agent_takes {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let claude = scene.thread(&project, "claude");
        let grok = scene.thread(&project, "grok");
        expect(scene.thread_now(&claude).commands.len()) to equal(0)

        scene.say(&claude, "hello").unwrap();
        scene.say(&grok, "hello").unwrap();
        scene.wait_until_idle(&claude).await;
        scene.wait_until_idle(&grok).await;
        expect(scene.thread_now(&claude).commands) to equal(kinds(&["compact", "review"]))
        expect(scene.thread_now(&grok).commands) to equal(kinds(&["compact", "review"]))
    }

    // requirement: every change is announced to whoever is listening
    story changes_are_announced {
        let scene = Scene::new();
        let folder = project_folder();
        let mut changes = scene.workspace.subscribe();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let mut names = Vec::new();
        while let Ok(event) = changes.try_recv() {
            names.push(match event {
                Event::Snapshot { .. } => "snapshot",
                Event::ProjectAdded { .. } => "project_added",
                Event::ProjectRemoved { .. } => "project_removed",
                Event::ThreadChanged { .. } => "thread_changed",
                Event::EntryAdded { .. } => "entry_added",
            });
        }
        expect(names.first().copied()) to equal(Some("project_added"))
        expect(names.iter().filter(|name| **name == "entry_added").count()) to equal(3)
        expect(names.last().copied()) to equal(Some("thread_changed"))
    }

    // requirement: a thread folder whose thread.json cannot be read is skipped; the other threads are shown
    story a_damaged_thread_does_not_hide_the_others {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        scene.thread(&project, "claude");

        let scene = scene.restart(|_| {
            let damaged: PathBuf = folder.path().join(".teryon/threads/damaged");
            create_dir_all(&damaged).unwrap();
            write(damaged.join("thread.json"), "{").unwrap();
        });
        expect(scene.threads().len()) to equal(1)
    }
}
