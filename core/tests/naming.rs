//! Specs for naming a thread: the agent is asked for a title until the thread has one someone chose.
//!
//! These drive the workspace directly. The agent is the fake one, which notes
//! the turn Teryon started it for, so a test can name the thread as that
//! turn, the way `teryon thread title` does. The command itself is covered
//! by the service's specs.

mod common;

use lets_expect::lets_expect;

use common::{invocations, noted_turn, project_folder, repository, unknown, Scene};
use teryon_core::agents::asking::ASKING;
use teryon_core::agents::naming::NAMING;
use teryon_core::protocol::Event;

lets_expect! { #tokio_test
    // requirement: a thread's title is the start of its first message until the agent names it
    // requirement: the agent of a thread without a chosen title is asked for one, before the rest of its instructions
    story an_unnamed_thread_asks_its_agent_for_a_title {
        let scene = Scene::new();
        let folder = project_folder();
        repository(&folder);
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "hello there").unwrap();
        scene.wait_until_idle(&thread).await;

        let now = scene.thread_now(&thread);
        expect(now.title) to equal("hello there".to_string())
        expect(now.named) to be_false
        let runs = invocations(&folder);
        expect(runs[0].contains(&format!("--append-system-prompt {}If the work will change files", NAMING.replace('\n', " ")))) to be_true
    }

    // requirement: outside a repository the agent is still asked for a title
    story a_plain_folder_still_asks_for_a_title {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        let runs = invocations(&folder);
        expect(runs[0].contains(&format!("--append-system-prompt {NAMING}This thread follows the `default` workflow"))) to be_true
        expect(runs[0].contains(&format!("{} --system-prompt-snapshot", ASKING.replace('\n', " ")))) to be_true
        expect(runs[0].contains("teryon worktree create")) to be_false
    }

    // requirement: Codex and Grok are asked for a title too, in their own ways
    story every_agent_is_asked_for_a_title {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let codex = scene.thread(&project, "codex");
        let grok = scene.thread(&project, "grok");

        scene.say(&codex, "hello").unwrap();
        scene.wait_until_idle(&codex).await;
        scene.say(&grok, "hello").unwrap();
        scene.wait_until_idle(&grok).await;

        let runs = invocations(&folder);
        expect(runs[0].contains("teryon thread title")) to be_true
        expect(runs[1].contains("--rules This thread has no title yet.")) to be_true
    }

    // requirement: a thread started with a title is not named again
    story a_given_title_is_not_asked_for {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.workspace.start_thread(project.id, Some("claude"), None, "Named").unwrap();
        expect(thread.named) to be_true

        scene.say(&thread, "hello").unwrap();
        scene.wait_until_idle(&thread).await;

        expect(invocations(&folder)[0].contains("teryon thread title")) to be_false
        expect(scene.thread_now(&thread).title) to equal("Named".to_string())
    }

    // requirement: the agent names the thread during its turn; the title is announced and kept, and the agent is not asked again
    // requirement: a title is put on one line, and a long one is shortened like one taken from a message
    story the_agent_names_the_thread {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "remember the turn and wait").unwrap();
        let turn = noted_turn(&folder).await;
        let mut changes = scene.workspace.subscribe();

        expect(scene.workspace.name_thread(thread.id, turn, "  Fix the\n login   redirect ").is_ok()) to be_true
        let now = scene.thread_now(&thread);
        expect(now.title.clone()) to equal("Fix the login redirect".to_string())
        expect(now.named) to be_true
        let announced = changes.try_recv().ok().and_then(|event| match event {
            Event::ThreadChanged { thread } => Some(thread.title),
            _ => None,
        });
        expect(announced) to equal(Some("Fix the login redirect".to_string()))

        expect(scene.workspace.name_thread(thread.id, turn, &"é".repeat(61)).is_ok()) to be_true
        expect(scene.thread_now(&thread).title) to equal(format!("{}…", "é".repeat(60)))

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        let scene = scene.restart(|_| {});
        expect(scene.thread_now(&thread).named) to be_true

        scene.say(&thread, "and again").unwrap();
        scene.wait_until_idle(&thread).await;
        expect(invocations(&folder)[1].contains("teryon thread title")) to be_false
        expect(scene.thread_now(&thread).title) to equal(format!("{}…", "é".repeat(60)))
    }

    // requirement: naming a thread is refused with a message the agent can act on
    story naming_a_thread_is_refused {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "remember the turn and wait").unwrap();
        let turn = noted_turn(&folder).await;
        let name = |thread_id, turn, title: &str| scene.workspace.name_thread(thread_id, turn, title).map_err(|problem| problem.to_string());

        expect(name(unknown(), turn, "Fix it")) to equal(Err("There is no such thread.".to_string()))
        expect(name(thread.id, unknown(), "Fix it")) to equal(Err("`teryon thread title` is for agents working in Teryon, during their turn.".to_string()))
        expect(name(thread.id, turn, " \n ")) to equal(Err("Give the thread a title of a few words.".to_string()))
        expect(scene.thread_now(&thread).named) to be_false

        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(name(thread.id, turn, "Fix it")) to equal(Err("`teryon thread title` is for agents working in Teryon, during their turn.".to_string()))
        expect(scene.thread_now(&thread).title) to equal("remember the turn and wait".to_string())
    }
}
