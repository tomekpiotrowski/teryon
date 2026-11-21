//! Where a thread's work stands: its stage, the workflow that orders its stages, and the rules that move it.
//!
//! A person running several agents at once needs to see which thread needs a
//! plan approved, which is being reviewed and which only waits to be applied.
//! Every thread is therefore in one stage of a fixed set. The set is fixed so
//! Teryon can act on what each stage means: start a review, offer to apply.
//! Workflows, set in `config.toml`, choose which of the stages a thread goes
//! through, so a small change can skip planning and review.
//!
//! Stages move only as a result of work: what Teryon sees for itself (a change
//! started, a review ended, a change applied), see [`moved`], and what the
//! thread's agent reports, see [`reported`]. The rules here know nothing
//! about storage, so they can be tested over every input.

pub mod picking;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::agents::Voice;
use crate::settings::{ProviderChoice, Settings, Strength, WorkflowDefinition};

/// Where a thread's work stands, in the order work goes through them.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The person describes what they want.
    #[default]
    Talking,
    /// The agent works out how, and asks the person to approve a plan.
    Planning,
    /// The agent changes files in the thread's copy.
    Implementing,
    /// A second agent reviews the change.
    Review,
    /// The change waits for the person to apply it.
    Ready,
    /// The change was applied, accepted elsewhere, or discarded.
    Done,
}

impl Stage {
    /// Every stage, in the fixed order.
    pub const ALL: [Self; 6] = [
        Self::Talking,
        Self::Planning,
        Self::Implementing,
        Self::Review,
        Self::Ready,
        Self::Done,
    ];

    /// The name in `config.toml`, on a thread, and in `teryon thread stage`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Talking => "talking",
            Self::Planning => "planning",
            Self::Implementing => "implementing",
            Self::Review => "review",
            Self::Ready => "ready",
            Self::Done => "done",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.name() == name)
    }

    /// What a person reads, unless `config.toml` relabels it.
    pub fn usual_label(self) -> &'static str {
        match self {
            Self::Talking => "Talking",
            Self::Planning => "Planning",
            Self::Implementing => "Implementing",
            Self::Review => "In review",
            Self::Ready => "Ready to apply",
            Self::Done => "Done",
        }
    }

    /// Whether an agent works in this stage. Ready and Done wait for the person.
    pub fn worked(self) -> bool {
        !matches!(self, Self::Ready | Self::Done)
    }
}

/// An ordered pick of stages a thread goes through.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Workflow {
    pub name: String,
    /// One line telling the agent what the workflow suits, so it can choose.
    pub description: String,
    /// In the fixed order, always ending with Ready and Done.
    pub stages: Vec<Stage>,
}

impl Workflow {
    pub fn has(&self, stage: Stage) -> bool {
        self.stages.contains(&stage)
    }

    /// The stage a thread on this workflow starts in.
    pub fn first(&self) -> Stage {
        self.stages.first().copied().unwrap_or_default()
    }

    /// The stage itself when the workflow has it, otherwise the next one it has.
    ///
    /// A thread moved to a stage its workflow skips, as when a small change
    /// starts a worktree, lands where that workflow goes next.
    pub fn settle(&self, stage: Stage) -> Stage {
        self.stages
            .iter()
            .copied()
            .find(|candidate| *candidate >= stage)
            .unwrap_or(Stage::Done)
    }

    /// The names of the stages, for messages that list them.
    fn names(&self) -> String {
        self.stages
            .iter()
            .map(|stage| format!("`{}`", stage.name()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A stage and how a person reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StageLabel {
    pub stage: Stage,
    pub label: String,
    /// Who works in the stage, such as "Prefer Claude · strong". Nothing when the stage keeps the thread's agent.
    pub chooses: Option<String>,
}

/// Every workflow on offer, which one new threads start on, and the stages' labels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Workflows {
    pub default: String,
    /// By name.
    pub workflows: Vec<Workflow>,
    /// One per stage, in the fixed order.
    pub labels: Vec<StageLabel>,
}

impl Default for Workflows {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

impl Workflows {
    /// The workflows of `config.toml`, or Teryon's own when it names none.
    ///
    /// The settings were checked when read, see [`crate::settings`].
    pub fn from_settings(settings: &Settings) -> Self {
        let definitions = settings
            .workflows
            .clone()
            .unwrap_or_else(built_in_workflows);
        Self {
            default: settings.workflow.default.clone(),
            workflows: definitions
                .into_iter()
                .map(|(name, definition)| Workflow {
                    name,
                    description: definition.description,
                    stages: definition.stages,
                })
                .collect(),
            labels: Stage::ALL
                .into_iter()
                .map(|stage| {
                    let setting = settings.stages.of(stage);
                    StageLabel {
                        stage,
                        label: setting
                            .and_then(|setting| setting.label.clone())
                            .unwrap_or_else(|| stage.usual_label().to_string()),
                        chooses: setting.filter(|setting| setting.chooses()).map(|setting| {
                            format!(
                                "{} · {}",
                                setting.provider.unwrap_or(ProviderChoice::Any),
                                setting.strength.unwrap_or(Strength::Medium).name()
                            )
                        }),
                    }
                })
                .collect(),
        }
    }

    pub fn named(&self, name: &str) -> Option<&Workflow> {
        self.workflows.iter().find(|workflow| workflow.name == name)
    }

    /// A thread's workflow. One it names that is no longer set up, or none, means the default.
    pub fn of(&self, name: &str) -> &Workflow {
        self.named(name)
            .or_else(|| self.named(&self.default))
            .or_else(|| self.workflows.first())
            .expect("the settings always hold a workflow")
    }

    pub fn label(&self, stage: Stage) -> String {
        self.labels
            .iter()
            .find(|label| label.stage == stage)
            .map_or_else(
                || stage.usual_label().to_string(),
                |label| label.label.clone(),
            )
    }
}

/// The workflows Teryon offers when `config.toml` names none.
pub fn built_in_workflows() -> BTreeMap<String, WorkflowDefinition> {
    BTreeMap::from([
        (
            "default".to_string(),
            WorkflowDefinition {
                description:
                    "Anything that needs thinking through: features, bugs, changes to design."
                        .to_string(),
                stages: Stage::ALL.to_vec(),
            },
        ),
        (
            "small".to_string(),
            WorkflowDefinition {
                description: "Small, clear changes: documentation, wording, settings.".to_string(),
                stages: vec![
                    Stage::Talking,
                    Stage::Implementing,
                    Stage::Ready,
                    Stage::Done,
                ],
            },
        ),
    ])
}

/// Something Teryon sees happen to a thread, which may move its stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Happening {
    /// The thread's agent started a change in a copy of the project.
    ChangeStarted,
    /// The thread's plan was written for the first time.
    PlanWritten,
    /// The person asked for a review.
    ReviewAsked,
    /// A review ended, with the number of problems the reviewer counted, if it said.
    Reviewed(Option<u32>),
    /// The change was applied, accepted on GitHub, discarded, or its copy removed.
    ChangeFinished,
    /// The person wrote to the thread's agent.
    PersonWrote,
}

/// The stage a happening moves a thread to. Nothing leaves it where it is.
pub fn moved(workflow: &Workflow, stage: Stage, happening: Happening) -> Option<Stage> {
    let to = match happening {
        // With Planning in the workflow, only the agent reporting `implementing`, once the plan is approved, leaves it:
        // an agent may well make a worktree while it is still planning.
        Happening::ChangeStarted
            if stage < Stage::Implementing && !workflow.has(Stage::Planning) =>
        {
            workflow.settle(Stage::Implementing)
        }
        Happening::PlanWritten if stage == Stage::Talking && workflow.has(Stage::Planning) => {
            Stage::Planning
        }
        Happening::ReviewAsked if workflow.has(Stage::Review) => Stage::Review,
        Happening::Reviewed(Some(0)) if stage == Stage::Review => Stage::Ready,
        Happening::Reviewed(Some(_)) if stage == Stage::Review => {
            if workflow.has(Stage::Implementing) {
                Stage::Implementing
            } else {
                workflow.first()
            }
        }
        Happening::ChangeFinished => Stage::Done,
        Happening::PersonWrote if stage == Stage::Done => workflow.first(),
        _ => return None,
    };
    (to != stage).then_some(to)
}

/// Why a stage or workflow an agent reported was refused. The text is meant for the agent.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StageError {
    #[error("There is no stage called `{0}`. The stages are `talking`, `planning`, `implementing`, `review`, `ready` and `done`.")]
    UnknownStage(String),
    #[error("The reviewer cannot move the thread; the count of findings at the end of a review does that.")]
    Reviewer,
    #[error("A thread reaches `done` only when its change is applied or discarded.")]
    Done,
    #[error("The `{workflow}` workflow skips `{stage}`. Its stages are {stages}.")]
    Skipped {
        workflow: String,
        stage: &'static str,
        stages: String,
    },
    #[error("A plan has to be approved and implemented before the thread moves to `{0}`. Report `implementing` once the person approved the plan.")]
    LeavingPlanning(&'static str),
    #[error("There is no workflow called `{name}`. The workflows are {known}.")]
    UnknownWorkflow { name: String, known: String },
    #[error("The workflow can only be chosen at the start of a thread, before any change; this thread is already under way.")]
    UnderWay,
}

/// The stage the thread's agent may move it to, after `teryon thread stage`.
///
/// Planning only leads to implementing, or back to talking: an agent that
/// skipped implementing would send an unbuilt plan for review.
pub fn reported(
    workflow: &Workflow,
    current: Stage,
    to: &str,
    voice: Voice,
) -> Result<Stage, StageError> {
    let to = Stage::parse(to).ok_or_else(|| StageError::UnknownStage(to.to_string()))?;
    if voice == Voice::Reviewer {
        return Err(StageError::Reviewer);
    }
    if to == Stage::Done {
        return Err(StageError::Done);
    }
    if !workflow.has(to) {
        return Err(StageError::Skipped {
            workflow: workflow.name.clone(),
            stage: to.name(),
            stages: workflow.names(),
        });
    }
    if current == Stage::Planning && to > Stage::Implementing {
        return Err(StageError::LeavingPlanning(to.name()));
    }
    Ok(to)
}

/// Who chose a thread's workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chooser {
    /// The thread's agent, through `teryon thread workflow`.
    Agent,
    Person,
}

/// The workflow a thread moves to, and the stage it lands in there.
///
/// The agent may only choose at the start, before any change, so a thread
/// doesn't jump workflows halfway. The person may choose any time. A thread
/// in a stage the new workflow skips moves on to the next one it has.
pub fn picked<'a>(
    workflows: &'a Workflows,
    current: &Workflow,
    stage: Stage,
    changed: bool,
    name: &str,
    chooser: Chooser,
) -> Result<(&'a Workflow, Stage), StageError> {
    let workflow = workflows
        .named(name)
        .ok_or_else(|| StageError::UnknownWorkflow {
            name: name.to_string(),
            known: workflows
                .workflows
                .iter()
                .map(|workflow| format!("`{}`", workflow.name))
                .collect::<Vec<_>>()
                .join(", "),
        })?;
    if chooser == Chooser::Agent && (stage != current.first() || changed) {
        return Err(StageError::UnderWay);
    }
    Ok((workflow, workflow.settle(stage)))
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;
    use crate::settings::StageSetting;

    fn workflows() -> Workflows {
        Workflows::from_settings(&Settings::default())
    }

    fn full() -> Workflow {
        workflows().of("default").clone()
    }

    fn small() -> Workflow {
        workflows().of("small").clone()
    }

    /// A workflow without review or implementing, which settings allow.
    fn talk_only() -> Workflow {
        Workflow {
            name: "talk".to_string(),
            description: String::new(),
            stages: vec![Stage::Talking, Stage::Review, Stage::Ready, Stage::Done],
        }
    }

    fn skipped_planning() -> StageError {
        StageError::Skipped {
            workflow: "small".to_string(),
            stage: "planning",
            stages: "`talking`, `implementing`, `ready`, `done`".to_string(),
        }
    }

    fn unknown_huge() -> StageError {
        StageError::UnknownWorkflow {
            name: "huge".to_string(),
            known: "`default`, `small`".to_string(),
        }
    }

    fn relabelled() -> Workflows {
        let mut settings = Settings::default();
        settings.stages.ready = Some(StageSetting {
            label: Some("Ready to merge".to_string()),
            ..StageSetting::default()
        });
        Workflows::from_settings(&settings)
    }

    lets_expect! {
        expect(Stage::parse(name)) as reading_a_stage {
            when(name = "review") as a_known_name to give_the_stage { equal(Some(Stage::Review)) }
            when(name = "Review") as a_capitalised_name to be_unknown { equal(None) }
            when(name = "") as an_empty_name to be_unknown { equal(None) }
        }

        expect(Stage::ALL.map(Stage::worked)) as stages_an_agent_works_in {
            to leave_ready_and_done_to_the_person { equal([true, true, true, true, false, false]) }
        }

        expect(workflow.settle(stage)) as settling_in_a_workflow {
            let workflow = small();
            when(stage = Stage::Implementing) as a_stage_the_workflow_has to stay { equal(Stage::Implementing) }
            when(stage = Stage::Planning) as a_skipped_stage to move_to_the_next { equal(Stage::Implementing) }
            when(stage = Stage::Review) as a_skipped_review to move_to_ready { equal(Stage::Ready) }
        }

        expect(workflows().of(&name).name.clone()) as the_workflow_of_a_thread {
            when(name = "small".to_string()) as a_known_one to be_it { equal("small".to_string()) }
            when(name = String::new()) as none to be_the_default { equal("default".to_string()) }
            when(name = "gone".to_string()) as one_no_longer_set_up to be_the_default { equal("default".to_string()) }
        }

        expect(workflows.label(Stage::Ready)) as the_label_of_ready {
            when(workflows = workflows()) as the_usual_labels to say_apply { equal("Ready to apply".to_string()) }
            when(workflows = relabelled()) as a_relabelled_stage to use_the_new_label { equal("Ready to merge".to_string()) }
        }

        expect(moved(&workflow, stage, happening)) as what_teryon_moves {
            let workflow = full();
            when(stage = Stage::Talking, happening = Happening::ChangeStarted) as a_change_started_while_talking to stay_for_approval { equal(None) }
            when(stage = Stage::Planning, happening = Happening::ChangeStarted) as a_change_started_while_planning to stay_for_approval { equal(None) }
            when(workflow = small(), stage = Stage::Talking, happening = Happening::ChangeStarted) as a_change_started_while_talking_without_planning to move_to_implementing { equal(Some(Stage::Implementing)) }
            when(stage = Stage::Talking, happening = Happening::PlanWritten) as a_plan_written_while_talking to move_to_planning { equal(Some(Stage::Planning)) }
            when(stage = Stage::Planning, happening = Happening::PlanWritten) as a_plan_written_while_planning to stay { equal(None) }
            when(stage = Stage::Implementing, happening = Happening::PlanWritten) as a_plan_written_while_implementing to stay { equal(None) }
            when(workflow = small(), stage = Stage::Talking, happening = Happening::PlanWritten) as a_plan_written_without_planning to stay { equal(None) }
            when(stage = Stage::Implementing, happening = Happening::ChangeStarted) as a_change_started_while_implementing to stay { equal(None) }
            when(stage = Stage::Ready, happening = Happening::ChangeStarted) as a_change_started_after_review to stay { equal(None) }
            when(workflow = talk_only(), stage = Stage::Talking, happening = Happening::ChangeStarted) as a_change_on_a_workflow_without_implementing to move_to_its_next_stage { equal(Some(Stage::Review)) }
            when(stage = Stage::Implementing, happening = Happening::ReviewAsked) as a_review_asked to move_to_review { equal(Some(Stage::Review)) }
            when(stage = Stage::Review, happening = Happening::ReviewAsked) as a_review_asked_again to stay { equal(None) }
            when(workflow = small(), stage = Stage::Implementing, happening = Happening::ReviewAsked) as a_review_on_a_workflow_without_one to stay { equal(None) }
            when(stage = Stage::Review, happening = Happening::Reviewed(Some(0))) as a_clean_review to move_to_ready { equal(Some(Stage::Ready)) }
            when(stage = Stage::Review, happening = Happening::Reviewed(Some(3))) as a_review_with_findings to move_back_to_implementing { equal(Some(Stage::Implementing)) }
            when(workflow = talk_only(), stage = Stage::Review, happening = Happening::Reviewed(Some(3))) as findings_on_a_workflow_without_implementing to move_to_the_first_stage { equal(Some(Stage::Talking)) }
            when(stage = Stage::Review, happening = Happening::Reviewed(None)) as a_review_without_a_count to stay { equal(None) }
            when(stage = Stage::Ready, happening = Happening::Reviewed(Some(2))) as findings_outside_review to stay { equal(None) }
            when(stage = Stage::Ready, happening = Happening::ChangeFinished) as a_finished_change to move_to_done { equal(Some(Stage::Done)) }
            when(stage = Stage::Done, happening = Happening::ChangeFinished) as a_finished_change_in_done to stay { equal(None) }
            when(stage = Stage::Done, happening = Happening::PersonWrote) as a_message_after_done to start_over { equal(Some(Stage::Talking)) }
            when(stage = Stage::Implementing, happening = Happening::PersonWrote) as a_message_while_working to stay { equal(None) }
        }

        expect(reported(&workflow, current, target, voice)) as a_reported_stage {
            let workflow = full();
            let voice = Voice::Author;
            let current = Stage::Talking;
            when(target = "planning") as talking_to_planning to be_allowed { equal(Ok(Stage::Planning)) }
            when(target = "implementing") as talking_to_implementing to be_allowed { equal(Ok(Stage::Implementing)) }
            when(target = "talking") as the_same_stage to be_allowed { equal(Ok(Stage::Talking)) }
            when(target = "dancing") as an_unknown_stage to be_refused { equal(Err(StageError::UnknownStage("dancing".to_string()))) }
            when(target = "done") as done to be_refused { equal(Err(StageError::Done)) }
            when(target = "review", voice = Voice::Reviewer) as the_reviewer to be_refused { equal(Err(StageError::Reviewer)) }
            when(workflow = small(), target = "planning") as a_stage_the_workflow_skips to be_refused {
                equal(Err(skipped_planning()))
            }
            when(current = Stage::Planning, target = "implementing") as planning_to_implementing to be_allowed { equal(Ok(Stage::Implementing)) }
            when(current = Stage::Planning, target = "talking") as planning_back_to_talking to be_allowed { equal(Ok(Stage::Talking)) }
            when(current = Stage::Planning, target = "review") as planning_to_review to be_refused { equal(Err(StageError::LeavingPlanning("review"))) }
            when(current = Stage::Planning, target = "ready") as planning_to_ready to be_refused { equal(Err(StageError::LeavingPlanning("ready"))) }
            when(current = Stage::Implementing, target = "review") as implementing_to_review to be_allowed { equal(Ok(Stage::Review)) }
            when(current = Stage::Review, target = "implementing") as back_to_work_after_review to be_allowed { equal(Ok(Stage::Implementing)) }
            when(workflow = small(), current = Stage::Implementing, target = "ready") as straight_to_ready_without_review to be_allowed { equal(Ok(Stage::Ready)) }
        }

        expect(picked(&all, &current, stage, changed, name, chooser).map(|(chosen, landed)| (chosen.name.clone(), landed))) as choosing_a_workflow {
            let all = workflows();
            let current = full();
            let stage = Stage::Talking;
            let changed = false;
            let chooser = Chooser::Agent;
            when(name = "small") as the_agent_at_the_start to be_allowed { equal(Ok(("small".to_string(), Stage::Talking))) }
            when(name = "huge") as an_unknown_workflow to be_refused {
                equal(Err(unknown_huge()))
            }
            when(name = "small", stage = Stage::Implementing) as the_agent_later to be_refused { equal(Err(StageError::UnderWay)) }
            when(name = "small", changed = true) as the_agent_after_a_change to be_refused { equal(Err(StageError::UnderWay)) }
            when(name = "small", stage = Stage::Planning, chooser = Chooser::Person) as the_person_in_a_skipped_stage to move_on { equal(Ok(("small".to_string(), Stage::Implementing))) }
            when(name = "small", stage = Stage::Review, changed = true, chooser = Chooser::Person) as the_person_during_review to move_on_to_ready { equal(Ok(("small".to_string(), Stage::Ready))) }
        }
    }
}
