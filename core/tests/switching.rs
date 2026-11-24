//! Specs for handing a thread to another agent partway through.
//!
//! When an agent's subscription runs out, switching is the only way to go
//! on with a thread, so these check that the new agent's program is the one
//! started, that it starts a conversation of its own, and that it is told
//! what was said before.

mod common;

use std::time::Duration;

use lets_expect::lets_expect;
use serde_json::json;
use tokio::time::sleep;

use common::{invocations, project_folder, unknown, Scene};
use teryon_core::agents::Voice;

lets_expect! { #tokio_test
    // requirement: a thread can move to another agent, which starts its own conversation, told what was said so far
    // requirement: the thread says who took over, and forgets the old agent's conversation and slash commands
    story the_author_moves_to_another_agent {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "codex", Some("gpt-6-astra")).unwrap();
        expect(changed.agent.clone()) to equal("codex".to_string())
        expect(changed.model.clone()) to equal(Some("gpt-6-astra".to_string()))
        expect(changed.conversation.clone()) to equal(None)
        expect(changed.commands.clone()) to equal(Vec::<String>::new())
        expect(changed.handover) to be_true
        let notice = scene.entries(&thread).last().cloned().unwrap();
        expect(notice.kind.clone()) to equal("notice".to_string())
        expect(notice.body["text"].clone()) to equal(json!("Codex takes over from Claude."))

        scene.say(&thread, "go on").unwrap();
        scene.wait_until_idle(&thread).await;
        let runs = invocations(&folder);
        expect(runs[1].starts_with("exec --json")) to be_true
        expect(runs[1].contains("-m gpt-6-astra")) to be_true
        expect(runs[1].contains("You are taking over this thread")) to be_true
        expect(runs[1].contains("Person: hello")) to be_true
        expect(runs[1].contains("The person's new message:  go on")) to be_true

        let now = scene.thread_now(&thread);
        expect(now.handover) to be_false
        expect(now.conversation.is_some()) to be_true

        scene.say(&thread, "and again").unwrap();
        scene.wait_until_idle(&thread).await;
        let runs = invocations(&folder);
        expect(runs[2].starts_with("exec resume")) to be_true
        expect(runs[2].contains("taking over")) to be_false
    }

    // requirement: a thread with nothing said yet moves to another agent without a handover
    story a_thread_not_yet_started_moves_without_a_handover {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.workspace.change_agent(thread.id, Voice::Author, "grok", None).unwrap();
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs.len()) to equal(1)
        expect(runs[0].contains("taking over")) to be_false
        expect(scene.thread_now(&thread).agent) to equal("grok".to_string())
    }

    // requirement: choosing the agent the thread already has changes only the model, and says nothing in the thread
    story choosing_the_same_agent_keeps_the_conversation {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;
        let before = scene.entries(&thread).len();
        let conversation = scene.thread_now(&thread).conversation;

        let changed = scene.workspace.change_agent(thread.id, Voice::Author, "claude", Some("opus")).unwrap();
        expect(changed.conversation.clone()) to equal(conversation)
        expect(changed.handover) to be_false
        expect(scene.entries(&thread).len()) to equal(before)
    }

    // requirement: the agent cannot change while it works, since the turn under way reports its conversation
    story the_agent_does_not_change_while_it_works {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "be slow").unwrap();
        for _ in 0..200 {
            if !invocations(&folder).is_empty() {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        let refused = scene.workspace.change_agent(thread.id, Voice::Author, "codex", None).map_err(|problem| problem.to_string());
        expect(refused.err()) to equal(Some("The agent is still working on the previous message.".to_string()))
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(scene.thread_now(&thread).agent) to equal("claude".to_string())
    }

    // requirement: an agent Teryon does not know, a missing thread or a missing reviewer is refused
    story unknown_choices_are_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        let unknown_agent = scene.workspace.change_agent(thread.id, Voice::Author, "mistral", None).map_err(|problem| problem.to_string());
        expect(unknown_agent.err()) to equal(Some("Choose Claude, Codex, Grok or Gemini.".to_string()))
        let elsewhere = scene.workspace.change_agent(unknown(), Voice::Author, "codex", None).map_err(|problem| problem.to_string());
        expect(elsewhere.err()) to equal(Some("There is no such thread.".to_string()))
        let reviewer = scene.workspace.change_agent(thread.id, Voice::Reviewer, "codex", None).map_err(|problem| problem.to_string());
        expect(reviewer.err()) to equal(Some("Nobody reviews this thread yet. Ask for a review first.".to_string()))
        expect(scene.thread_now(&thread).agent) to equal("claude".to_string())
    }
}
