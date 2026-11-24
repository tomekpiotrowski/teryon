//! Specs for how the last run in a thread ended, which Home reads to tell failed threads from finished ones.

mod common;

use lets_expect::lets_expect;

use common::{project_folder, Scene};
use teryon_core::store::{RunEnd, Thread};

/// What a thread's last run ended as, once the agent has stopped.
async fn ended_after(message: &str, stop_it: bool) -> Option<RunEnd> {
    ended_after_by("claude", message, stop_it).await
}

async fn ended_after_by(agent: &str, message: &str, stop_it: bool) -> Option<RunEnd> {
    let scene = Scene::new();
    let folder = project_folder();
    let project = scene.project(&folder);
    let thread = scene.thread(&project, agent);
    scene.say(&thread, message).unwrap();
    if stop_it {
        scene.workspace.stop(thread.id).unwrap();
    }
    scene.wait_until_idle(&thread).await;
    scene.thread_now(&thread).ended
}

lets_expect! { #tokio_test
    // requirement: a thread remembers how its last run ended
    story a_run_that_finishes_is_recorded_as_finished {
        let ended = ended_after("hello", false).await;
        expect(ended) to equal(Some(RunEnd::Finished))
    }

    story a_run_that_ends_without_finishing_is_recorded_as_failed {
        let ended = ended_after("please fail", false).await;
        expect(ended) to equal(Some(RunEnd::Failed))
    }

    // requirement: a failure the program reports itself is a failed run, not a finished one
    story a_failure_claude_reports_is_recorded_as_failed {
        let ended = ended_after("end early", false).await;
        expect(ended) to equal(Some(RunEnd::Failed))
    }

    story a_failure_gemini_reports_is_recorded_as_failed {
        let ended = ended_after_by("gemini", "end early", false).await;
        expect(ended) to equal(Some(RunEnd::Failed))
    }

    story a_run_that_hits_a_usage_limit_is_recorded_as_limited {
        let ended = ended_after("hit quota", false).await;
        expect(ended) to equal(Some(RunEnd::Limited))
    }

    story a_run_the_person_stops_is_recorded_as_stopped {
        let ended = ended_after("be slow", true).await;
        expect(ended) to equal(Some(RunEnd::Stopped))
    }

    story a_thread_that_never_ran_has_no_end {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread: Thread = scene.thread(&project, "claude");
        expect(scene.thread_now(&thread).ended) to equal(None)
    }

    story a_new_run_clears_the_end_of_the_last_one {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        scene.say(&thread, "please fail").unwrap();
        scene.wait_until_idle(&thread).await;
        scene.say(&thread, "be slow").unwrap();
        let during = scene.thread_now(&thread).ended;
        scene.workspace.stop(thread.id).unwrap();
        scene.wait_until_idle(&thread).await;
        expect(during) to equal(None)
    }

    story a_thread_stored_before_ends_were_recorded_still_loads {
        let scene = Scene::new();
        let folder = project_folder();
        let project = scene.project(&folder);
        let thread = scene.thread(&project, "claude");
        let mut stored = serde_json::to_value(&thread).unwrap();
        stored.as_object_mut().unwrap().remove("ended");
        let loaded: Result<Thread, _> = serde_json::from_value(stored);
        expect(loaded.map(|thread| thread.ended).ok()) to equal(Some(None))
    }
}
