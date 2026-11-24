//! What a thread's agent is told about the thread's stage and workflow.
//!
//! Teryon sees some stages for itself: a change started, a review ended, a
//! change applied. Others only the agent knows, such as when talking turned
//! into planning, or when a plan was approved. So the agent is told where the
//! thread stands and asked to report a new stage through `teryon thread
//! stage`, the same way it names the thread. Reporting matters: the next
//! stage may be worked on by another agent, started once this turn ends.
//!
//! At the start of a thread the agent is also offered the workflows, so a
//! small change can skip planning and review.

use std::path::Path;

use crate::workflow::{Stage, Workflow, Workflows};

/// What the thread's own agent is told about its stage, every turn.
///
/// `choosing` is whether the agent may still choose the workflow: only at
/// the start of a thread, before any change.
pub fn staging(workflows: &Workflows, workflow: &Workflow, stage: Stage, choosing: bool) -> String {
    let stages = workflow
        .stages
        .iter()
        .map(|stage| format!("`{}`", stage.name()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        concat!(
            "This thread follows the `{workflow}` workflow, whose stages are {stages}. It is in `{stage}` now. ",
            "When the work moves to another stage, run `teryon thread stage <stage>`: once your turn ends, ",
            "Teryon may hand the next stage to another agent. ",
            "When the work has already moved on and you haven't said so, report the stage now. "
        ),
        workflow = workflow.name,
        stages = stages,
        stage = stage.name(),
    );
    let stage_advice = advice(workflow, stage);
    if !stage_advice.is_empty() {
        text.push_str(&stage_advice);
        text.push(' ');
    }
    if choosing && workflows.workflows.len() > 1 {
        let offered = workflows
            .workflows
            .iter()
            .map(|workflow| match workflow.description.is_empty() {
                true => format!("`{}`", workflow.name),
                false => format!(
                    "`{}`: {}",
                    workflow.name,
                    workflow.description.trim_end_matches('.')
                ),
            })
            .collect::<Vec<_>>()
            .join("; ");
        text.push_str(&format!(
            "Before anything else, if another workflow fits the work better, switch with `teryon thread workflow <name>`. The workflows are {offered}. "
        ));
    }
    text.trim_end().to_string()
}

/// What an agent of the thread is told about the thread's plan, see [`crate::plans`].
///
/// While planning, where to write the plan, and not to start a worktree
/// before the person approved it. Elsewhere, where the plan is, once there
/// is one. Nothing when there is nothing to say.
pub fn planning(path: &Path, workflow: &Workflow, stage: Stage, written: bool) -> Option<String> {
    let path = path.display();
    if stage == Stage::Planning && workflow.has(Stage::Planning) {
        return Some(format!(
            "Write the plan for this thread as markdown in `{path}`, not in the project and not in a worktree. Don't create a worktree before the person approves the plan."
        ));
    }
    written.then(|| {
        format!("The plan for this thread is in `{path}`. Keep it current when the plan changes.")
    })
}

/// What the reviewer is told about the plan the change was built from, once there is one.
pub fn reviewing_plan(path: &Path) -> String {
    format!(
        "The change was built from the plan in `{}`; check it against the plan.",
        path.display()
    )
}

/// What a stage is for, what it must not do, and when to report the next stage.
pub fn advice(workflow: &Workflow, stage: Stage) -> String {
    let after_implementing = match workflow.has(Stage::Review) {
        true => "`review`",
        false => "`ready`",
    };
    match stage {
        Stage::Talking if workflow.has(Stage::Planning) => {
            "Find out what the person wants and ask about anything unclear. Don't work out how to do it, write a plan or change files here. Once the request is clear, report `planning`.".to_string()
        }
        Stage::Talking => {
            "Find out what the person wants and ask about anything unclear. Don't change files here. Once the request is clear, report `implementing`.".to_string()
        }
        Stage::Planning => {
            "Work out how, write the plan, and ask the person to approve it with `teryon thread ask`. Don't change project files before they approve. Report `implementing` only once they agree.".to_string()
        }
        Stage::Implementing if workflow.has(Stage::Planning) => format!(
            "Change files as the approved plan says. Report {after_implementing} once the change is complete and its tests pass."
        ),
        Stage::Implementing => format!(
            "Change files as agreed. Report {after_implementing} once the change is complete and its tests pass."
        ),
        Stage::Review => {
            "Another agent is reviewing the change. If you go back to work on it, report `implementing`.".to_string()
        }
        Stage::Ready => {
            "The change waits for the person to apply it. If you go back to work on it, report `implementing`.".to_string()
        }
        Stage::Done => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;
    use crate::settings::Settings;

    fn workflows() -> Workflows {
        Workflows::from_settings(&Settings::default())
    }

    fn brief(name: &str, stage: Stage, choosing: bool) -> String {
        let workflows = workflows();
        let workflow = workflows.of(name).clone();
        staging(&workflows, &workflow, stage, choosing)
    }

    fn plan_text(name: &str, stage: Stage, written: bool) -> Option<String> {
        let workflows = workflows();
        planning(Path::new("/p/plan.md"), workflows.of(name), stage, written)
    }

    lets_expect! {
        expect(plan_text(name, stage, written)) as what_the_agent_is_told_about_the_plan {
            let name = "default";
            let written = false;
            when(stage = Stage::Talking) as while_talking_without_a_plan {
                to say_nothing { equal(None) }
            }
            when(stage = Stage::Talking, written = true) as while_talking_with_a_plan {
                to say_where_it_is { equal(Some("The plan for this thread is in `/p/plan.md`. Keep it current when the plan changes.".to_string())) }
            }
            when(stage = Stage::Planning) as while_planning {
                to say_where_to_write_it { equal(Some("Write the plan for this thread as markdown in `/p/plan.md`, not in the project and not in a worktree. Don't create a worktree before the person approves the plan.".to_string())) }
            }
            when(stage = Stage::Planning, written = true) as while_planning_with_a_plan to still_say_where_to_write_it { have(clone().unwrap().starts_with("Write the plan")) be_true }
            when(stage = Stage::Implementing) as later_without_a_plan to say_nothing { equal(None) }
            when(stage = Stage::Implementing, written = true) as later_with_a_plan to say_where_it_is { equal(Some("The plan for this thread is in `/p/plan.md`. Keep it current when the plan changes.".to_string())) }
            when(name = "small", stage = Stage::Talking) as talking_without_planning to say_nothing { equal(None) }
            when(name = "small", stage = Stage::Talking, written = true) as talking_without_planning_with_a_plan to say_where_it_is { have(clone().unwrap().starts_with("The plan for this thread is in")) be_true }
        }

        expect(reviewing_plan(Path::new("/p/plan.md"))) as what_the_reviewer_is_told {
            to point_at_the_plan { equal("The change was built from the plan in `/p/plan.md`; check it against the plan.".to_string()) }
        }

        expect(brief(name, stage, choosing)) {
            let name = "default";
            let choosing = false;

            when(stage = Stage::Talking) as talking_on_the_default_workflow {
                to name_the_workflow_and_stage { have(contains("follows the `default` workflow, whose stages are `talking`, `planning`, `implementing`, `review`, `ready`, `done`. It is in `talking` now.")) be_true }
                to name_the_command { have(contains("`teryon thread stage <stage>`")) be_true }
                to set_boundaries_and_say_when_to_report_planning { have(contains("Find out what the person wants and ask about anything unclear. Don't work out how to do it, write a plan or change files here. Once the request is clear, report `planning`.")) be_true }
                to leave_out_the_workflows { have(contains("teryon thread workflow")) be_false }
            }
            when(stage = Stage::Talking, choosing = true) as talking_at_the_start {
                to offer_the_workflows { have(contains("switch with `teryon thread workflow <name>`. The workflows are `default`: Anything that needs thinking through: features, bugs, changes to design; `small`: Small, clear changes: documentation, wording, settings.")) be_true }
            }
            when(name = "small", stage = Stage::Talking) as talking_on_a_workflow_without_planning {
                to set_boundaries_and_say_when_to_report_implementing { have(contains("Find out what the person wants and ask about anything unclear. Don't change files here. Once the request is clear, report `implementing`.")) be_true }
            }
            when(stage = Stage::Planning) as planning {
                to ask_for_approval_and_leave_project_files_alone { have(contains("Work out how, write the plan, and ask the person to approve it with `teryon thread ask`. Don't change project files before they approve. Report `implementing` only once they agree.")) be_true }
            }
            when(stage = Stage::Implementing) as implementing {
                to follow_the_plan_and_say_when_to_report_review { have(contains("Change files as the approved plan says. Report `review` once the change is complete and its tests pass.")) be_true }
            }
            when(name = "small", stage = Stage::Implementing) as implementing_without_planning_or_review {
                to avoid_mentioning_a_plan_and_say_when_to_report_ready { have(contains("Change files as agreed. Report `ready` once the change is complete and its tests pass.")) be_true }
            }
            when(stage = Stage::Review) as during_a_review {
                to say_how_to_go_back { have(contains("Another agent is reviewing the change. If you go back to work on it, report `implementing`.")) be_true }
            }
            when(stage = Stage::Ready) as ready {
                to say_how_to_go_back { have(contains("The change waits for the person to apply it. If you go back to work on it, report `implementing`.")) be_true }
            }
            when(stage = Stage::Done) as done {
                to end_without_advice { have(ends_with("report the stage now.")) be_true }
            }
        }
    }
}
