//! Moving threads between stages, and starting the next stage's agent by itself.
//!
//! The rules live in [`crate::workflow`]; this applies them to stored threads,
//! writes the line that says why a thread moved, and decides, when a turn
//! ends, whether another agent should take over. Kept apart from the rest of
//! the workspace because the end of a turn is where a mistake costs most: a
//! loop between an author and a reviewer uses up a subscription while nobody
//! watches, so every automatic start passes the checks in [`Workspace::carry_on`].

use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::MutexGuard;

use chrono::Utc;
use serde_json::json;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{Inner, Workspace, WorkspaceError};
use crate::agents::models::{claude_alias_matches, split_gemini_model, Effort, ModelOption};
use crate::agents::{Agent, EntryDraft, EntryKind, Voice};
use crate::changes::reviewing::findings_in;
use crate::plans::{plan_line, plan_path, read_plan, LARGEST_PLAN};
use crate::protocol::{DoneSeen, Event, Starting};
use crate::settings::Strength;
use crate::store::threads::ThreadStore;
use crate::store::{Entry, Status, Thread};
use crate::workflow::picking::{installed, pick, usage_left, Left, Pick, PickError, Situation};
use crate::workflow::{moved, picked, reported, Chooser, Happening, Stage, Workflow};

/// Who moved a thread to another stage, for the line in the thread.
pub(super) enum Mover {
    /// The thread's agent, through `teryon thread stage` or by choosing a workflow.
    Agent(Option<Agent>),
    /// Something the person did, said in a few words, such as "you asked for a review".
    Person(String),
    /// Something Teryon saw, such as "a change started".
    Teryon(String),
}

/// How a turn started, kept until it ends to decide whether another agent should take over.
#[derive(Clone, Copy, Debug)]
pub(super) struct TurnStart {
    pub voice: Voice,
    pub stage: Stage,
    /// The provider whose usage ran out during the turn, if it did.
    pub ran_out: Option<Agent>,
    /// Whether the person moved the thread by choosing another workflow, rather than a turn moving it.
    pub rechosen: bool,
}

impl TurnStart {
    /// How a turn of `voice` in `stage` starts.
    pub(super) fn of(voice: Voice, stage: Stage) -> Self {
        Self {
            voice,
            stage,
            ran_out: None,
            rechosen: false,
        }
    }
}

/// Whether `starter` is the person, who picks a put-away thread back up.
///
/// A message Teryon sends to carry a stage on must not. The thread was put
/// away because it asked nothing, and Teryon talking to itself is not the
/// person deciding to resume it.
pub(super) fn brings_back(starter: &Starter) -> bool {
    matches!(starter, Starter::Person)
}

/// Forgets what the board remembered when a thread moves to `to`.
///
/// Arriving in Done is a new receipt, so the finish has not been seen.
/// Any move brings a put-away thread back as well: a finish has to show
/// once, and any other stage is the thread asking something again.
fn landed(thread: &mut Thread, to: Stage) {
    thread.done_seen = false;
    match to {
        Stage::Talking
        | Stage::Planning
        | Stage::Implementing
        | Stage::Review
        | Stage::Ready
        | Stage::Done => thread.put_away = false,
    }
}

/// Who starts a turn: the person, or Teryon carrying a thread on to its next stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Starter {
    Person,
    /// Teryon, for the stage, on the agent picked for it, which the stage remembers.
    Teryon {
        stage: Stage,
        pick: Pick,
    },
}

/// What Teryon does when a turn ends.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Next {
    Nothing,
    /// Starts the thread's own agent, after switching it to this agent and model.
    Author {
        pick: Pick,
        message: String,
    },
    Review(Pick),
    /// Nothing starts, but the stage remembers the agent it would have picked, so it keeps it later.
    Keep(Pick),
    /// Nobody can work in the stage; the text says why.
    Refused(String),
}

impl Inner {
    pub(super) fn workflow_of(&self, thread: &Thread) -> Workflow {
        self.setup().workflows.of(&thread.workflow).clone()
    }

    /// Moves a thread to a stage and writes the line that says who moved it. The caller holds the lock.
    pub(super) fn move_to(
        &self,
        store: &mut ThreadStore,
        thread: &Thread,
        to: Stage,
        mover: Mover,
    ) -> Result<(Entry, Thread), WorkspaceError> {
        let workflows = self.setup().workflows.clone();
        let from = thread.stage;
        let (by, agent, why) = match &mover {
            Mover::Agent(agent) => (
                "agent",
                agent.map(Agent::name),
                format!("by {}", agent.map_or("the agent", Agent::label)),
            ),
            Mover::Person(why) => ("person", None, why.clone()),
            Mover::Teryon(why) => ("teryon", None, why.clone()),
        };
        let text = format!(
            "{} → {} · {why}",
            workflows.label(from),
            workflows.label(to)
        );
        let mut body = json!({ "text": text, "from": from, "to": to, "by": by });
        if let Some(agent) = agent {
            body["agent"] = agent.into();
        }
        let entry = store.append(thread.id, EntryKind::Stage.name(), body)?;
        let thread = store.change(thread.id, |thread| {
            landed(thread, to);
            thread.stage = to;
            thread.stage_since = Some(Utc::now());
        })?;
        Ok((entry, thread))
    }

    /// Moves a thread when something that happened to it calls for it. The caller holds the lock.
    pub(super) fn happen(
        &self,
        store: &mut ThreadStore,
        thread: &Thread,
        happening: Happening,
        mover: Mover,
    ) -> Result<Option<(Entry, Thread)>, WorkspaceError> {
        let workflow = self.workflow_of(thread);
        let Some(to) = moved(&workflow, thread.stage, happening) else {
            return Ok(None);
        };
        self.move_to(store, thread, to, mover).map(Some)
    }

    /// Who works in a stage, from its settings. Nothing when the stage doesn't choose.
    ///
    /// `excluded` is the change's author for a review; `ran_out` a provider
    /// that just ran out in this stage.
    pub(super) fn stage_pick(
        &self,
        thread: Option<&Thread>,
        stage: Stage,
        excluded: Option<Agent>,
        ran_out: Option<Agent>,
    ) -> Option<Result<Pick, PickError>> {
        let earlier = thread
            .and_then(|thread| thread.picks.get(&stage))
            .and_then(|name| Agent::parse(name));
        self.stage_pick_after(earlier, stage, excluded, ran_out)
    }

    /// Like [`Inner::stage_pick`], keeping `earlier` while it can still work.
    pub(super) fn stage_pick_after(
        &self,
        earlier: Option<Agent>,
        stage: Stage,
        excluded: Option<Agent>,
        ran_out: Option<Agent>,
    ) -> Option<Result<Pick, PickError>> {
        let (setting, label, agents) = {
            let setup = self.setup();
            let setting = setup
                .stages
                .of(stage)
                .filter(|setting| setting.chooses())
                .cloned()?;
            (setting, setup.workflows.label(stage), setup.agents.clone())
        };
        let on_machine: Vec<Agent> = Agent::ALL
            .into_iter()
            .filter(|agent| installed(agents.program(*agent)))
            .collect();
        let now = Utc::now();
        let limits = self.usage().limits();
        let left = |agent: Agent| -> Left {
            usage_left(
                limits.iter().find(|limits| limits.agent == agent.name()),
                now,
            )
        };
        let situation = Situation {
            stage: &label,
            author: excluded,
            earlier,
            ran_out,
            installed: &on_machine,
            left: &left,
        };
        Some(pick(&setting, &agents, &situation, &mut |count| {
            (Uuid::new_v4().as_u128() % count as u128) as usize
        }))
    }

    /// The model a stage names for an agent's strength. Nothing when the stage names no strength, or the agent has no model for it.
    ///
    /// For a person who picked an agent without a model: the stage still says how strong the work should be.
    pub(super) fn first_stage_choice(
        &self,
        stage: Stage,
        agent: Agent,
    ) -> (Option<String>, Option<Effort>) {
        let setup = self.setup();
        let Some(setting) = setup.stages.of(stage).filter(|setting| setting.chooses()) else {
            return (None, None);
        };
        setup
            .agents
            .choice_for(agent, setting.strength.unwrap_or(Strength::Medium))
    }

    /// Who the next new thread starts with, chosen now when the held choice no longer fits.
    ///
    /// The held choice stays while the same pick would be allowed to return
    /// it: asking with it as the earlier pick keeps it, and replaces it only
    /// when its provider cannot work, is not allowed, or no longer runs the
    /// model its strength names. Clients are told when what they show changes.
    pub(super) fn refresh_starting(&self) -> Option<Starting> {
        let mut held = self.starting();
        self.update_starting(&mut held, false);
        starting_of(held.as_ref())
    }

    /// Updates the held choice and tells clients when it changed.
    ///
    /// The caller holds the choice's lock, and keeps it from picking through
    /// saving the thread to [`Inner::advance_starting`], so two new threads
    /// never start on the same choice. The announcement is made under the
    /// lock too, so clients never end up showing an older choice. The first
    /// stage is read under it as well, so a settings save cannot be undone by
    /// a call that began before it.
    ///
    /// With `fresh`, the earlier choice is not kept, as after a thread started on it.
    pub(super) fn update_starting(&self, held: &mut Held<'_>, fresh: bool) {
        let first = self.setup().workflows.of("").first();
        let shown = starting_of(held.as_ref());
        let earlier = match fresh {
            true => None,
            false => held
                .as_ref()
                .and_then(|held| held.as_ref().ok())
                .map(|pick| pick.agent),
        };
        **held = self.stage_pick_after(earlier, first, None, None);
        if let (true, Some(Ok(pick))) = (fresh, held.as_ref()) {
            debug!(agent = pick.agent.name(), reason = %pick.why, "Selected the next automatic starting agent");
        }
        let now = starting_of(held.as_ref());
        if now != shown {
            self.announce(Event::StartingChanged { starting: now });
        }
    }

    /// Says in the thread who takes a stage, and hands the thread's own conversation to them. The caller holds the lock.
    ///
    /// A new program starts its own conversation, told what was said so far.
    /// When the reviewer is the program now asked to write, the reviewer goes:
    /// an agent cannot review its own change, and the next review picks
    /// another. Done under the same lock as the check that the thread is
    /// free, so a message the person sent meanwhile keeps its agent.
    pub(super) fn hand_over(
        &self,
        store: &mut ThreadStore,
        thread: &Thread,
        stage: Stage,
        pick: &Pick,
        author: bool,
    ) -> Result<(Vec<Entry>, Thread), WorkspaceError> {
        let label = self.setup().workflows.label(stage);
        let notice = EntryDraft::notice(&taking_over(pick, &label));
        let mut entries = vec![store.append(thread.id, notice.kind.name(), notice.body)?];
        let switching = thread.agent != pick.agent.name();
        let same_model = !switching
            && same_model_choice(
                pick.agent,
                thread.model.as_deref(),
                pick.model.as_deref(),
                pick.effort,
                &self.catalog_of(pick.agent),
            );
        let same_effort = pick.effort.is_none() || thread.effort == pick.effort;
        if !author || (!switching && same_model && same_effort) {
            return Ok((entries, thread.clone()));
        }
        if let (true, Some(current)) = (switching, Agent::parse(&thread.agent)) {
            let notice = EntryDraft::notice(&format!(
                "{} takes over from {}.",
                pick.agent.label(),
                current.label()
            ));
            let mut body = notice.body;
            body["takes_over"] = true.into();
            entries.push(store.append(thread.id, notice.kind.name(), body)?);
        }
        let agent = pick.agent;
        let (model, effort) = if !switching && same_model {
            self.resolve_choice(agent, thread.model.clone(), pick.effort.or(thread.effort))
        } else {
            self.resolve_choice(agent, pick.model.clone(), pick.effort)
        };
        let thread = store.change(thread.id, |thread| {
            thread.model = model;
            thread.effort = effort;
            if switching {
                thread.agent = agent.name().to_string();
                thread.conversation = None;
                thread.handover = true;
                thread.commands = Vec::new();
                if thread
                    .reviewer
                    .as_ref()
                    .is_some_and(|reviewer| reviewer.agent == agent.name())
                {
                    thread.reviewer = None;
                }
            }
        })?;
        Ok((entries, thread))
    }

    /// Where a thread's plan is, when it has one.
    pub(super) fn plan_of(&self, thread: &Thread) -> Option<PathBuf> {
        thread.plan.as_ref()?;
        let state = self.state();
        let project = state.project(thread.project_id).ok()?;
        Some(plan_path(&project.directory, thread.id))
    }

    /// Reads the thread's plan, and when it was written or changed, says so in the thread.
    ///
    /// Runs after each turn of the thread's agent and when the person opens
    /// the thread, so a plan edited by hand shows too. A first plan written
    /// while talking moves the thread to planning.
    pub(super) fn look_at_plan(&self, thread_id: Uuid) {
        let mut state = self.state();
        let Ok((_, thread)) = state.holding(thread_id) else {
            return;
        };
        let Ok(project) = state.project(thread.project_id) else {
            return;
        };
        let Some(plan) = read_plan(&plan_path(&project.directory, thread_id)) else {
            return;
        };
        if thread.plan.as_ref().map(|seen| &seen.fingerprint) == Some(&plan.fingerprint) {
            return;
        }
        let Ok((store, _)) = state.holding(thread_id) else {
            return;
        };
        let first = thread.plan.is_none();
        let body = json!({ "text": plan_line(&plan, first), "title": plan.title, "first": first });
        let noted = store
            .append(thread_id, EntryKind::Plan.name(), body)
            .map_err(WorkspaceError::from)
            .and_then(|entry| {
                let thread = store.change(thread_id, |thread| thread.plan = Some(plan))?;
                let moved = match first {
                    true => self.happen(
                        store,
                        &thread,
                        Happening::PlanWritten,
                        Mover::Teryon("a plan was written".to_string()),
                    )?,
                    false => None,
                };
                Ok((entry, thread, moved))
            });
        drop(state);
        match noted {
            Ok((entry, thread, moved)) => {
                self.announce(Event::EntryAdded { entry });
                self.announce(Event::ThreadChanged { thread });
                self.announce_moved(moved);
            }
            Err(problem) => warn!(%thread_id, %problem, "could not note a thread's plan"),
        }
    }

    /// Tells everyone about a stage line and the thread it moved.
    pub(super) fn announce_moved(&self, moved: Option<(Entry, Thread)>) {
        if let Some((entry, thread)) = moved {
            self.announce(Event::EntryAdded { entry });
            self.announce(Event::ThreadChanged { thread });
        }
    }
}

impl Workspace {
    /// Puts away every quiet thread in `project_id`, or in every project.
    ///
    /// The board is for work that asks something of the person. A quiet
    /// thread is still just a conversation, so the person can take the whole
    /// group off the board at once. Nothing is deleted. The check is made
    /// again under the lock, so one that started running after it was counted
    /// stays. The answer is how many were newly marked.
    pub fn put_away_quiet(&self, project_id: Option<Uuid>) -> Result<u32, WorkspaceError> {
        let mut state = self.inner.state();
        if let Some(id) = project_id {
            state.project(id)?;
        }
        let ids: Vec<Uuid> = state
            .all_threads()
            .into_iter()
            .filter(|thread| {
                let in_scope = match project_id {
                    Some(id) => thread.project_id == id,
                    None => true,
                };
                in_scope && thread.quiet()
            })
            .map(|thread| thread.id)
            .collect();
        let mut marked = Vec::new();
        for id in ids {
            let (store, thread) = state.holding(id)?;
            if !thread.quiet() {
                continue;
            }
            marked.push(store.change(id, |thread| thread.put_away = true)?);
        }
        drop(state);
        for thread in marked.iter().cloned() {
            self.inner.announce(Event::ThreadChanged { thread });
        }
        Ok(marked.len() as u32)
    }

    /// Marks finishes the board showed during a visit.
    ///
    /// A thread is marked only when it is still Done and it entered Done at
    /// or before the time the board showed it. The instant is `stage_since`,
    /// or `created_at` when that was not stored. A later finish, a thread in
    /// another stage, and an unknown thread are left alone, and one of those
    /// does not throw away the rest.
    pub fn acknowledge_done(&self, seen: &[DoneSeen]) -> Result<(), WorkspaceError> {
        let mut state = self.inner.state();
        let mut marked = Vec::new();
        for item in seen {
            let (store, thread) = match state.holding(item.thread_id) {
                Ok(found) => found,
                Err(_) => continue,
            };
            if thread.stage != Stage::Done || thread.done_seen {
                continue;
            }
            let entered = thread.stage_since.unwrap_or(thread.created_at);
            if entered > item.at {
                continue;
            }
            marked.push(store.change(item.thread_id, |thread| thread.done_seen = true)?);
        }
        drop(state);
        for thread in marked {
            self.inner.announce(Event::ThreadChanged { thread });
        }
        Ok(())
    }

    /// Moves a thread to the stage its agent reported, through `teryon thread stage`.
    ///
    /// Only the agent running the thread's current turn may, as for a title.
    /// The next stage's agent starts once the turn ends, see [`Self::carry_on`].
    /// Refusals are logged, so a thread stuck in a stage can be explained.
    pub fn report_stage(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        stage: &str,
    ) -> Result<(), WorkspaceError> {
        let reporting = self.take_stage(thread_id, turn, stage);
        if let Err(problem) = &reporting {
            warn!(%thread_id, %problem, "refused the stage an agent reported");
        }
        self.inner.announce_moved(reporting?);
        Ok(())
    }

    fn take_stage(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        stage: &str,
    ) -> Result<Option<(Entry, Thread)>, WorkspaceError> {
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        let voice = self.agent_turn(&thread, turn, "teryon thread stage")?;
        let workflow = self.inner.workflow_of(&thread);
        let to = reported(&workflow, thread.stage, stage, voice)
            .map_err(|problem| WorkspaceError::Invalid(problem.to_string()))?;
        if to == thread.stage {
            return Ok(None);
        }
        let agent = Agent::parse(&thread.agent);
        self.inner
            .move_to(store, &thread, to, Mover::Agent(agent))
            .map(Some)
    }

    /// Which of a thread's agents runs the turn, refusing anyone else.
    fn agent_turn(
        &self,
        thread: &Thread,
        turn: Uuid,
        command: &str,
    ) -> Result<Voice, WorkspaceError> {
        if self.inner.runner.turn_of(thread.id) != Some(turn) {
            return Err(WorkspaceError::Invalid(format!(
                "`{command}` is for agents working in Teryon, during their turn."
            )));
        }
        let reviewing = thread
            .reviewer
            .as_ref()
            .is_some_and(|reviewer| reviewer.reviewing);
        Ok(if reviewing {
            Voice::Reviewer
        } else {
            Voice::Author
        })
    }

    /// The text of a thread's plan, for the Plan screen.
    pub fn plan(&self, thread_id: Uuid) -> Result<String, WorkspaceError> {
        let path = {
            let mut state = self.inner.state();
            let (_, thread) = state.holding(thread_id)?;
            plan_path(&state.project(thread.project_id)?.directory, thread_id)
        };
        let text = read_to_string(&path)
            .map_err(|_| WorkspaceError::Invalid("This thread has no plan yet.".to_string()))?;
        if text.len() > LARGEST_PLAN {
            return Err(WorkspaceError::Invalid(
                "This plan is larger than 1 MB, too large to show.".to_string(),
            ));
        }
        Ok(text)
    }

    /// Puts a thread on another workflow: chosen by its agent during `turn`, or by the person without one.
    ///
    /// The agent may only choose at the start of a thread, before any change.
    /// A thread in a stage the new workflow skips moves on to the next one it
    /// has. The providers picked for each stage are forgotten, since the new
    /// workflow may want others.
    pub fn choose_workflow(
        &self,
        thread_id: Uuid,
        turn: Option<Uuid>,
        name: &str,
    ) -> Result<Thread, WorkspaceError> {
        let choosing = self.take_workflow(thread_id, turn, name);
        if let (Err(problem), Some(_)) = (&choosing, turn) {
            warn!(%thread_id, %problem, "refused the workflow an agent chose");
        }
        let (entries, thread, from) = choosing?;
        for entry in entries {
            self.inner.announce(Event::EntryAdded { entry });
        }
        self.inner.announce(Event::ThreadChanged {
            thread: thread.clone(),
        });
        // A thread the person moved while nobody works gets the new stage's agent, as a turn ending there would.
        // During a turn, the turn's end does that.
        if turn.is_none() && thread.stage != from && thread.status != Status::Running {
            self.carry_on(
                thread_id,
                TurnStart {
                    rechosen: true,
                    ..TurnStart::of(Voice::Author, from)
                },
            );
        }
        Ok(thread)
    }

    fn take_workflow(
        &self,
        thread_id: Uuid,
        turn: Option<Uuid>,
        name: &str,
    ) -> Result<(Vec<Entry>, Thread, Stage), WorkspaceError> {
        let mut state = self.inner.state();
        let (store, thread) = state.holding(thread_id)?;
        let from = thread.stage;
        let chooser = match turn {
            Some(turn) => {
                if self.agent_turn(&thread, turn, "teryon thread workflow")? == Voice::Reviewer {
                    return Err(WorkspaceError::Invalid(
                        "`teryon thread workflow` is for the thread's own agent.".to_string(),
                    ));
                }
                Chooser::Agent
            }
            None => Chooser::Person,
        };
        let workflows = self.inner.setup().workflows.clone();
        let current = workflows.of(&thread.workflow).clone();
        let (workflow, stage) = picked(
            &workflows,
            &current,
            thread.stage,
            thread.change.is_some() || thread.finished.is_some(),
            name.trim(),
            chooser,
        )
        .map_err(|problem| WorkspaceError::Invalid(problem.to_string()))?;
        if workflow.name == current.name {
            return Ok((Vec::new(), thread, from));
        }
        let by = match chooser {
            Chooser::Agent => format!(
                " · by {}",
                Agent::parse(&thread.agent).map_or("the agent", Agent::label)
            ),
            Chooser::Person => " · by you".to_string(),
        };
        let notice = EntryDraft::notice(&format!("Follows the {} workflow{by}.", workflow.name));
        let mut entries = vec![store.append(thread_id, notice.kind.name(), notice.body)?];
        let name = workflow.name.clone();
        let skipped = workflows.label(thread.stage);
        let person = chooser == Chooser::Person;
        let mut thread = store.change(thread_id, |thread| {
            thread.workflow = name.clone();
            thread.picks.clear();
            // The person choosing a workflow picks the thread up. The agent
            // choosing one does not, unless the move below takes it to another stage.
            if person {
                thread.put_away = false;
            }
        })?;
        if stage != thread.stage {
            let why = format!("the {name} workflow skips {skipped}");
            let (entry, moved) = self
                .inner
                .move_to(store, &thread, stage, Mover::Teryon(why))?;
            entries.push(entry);
            thread = moved;
        }
        Ok((entries, thread, from))
    }

    /// Starts the next stage's agent when a turn ends, if the stage calls for one.
    ///
    /// Nothing starts unless `config.toml` names who works in some stage,
    /// the stage changed during the turn or the turn's provider ran out of
    /// usage, and never while the person has stopped the thread or after
    /// `automatic_rounds` starts without them.
    /// Problems go into the thread as an error, so the person sees why it waits.
    pub(super) fn carry_on(&self, thread_id: Uuid, start: TurnStart) {
        let (limit, chooses) = {
            let setup = self.inner.setup();
            (setup.workflow.automatic_rounds, setup.stages.any_chooses())
        };
        let thread = match self.inner.state().holding(thread_id) {
            Ok((_, thread)) => thread,
            Err(_) => return,
        };
        let moved = thread.stage != start.stage;
        // Shutting down stops agents; their turns ending must not start others.
        if self.inner.stopping.load(Ordering::SeqCst)
            || thread.status == Status::Running
            || thread.held
            || limit == 0
            || !chooses
            || (!moved && start.ran_out.is_none())
        {
            return;
        }
        let next = self.next_step(&thread, &start);
        let stage = thread.stage;
        match &next {
            Next::Nothing => return,
            Next::Keep(pick) => return self.remember(thread_id, stage, pick.agent),
            _ => {}
        }
        if thread.rounds >= limit {
            warn!(%thread_id, rounds = thread.rounds, "stopped starting agents without the person");
            self.write(
                thread_id,
                EntryDraft::notice(&format!(
                    "Stopped after {limit} {} without you. Send a message to carry on.",
                    if limit == 1 { "round" } else { "rounds" }
                )),
            );
            return;
        }
        let started = match next {
            Next::Nothing | Next::Keep(_) => Ok(()),
            Next::Refused(why) => {
                self.write(thread_id, EntryDraft::error(&why));
                Ok(())
            }
            Next::Review(pick) => {
                info!(%thread_id, stage = stage.name(), agent = pick.agent.name(), model = ?pick.model, rounds = thread.rounds + 1, "starting a review by itself");
                let (agent, model) = (pick.agent, pick.model.clone());
                self.review(
                    thread_id,
                    agent,
                    model,
                    pick.effort,
                    Starter::Teryon { stage, pick },
                )
                .map(|_| ())
            }
            Next::Author { pick, message } => {
                info!(%thread_id, stage = stage.name(), agent = pick.agent.name(), model = ?pick.model, rounds = thread.rounds + 1, "starting an agent by itself");
                self.send(
                    thread_id,
                    &message,
                    Voice::Author,
                    Starter::Teryon { stage, pick },
                )
                .map(|_| ())
            }
        };
        match started {
            // The person sent a message or asked for a review in the meantime; their turn wins.
            Err(WorkspaceError::Busy(_)) => {}
            Err(problem) => {
                warn!(%thread_id, %problem, "could not start the next stage's agent");
                self.write(thread_id, EntryDraft::error(&problem.to_string()));
            }
            Ok(()) => {}
        }
    }

    /// Remembers the agent picked for a stage without starting it, so the stage keeps it next time.
    fn remember(&self, thread_id: Uuid, stage: Stage, agent: Agent) {
        let mut state = self.inner.state();
        let Ok((store, thread)) = state.holding(thread_id) else {
            return;
        };
        if thread.picks.get(&stage).map(String::as_str) == Some(agent.name()) {
            return;
        }
        let changed = store.change(thread_id, |thread| {
            thread.picks.insert(stage, agent.name().to_string());
        });
        drop(state);
        match changed {
            Ok(thread) => self.inner.announce(Event::ThreadChanged { thread }),
            Err(problem) => warn!(%thread_id, %problem, "could not remember a stage's agent"),
        }
    }

    /// Who works next, from the stage the thread is in now and how the turn went.
    fn next_step(&self, thread: &Thread, start: &TurnStart) -> Next {
        let stage = thread.stage;
        if !stage.worked() {
            return Next::Nothing;
        }
        let moved = stage != start.stage;
        let author = Agent::parse(&thread.agent);
        let picking = |excluded: Option<Agent>| {
            self.inner
                .stage_pick(Some(thread), stage, excluded, start.ran_out)
        };
        if stage == Stage::Review {
            if !moved && start.voice != Voice::Reviewer {
                return Next::Nothing;
            }
            let chosen = match (picking(author), &thread.reviewer) {
                (Some(chosen), _) => chosen,
                (None, Some(reviewer)) => match Agent::parse(&reviewer.agent) {
                    Some(agent) if start.ran_out != Some(agent) => Ok(Pick {
                        agent,
                        model: reviewer.model.clone(),
                        effort: reviewer.effort,
                        why: format!("{} as before", agent.label()),
                    }),
                    _ => return Next::Nothing,
                },
                (None, None) => return Next::Nothing,
            };
            return match chosen {
                Ok(pick) => Next::Review(pick),
                Err(problem) => Next::Refused(problem.to_string()),
            };
        }
        let chosen = match picking(None) {
            Some(chosen) => chosen,
            None if start.ran_out.is_some() => return Next::Nothing,
            None => match author {
                Some(agent) => Ok(Pick {
                    agent,
                    model: thread.model.clone(),
                    effort: thread.effort,
                    why: format!("{} as before", agent.label()),
                }),
                None => return Next::Nothing,
            },
        };
        let pick = match chosen {
            Ok(pick) => pick,
            Err(problem) => return Next::Refused(problem.to_string()),
        };
        let same = Some(pick.agent) == author
            && same_model_choice(
                pick.agent,
                thread.model.as_deref(),
                pick.model.as_deref(),
                pick.effort,
                &self.inner.catalog_of(pick.agent),
            )
            && (pick.effort.is_none() || pick.effort == thread.effort);
        if start.voice == Voice::Author && same && start.ran_out.is_none() {
            // The agent carries on with the new stage in its own turn, or on the person's next message.
            return Next::Keep(pick);
        }
        let message = match (start.voice, start.ran_out) {
            (_, Some(agent)) => format!(
                "{} ran out of usage in the middle of the work above. Carry on where it stopped.",
                agent.label()
            ),
            (Voice::Reviewer, None) => self
                .findings(thread)
                .unwrap_or_else(|| "A reviewer looked at the change. Read the review above, and fix what you agree with.".to_string()),
            // The person chose another workflow: nothing was approved, only the stage moved.
            (Voice::Author, None) if start.rechosen => {
                opening(Stage::Talking, stage, self.inner.plan_of(thread).as_deref())
            }
            (Voice::Author, None) => {
                opening(start.stage, stage, self.inner.plan_of(thread).as_deref())
            }
        };
        Next::Author { pick, message }
    }

    /// What the reviewer found in the last review, as a message to the author, like the person's "Pass findings" draft.
    fn findings(&self, thread: &Thread) -> Option<String> {
        let entries = self.entries(thread.id).ok()?;
        let start = entries
            .iter()
            .rposition(|entry| entry.body["starts_review"] == true)
            .map_or(0, |position| position + 1);
        let written: Vec<&str> = entries[start..]
            .iter()
            .filter(|entry| {
                entry.kind == EntryKind::AgentMessage.name() && entry.body["reviewer"] == true
            })
            .filter_map(|entry| entry.body["text"].as_str())
            .collect();
        let review = written
            .iter()
            .rev()
            .find(|text| findings_in(text).is_some())
            .or(written.first())?;
        let findings: Vec<&str> = review
            .lines()
            .filter(|line| !line.trim_start().starts_with("Findings:"))
            .collect();
        Some(format!(
            "A reviewer looked at the change and found:\n\n{}\n\nFix what you agree with, and say why for anything you don't.",
            findings.join("\n").trim()
        ))
    }

    /// Adds an entry to a thread and marks it for the person's attention.
    fn write(&self, thread_id: Uuid, draft: EntryDraft) {
        let mut state = self.inner.state();
        let Ok((store, _)) = state.holding(thread_id) else {
            return;
        };
        let written = store
            .append(thread_id, draft.kind.name(), draft.body)
            .and_then(|entry| {
                store
                    .change(thread_id, |thread| thread.unread = true)
                    .map(|thread| (entry, thread))
            });
        drop(state);
        match written {
            Ok((entry, thread)) => {
                self.inner.announce(Event::EntryAdded { entry });
                self.inner.announce(Event::ThreadChanged { thread });
            }
            Err(problem) => warn!(%thread_id, %problem, "could not write to a thread"),
        }
    }
}

/// Whether a thread's current model already matches what a stage picked.
///
/// The chooser picks versioned catalog names (such as `claude-opus-5-5`), while
/// stage strengths often name Claude aliases (`opus`, `sonnet`, `haiku`), leave
/// the model unset to mean the provider's default, or name a grouped Gemini base
/// model. Treating equivalent choices as the same avoids unnecessary takeovers.
fn same_model_choice(
    agent: Agent,
    current: Option<&str>,
    picked: Option<&str>,
    picked_effort: Option<Effort>,
    catalog: &[ModelOption],
) -> bool {
    if current == picked {
        return true;
    }
    match (current, picked) {
        (Some(current), Some(picked)) => match agent {
            Agent::Claude => {
                claude_alias_matches(picked, current) || claude_alias_matches(current, picked)
            }
            Agent::Gemini => {
                let (picked_base, picked_suffix) = split_gemini_model(picked, catalog);
                let (current_base, _) = split_gemini_model(current, catalog);
                current_base == picked_base && (picked_suffix.is_none() || picked_effort.is_some())
            }
            Agent::Codex | Agent::Grok => false,
        },
        (Some(current), None) => {
            let id = match agent {
                Agent::Gemini => split_gemini_model(current, catalog).0,
                _ => current,
            };
            catalog.iter().any(|option| {
                option.is_default
                    && (option.id == id
                        || (agent == Agent::Claude && claude_alias_matches(id, &option.id)))
            })
        }
        (None, Some(_)) => false,
        (None, None) => true,
    }
}

/// "Codex · gpt-6-astra takes Implementing (only Codex)."
fn taking_over(pick: &Pick, stage: &str) -> String {
    let model = pick
        .model
        .as_deref()
        .map_or_else(String::new, |model| format!(" · {model}"));
    let effort = pick
        .effort
        .map_or_else(String::new, |effort| format!(" · {}", effort.label()));
    format!(
        "{}{model}{effort} takes {stage} ({}).",
        pick.agent.label(),
        pick.why
    )
}

/// What the agent of a stage is told when Teryon starts it, from the stage the thread came from.
///
/// A thread with a plan names the plan's file, so an agent of another
/// program, which only gets an account of the conversation, reads the plan itself.
pub(super) fn opening(from: Stage, to: Stage, plan: Option<&Path>) -> String {
    match (from, to, plan.map(Path::display)) {
        (Stage::Planning, Stage::Implementing, Some(plan)) => {
            format!("The plan in `{plan}` is approved. Implement it.")
        }
        (Stage::Planning, Stage::Implementing, None) => {
            "The plan above is approved. Implement it.".to_string()
        }
        (_, Stage::Implementing, Some(plan)) => format!("Implement the plan in `{plan}`."),
        (_, Stage::Implementing, None) => "Implement what was discussed above.".to_string(),
        (_, Stage::Planning, Some(plan)) => {
            format!("Look over the plan in `{plan}`, finish it, and ask the person to approve it.")
        }
        (_, Stage::Planning, None) => {
            "Work out a plan for what was discussed above, and ask the person to approve it."
                .to_string()
        }
        _ => "Carry on with the work above.".to_string(),
    }
}

/// The held choice of who the next new thread starts with, under its lock.
pub(super) type Held<'a> = MutexGuard<'a, Option<Result<Pick, PickError>>>;

/// What clients are told about a held choice.
pub(super) fn starting_of(held: Option<&Result<Pick, PickError>>) -> Option<Starting> {
    held.map(|held| match held {
        Ok(pick) => Starting::Provider {
            agent: pick.agent.name().to_string(),
            model: pick.model.clone(),
            effort: pick.effort,
        },
        Err(problem) => Starting::Unavailable {
            problem: problem.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::{read_to_string, write};

    use chrono::Duration;
    use lets_expect::lets_expect;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::*;
    use crate::protocol::Event;
    use crate::workspace::Setup;

    fn pick(model: Option<&str>) -> Pick {
        Pick {
            agent: Agent::Codex,
            model: model.map(str::to_string),
            effort: None,
            why: "only Codex".to_string(),
        }
    }

    /// A turn Teryon started, for the cases where that must not bring a thread back.
    fn teryon_start() -> Starter {
        Starter::Teryon {
            stage: Stage::Implementing,
            pick: pick(None),
        }
    }

    lets_expect! {
        expect(opening(from, to, plan)) {
            let plan: Option<&Path> = None;
            when(from = Stage::Planning, to = Stage::Implementing, plan = Some(Path::new("/p/plan.md"))) as an_approved_plan_file to name_it { equal("The plan in `/p/plan.md` is approved. Implement it.".to_string()) }
            when(from = Stage::Talking, to = Stage::Implementing, plan = Some(Path::new("/p/plan.md"))) as implementing_a_plan_file to name_it { equal("Implement the plan in `/p/plan.md`.".to_string()) }
            when(from = Stage::Talking, to = Stage::Planning, plan = Some(Path::new("/p/plan.md"))) as planning_with_a_plan_file to ask_to_finish_it { equal("Look over the plan in `/p/plan.md`, finish it, and ask the person to approve it.".to_string()) }
            when(from = Stage::Planning, to = Stage::Implementing) as an_approved_plan to ask_for_it_to_be_implemented { equal("The plan above is approved. Implement it.".to_string()) }
            when(from = Stage::Talking, to = Stage::Implementing) as implementing_without_a_plan to implement_the_conversation { equal("Implement what was discussed above.".to_string()) }
            when(from = Stage::Talking, to = Stage::Planning) as planning to ask_for_a_plan { equal("Work out a plan for what was discussed above, and ask the person to approve it.".to_string()) }
            when(from = Stage::Planning, to = Stage::Talking) as back_to_talking to carry_on { equal("Carry on with the work above.".to_string()) }
        }

        expect(taking_over(&pick, "Implementing")) {
            when(pick = pick(Some("gpt-6-astra"))) as a_model to name_it { equal("Codex · gpt-6-astra takes Implementing (only Codex).".to_string()) }
            when(pick = Pick { effort: Some(Effort::Xhigh), ..pick(Some("gpt-6-astra")) }) as a_model_and_effort to name_both { equal("Codex · gpt-6-astra · Extra high takes Implementing (only Codex).".to_string()) }
            when(pick = pick(None)) as the_default_model to leave_it_out { equal("Codex takes Implementing (only Codex).".to_string()) }
        }

        story same_model_choice_matches_aliases_defaults_and_gemini_variants {
            let claude = vec![ModelOption {
                id: "claude-opus-5-5".to_string(),
                label: "Opus 5.5".to_string(),
                efforts: vec![Effort::Low, Effort::Medium, Effort::High],
                default_effort: Some(Effort::Medium),
                is_default: true,
            }];
            let codex = vec![
                ModelOption {
                    id: "gpt-6-astra".to_string(),
                    label: "GPT-6 Astra".to_string(),
                    efforts: vec![Effort::Low, Effort::Medium, Effort::High],
                    default_effort: Some(Effort::Medium),
                    is_default: true,
                },
                ModelOption {
                    id: "gpt-6-sol".to_string(),
                    label: "GPT-6 Sol".to_string(),
                    efforts: vec![Effort::High],
                    default_effort: Some(Effort::High),
                    is_default: false,
                },
            ];
            let gemini = vec![ModelOption {
                id: "gemini-4-neon".to_string(),
                label: "Gemini 4 Neon".to_string(),
                efforts: vec![Effort::Low, Effort::High],
                default_effort: Some(Effort::High),
                is_default: true,
            }];

            expect(same_model_choice(Agent::Claude, Some("claude-opus-5-5"), Some("opus"), None, &claude)) to be_true
            expect(same_model_choice(Agent::Claude, Some("opus"), Some("claude-opus-5-5"), None, &claude)) to be_true
            expect(same_model_choice(Agent::Claude, Some("claude-sonnet-5-5"), Some("opus"), None, &claude)) to be_false
            expect(same_model_choice(Agent::Claude, None, Some("haiku"), None, &claude)) to be_false
            expect(same_model_choice(Agent::Codex, Some("gpt-6-astra"), None, None, &codex)) to be_true
            expect(same_model_choice(Agent::Codex, Some("gpt-6-sol"), None, None, &codex)) to be_false
            expect(same_model_choice(Agent::Gemini, Some("gemini-4-neon-high"), Some("gemini-4-neon"), None, &gemini)) to be_true
            expect(same_model_choice(Agent::Gemini, Some("gemini-4-neon-high"), Some("gemini-4-neon-low"), None, &gemini)) to be_false
            expect(same_model_choice(Agent::Gemini, Some("gemini-4-neon-high"), Some("gemini-4-neon-low"), Some(Effort::High), &gemini)) to be_true
            expect(same_model_choice(Agent::Gemini, Some("gemini-4-neon-high"), None, None, &gemini)) to be_true
        }

        expect(brings_back(&starter)) {
            when(starter = Starter::Person) as the_person to bring_the_thread_back { be_true }
            when(starter = teryon_start()) as teryon to leave_a_put_away_thread { be_false }
        }

        expect(flags_after(to)) {
            when(to = Stage::Done) as moving_to_done to clear_the_old_receipt_and_bring_it_back { equal((false, false)) }
            when(to = Stage::Talking) as moving_to_talking to clear_both { equal((false, false)) }
            when(to = Stage::Planning) as moving_to_planning to clear_both { equal((false, false)) }
            when(to = Stage::Implementing) as moving_to_implementing to clear_both { equal((false, false)) }
            when(to = Stage::Review) as moving_to_review to clear_both { equal((false, false)) }
            when(to = Stage::Ready) as moving_to_ready to clear_both { equal((false, false)) }
        }

        expect(finish_marked(before)) {
            when(before = false) as the_time_the_thread_finished to mark_it_and_leave_the_others { equal((true, false, Stage::Talking)) }
            when(before = true) as a_time_before_that_finish to mark_nothing { equal((false, false, Stage::Talking)) }
        }
    }

    /// `put_away` and `done_seen` after a thread that had both is moved to `to`.
    fn flags_after(to: Stage) -> (bool, bool) {
        let mut thread = Thread {
            id: Uuid::nil(),
            project_id: Uuid::nil(),
            title: String::new(),
            named: false,
            agent: String::new(),
            model: None,
            effort: None,
            conversation: None,
            handover: false,
            status: Status::Idle,
            unread: false,
            asking: false,
            commands: Vec::new(),
            change: None,
            finished: None,
            reviewer: None,
            workflow: String::new(),
            stage: Stage::Talking,
            stage_since: None,
            plan: None,
            rounds: 0,
            held: false,
            put_away: true,
            done_seen: true,
            picks: BTreeMap::new(),
            ended: None,
            preview: None,
            created_at: Utc::now(),
        };
        landed(&mut thread, to);
        (thread.put_away, thread.done_seen)
    }

    /// Whether a finish with no stored `stage_since` is marked, and what an unrelated thread keeps.
    ///
    /// `before` asks with a time one second earlier than the thread was created,
    /// which is the finish's instant when `stage_since` was never stored. The
    /// call also names an unknown thread, which must not throw the rest away.
    fn finish_marked(before: bool) -> (bool, bool, Stage) {
        let home = tempdir().unwrap();
        let directory = tempdir().unwrap();
        let (finished, other) = {
            let workspace = Workspace::open(home.path(), Setup::default()).unwrap();
            let project = workspace
                .add_project("Sample", directory.path().to_str().unwrap())
                .unwrap();
            let finished = workspace
                .start_thread(project.id, Some("claude"), None, "Finished")
                .unwrap();
            let other = workspace
                .start_thread(project.id, Some("claude"), None, "Talking")
                .unwrap();
            drop(workspace);
            let path = std::path::PathBuf::from(&project.directory)
                .join(".teryon/threads")
                .join(finished.id.to_string())
                .join("thread.json");
            let mut value: Value = serde_json::from_str(&read_to_string(&path).unwrap()).unwrap();
            let object = value.as_object_mut().unwrap();
            object.insert("stage".to_string(), serde_json::json!("done"));
            object.insert("done_seen".to_string(), serde_json::json!(false));
            object.remove("stage_since");
            write(&path, serde_json::to_string(&value).unwrap()).unwrap();
            (finished.id, other.id)
        };
        let workspace = Workspace::open(home.path(), Setup::default()).unwrap();
        let created = match workspace.snapshot() {
            Event::Snapshot { threads, .. } => {
                threads
                    .iter()
                    .find(|thread| thread.id == finished)
                    .unwrap()
                    .created_at
            }
            _ => unreachable!("a snapshot"),
        };
        let at = if before {
            created - Duration::seconds(1)
        } else {
            created
        };
        workspace
            .acknowledge_done(&[
                DoneSeen {
                    thread_id: Uuid::new_v4(),
                    at,
                },
                DoneSeen {
                    thread_id: other,
                    at,
                },
                DoneSeen {
                    thread_id: finished,
                    at,
                },
            ])
            .unwrap();
        match workspace.snapshot() {
            Event::Snapshot { threads, .. } => {
                let finished = threads.iter().find(|thread| thread.id == finished).unwrap();
                let other = threads.iter().find(|thread| thread.id == other).unwrap();
                (finished.done_seen, other.done_seen, other.stage)
            }
            _ => unreachable!("a snapshot"),
        }
    }
}
