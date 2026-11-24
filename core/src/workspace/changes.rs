//! A thread's change, as the workspace handles it: starting one, looking at
//! it, claiming copies made by hand, opening it, and finishing it.
//!
//! Finishing (applying, catching up, discarding) changes the repository, so
//! two of those never run at once in one project, and an agent is never
//! started for a thread whose change is being finished.
//!
//! The rules about changes live in `crate::changes`; this is where they meet
//! the workspace's threads, its lock and its announcements. Kept apart from
//! the rest of the workspace because it is most of what the workspace does
//! with git besides reading it.

use std::collections::{HashMap, HashSet};
use std::env::temp_dir;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::sleep;
use tracing::{error, warn};
use uuid::Uuid;

use crate::agents::{EntryDraft, EntryKind};
use crate::changes::claim::{claimant, Candidate};
use crate::changes::create::{create, take_over};
use crate::changes::finish::{apply, catch_up, discard, draft_description, CaughtUp, FinishError};
use crate::changes::github::{review_now, send_for_review, tidy_after_acceptance, Tidied};
use crate::changes::open::{browser, command_line, file_manager, CommandLine};
use crate::changes::status::{branch_present, read_state, Look};
use crate::changes::{copies_in, Change, ChangeState, Finish, FoundCopy, Review, ReviewState};
use crate::protocol::{Event, Opener};
use crate::store::{Status, Thread};
use crate::workflow::Happening;
use crate::workspace::stages::Mover;
use crate::workspace::{Actor, Inner, Workspace, WorkspaceError};

/// How long Discard waits for a stopped agent to end.
const STOP_PATIENCE: Duration = Duration::from_secs(30);

impl Workspace {
    /// A starting point for the description of a thread's change, for the person to edit before applying.
    ///
    /// What the agent said about its work when it committed, or else the thread's title.
    pub async fn draft_description(&self, thread_id: Uuid) -> Result<String, WorkspaceError> {
        let (directory, _, thread, change) = self.inner.finishable(thread_id)?;
        let drafted = draft_description(&directory, &change).await?;
        Ok(drafted.unwrap_or_else(|| {
            if thread.title.is_empty() {
                change.name.replace('-', " ")
            } else {
                thread.title.clone()
            }
        }))
    }

    /// Applies a thread's change to the project as one entry in its history, and forgets the change.
    pub async fn apply_change(
        &self,
        thread_id: Uuid,
        description: &str,
    ) -> Result<(), WorkspaceError> {
        self.apply_change_by(thread_id, description, Actor::Person)
            .await
    }

    /// Applies a thread's change on behalf of `actor`, leaving an attributed notice when another thread acts.
    pub async fn apply_change_by(
        &self,
        thread_id: Uuid,
        description: &str,
        actor: Actor,
    ) -> Result<(), WorkspaceError> {
        let (directory, project_id, _, change) = self.inner.finishable(thread_id)?;
        let _turn = self.inner.finishing(thread_id)?;
        let _project = self.inner.project_lock(project_id).lock_owned().await;
        self.inner.check_idle(thread_id)?;
        let note = self
            .inner
            .state()
            .source(actor)
            .map_or_else(|| "Applied to the project.".to_string(), |s| s.notice("applied this change."));
        apply(&directory, &change, description).await?;
        self.inner
            .forget_change(thread_id, Finish::Applied, &note)?;
        self.inner.look_at_all(project_id).await;
        Ok(())
    }

    /// Brings a thread's change up to date with newer work in the project, unless the two overlap.
    pub async fn catch_up(&self, thread_id: Uuid) -> Result<CaughtUp, WorkspaceError> {
        self.catch_up_by(thread_id, Actor::Person).await
    }

    /// Brings a thread's change up to date on behalf of `actor`.
    pub async fn catch_up_by(
        &self,
        thread_id: Uuid,
        actor: Actor,
    ) -> Result<CaughtUp, WorkspaceError> {
        let (directory, project_id, _, change) = self.inner.finishable(thread_id)?;
        let _turn = self.inner.finishing(thread_id)?;
        let _project = self.inner.project_lock(project_id).lock_owned().await;
        self.inner.check_idle(thread_id)?;
        let source = self.inner.state().source(actor);
        let caught_up = catch_up(&directory, &change).await?;
        if caught_up == CaughtUp::Done {
            if let Some(source) = source {
                self.inner
                    .append_notice(thread_id, &source.notice("caught this change up."))?;
            }
        }
        self.inner.look_at(thread_id, Look::Thorough).await;
        Ok(caught_up)
    }

    /// Throws a thread's change away: its copy with everything in it, and its branch.
    ///
    /// Stops the thread's agent first, if it is working.
    pub async fn discard_change(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        self.discard_change_by(thread_id, Actor::Person).await
    }

    /// Throws a thread's change away on behalf of `actor`, leaving an attributed notice when another thread acts.
    pub async fn discard_change_by(
        &self,
        thread_id: Uuid,
        actor: Actor,
    ) -> Result<(), WorkspaceError> {
        let (directory, project_id, change, source) = {
            let mut state = self.inner.state();
            let source = state.source(actor);
            let (_, thread) = state.holding(thread_id)?;
            let change = thread.change.ok_or(FinishError::NoChange)?;
            (
                state.project(thread.project_id)?.directory.clone(),
                thread.project_id,
                *change,
                source,
            )
        };
        let _turn = self.inner.finishing(thread_id)?;
        match &source {
            Some(source) => self
                .inner
                .runner
                .stop_by_thread(thread_id, source.notice("stopped this thread.")),
            None => self.inner.runner.stop(thread_id),
        }
        for _ in 0..STOP_PATIENCE.as_millis() / 50 {
            if self.inner.runner.turn_of(thread_id).is_none() {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        let _project = self.inner.project_lock(project_id).lock_owned().await;
        let note = source.map_or_else(
            || "Changes discarded.".to_string(),
            |s| s.notice("discarded this change."),
        );
        discard(&directory, &change).await?;
        self.inner
            .forget_change(thread_id, Finish::Discarded, &note)?;
        self.inner.look_at_all(project_id).await;
        Ok(())
    }

    /// Starts a change for a thread: a copy of the project on a new branch, named by the agent.
    ///
    /// Only the agent running the thread's current turn may, which it proves
    /// with the turn's identifier from its environment. That keeps a command
    /// copied to a terminal, or run after the turn, from touching the thread.
    /// Returns where the copy is.
    pub async fn start_change(
        &self,
        thread_id: Uuid,
        turn: Uuid,
        name: &str,
        from: Option<&str>,
    ) -> Result<PathBuf, WorkspaceError> {
        let (directory, project_id) = {
            let mut state = self.inner.state();
            let (_, thread) = state.holding(thread_id)?;
            if self.inner.runner.turn_of(thread_id) != Some(turn) {
                return Err(WorkspaceError::Invalid(
                    "`teryon worktree create` is for agents working in Teryon, during their turn."
                        .to_string(),
                ));
            }
            if let Some(change) = &thread.change {
                return Err(WorkspaceError::Invalid(format!(
                    "This thread already has a worktree at `{}`. Work there.",
                    change.path.display()
                )));
            }
            (
                state.project(thread.project_id)?.directory.clone(),
                thread.project_id,
            )
        };

        // Noticed before it exists, so a look in between cannot take it for one
        // the agent made by hand and give it away with a guessed starting point.
        self.inner.notice(project_id, name);
        let change = create(&directory, name, from).await?;
        let path = change.path.clone();
        self.inner.adopt(thread_id, change)?;
        self.inner.look_at(thread_id, Look::Quick).await;
        Ok(path)
    }

    /// Reads again where a thread's change stands, closely, and how its review is going. For when a person opens the thread.
    pub async fn refresh_change(&self, thread_id: Uuid) {
        self.inner.look_at_plan(thread_id);
        self.inner.look_at(thread_id, Look::Thorough).await;
        self.inner.follow_review(thread_id).await;
    }

    /// Sends a thread's change for review on GitHub, described by `description`.
    pub async fn send_for_review(
        &self,
        thread_id: Uuid,
        description: &str,
    ) -> Result<(), WorkspaceError> {
        self.send_for_review_by(thread_id, description, Actor::Person)
            .await
    }

    /// Sends a thread's change for review on GitHub on behalf of `actor`.
    pub async fn send_for_review_by(
        &self,
        thread_id: Uuid,
        description: &str,
        actor: Actor,
    ) -> Result<(), WorkspaceError> {
        let (directory, project_id, _, change) = self.inner.finishable(thread_id)?;
        let _turn = self.inner.finishing(thread_id)?;
        let _project = self.inner.project_lock(project_id).lock_owned().await;
        self.inner.check_idle(thread_id)?;
        let source = self.inner.state().source(actor);
        let program = self.inner.setup().github.program.clone();
        let review = send_for_review(&directory, &change, description, &program).await?;
        self.inner.store_review(thread_id, review)?;
        if let Some(source) = source {
            self.inner
                .append_notice(thread_id, &source.notice("sent this change for review."))?;
        }
        self.inner.look_at(thread_id, Look::Thorough).await;
        Ok(())
    }

    /// Opens a thread's review on GitHub in the person's browser.
    pub fn open_review(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        let (_, thread) = self.inner.state().holding(thread_id)?;
        let review = thread
            .change
            .and_then(|change| change.review)
            .ok_or(WorkspaceError::Busy(
                "This change has not been sent for review.",
            ))?;
        let directory = temp_dir();
        start_detached(&browser(&review.url), &directory)
    }

    /// Opens a thread's copy in the file manager, or in the editor or terminal set up for it.
    pub fn open_copy(&self, thread_id: Uuid, with: Opener) -> Result<(), WorkspaceError> {
        let (_, thread) = self.inner.state().holding(thread_id)?;
        let change = thread
            .change
            .ok_or(WorkspaceError::Busy("This thread has no changes to open."))?;
        if !change.path.is_dir() {
            return Err(WorkspaceError::Busy(
                "This thread's copy was removed outside Teryon.",
            ));
        }
        let open = self.inner.setup().open.clone();
        let line = match with {
            Opener::Folder => Some(file_manager(&change.path)),
            Opener::Editor => open
                .editor
                .as_deref()
                .and_then(|editor| command_line(editor, &change.path)),
            Opener::Terminal => open
                .terminal
                .as_deref()
                .and_then(|terminal| command_line(terminal, &change.path)),
        }
        .ok_or(WorkspaceError::Busy(
            "Nothing is set up to open it with. Set it in Settings.",
        ))?;
        start_detached(&line, &change.path)
    }
}

/// Marks a thread as having its change finished, until dropped.
pub(super) struct Finishing {
    inner: Arc<Inner>,
    thread_id: Uuid,
}

impl Drop for Finishing {
    fn drop(&mut self) {
        self.inner.finishing_threads().remove(&self.thread_id);
    }
}

impl Inner {
    /// The project folder, the project, the thread and its change, for finishing it while no agent works in it.
    fn finishable(
        &self,
        thread_id: Uuid,
    ) -> Result<(PathBuf, Uuid, Thread, Change), WorkspaceError> {
        let mut state = self.state();
        let (_, thread) = state.holding(thread_id)?;
        let change = thread.change.clone().ok_or(FinishError::NoChange)?;
        if thread.status == Status::Running {
            return Err(FinishError::AgentRunning.into());
        }
        let directory = state.project(thread.project_id)?.directory.clone();
        Ok((directory, thread.project_id, thread, *change))
    }

    /// Fails when an agent started working in the thread meanwhile.
    fn check_idle(&self, thread_id: Uuid) -> Result<(), WorkspaceError> {
        let (_, thread) = self.state().holding(thread_id)?;
        if thread.status == Status::Running {
            return Err(FinishError::AgentRunning.into());
        }
        Ok(())
    }

    /// Notes that a thread's change is being finished, so no agent is started for it meanwhile.
    fn finishing(self: &Arc<Self>, thread_id: Uuid) -> Result<Finishing, WorkspaceError> {
        if !self.finishing_threads().insert(thread_id) {
            return Err(WorkspaceError::Busy(
                "Teryon is already finishing this thread's changes.",
            ));
        }
        Ok(Finishing {
            inner: self.clone(),
            thread_id,
        })
    }

    pub(super) fn finishing_threads(&self) -> MutexGuard<'_, HashSet<Uuid>> {
        self.finishing_threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The lock that keeps two changes in one project from being finished at once.
    fn project_lock(&self, project_id: Uuid) -> Arc<AsyncMutex<()>> {
        self.project_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(project_id)
            .or_default()
            .clone()
    }

    /// Forgets a thread's change, remembers how it ended, and says so in the thread.
    fn forget_change(
        &self,
        thread_id: Uuid,
        finish: Finish,
        note: &str,
    ) -> Result<(), WorkspaceError> {
        self.forget_change_when(thread_id, finish, note, |_| true)
    }

    /// Like [`Self::forget_change`], but only when `wanted` holds for the thread, checked under the same lock.
    fn forget_change_when(
        &self,
        thread_id: Uuid,
        finish: Finish,
        note: &str,
        wanted: impl FnOnce(&Thread) -> bool,
    ) -> Result<(), WorkspaceError> {
        let mut state = self.state();
        let (store, thread) = state.holding(thread_id)?;
        if !wanted(&thread) {
            return Ok(());
        }
        let notice = EntryDraft::notice(note);
        let entry = store.append(thread_id, EntryKind::Notice.name(), notice.body)?;
        let thread = store.change(thread_id, |thread| {
            thread.change = None;
            thread.finished = Some(finish);
            // The reviewer goes with the change: it only ever works inside the change's copy.
            thread.reviewer = None;
        })?;
        let why = match finish {
            Finish::Applied => "applied to the project",
            Finish::Accepted => "accepted on GitHub",
            Finish::Discarded => "discarded",
            Finish::Removed => "its copy was removed",
        };
        let moved = self.happen(
            store,
            &thread,
            Happening::ChangeFinished,
            Mover::Teryon(why.to_string()),
        )?;
        drop(state);
        self.announce(Event::EntryAdded { entry });
        self.announce(Event::ThreadChanged { thread });
        self.announce_moved(moved);
        Ok(())
    }

    /// Asks GitHub how a thread's review is going, and tidies the change away once it is accepted.
    ///
    /// Problems are logged: this runs when someone opens the thread, with
    /// nobody waiting for it, and the next opening asks again.
    async fn follow_review(self: &Arc<Self>, thread_id: Uuid) {
        let Ok((directory, project_id, _, change)) = self.finishable(thread_id) else {
            return;
        };
        let Some(review) = change
            .review
            .clone()
            .filter(|review| review.state == ReviewState::InReview)
        else {
            return;
        };
        let program = self.setup().github.program.clone();
        let now = match review_now(&directory, &review, &program).await {
            Ok(now) => now,
            Err(problem) => {
                warn!(
                    "could not ask GitHub about the review of {}: {problem}",
                    change.name
                );
                return;
            }
        };
        match now.state {
            ReviewState::InReview => {}
            ReviewState::Declined => {
                if let Err(problem) = self.store_review(thread_id, now) {
                    error!("could not store a declined review: {problem}");
                }
            }
            ReviewState::Accepted => {
                let Ok(_turn) = self.finishing(thread_id) else {
                    return;
                };
                let _project = self.project_lock(project_id).lock_owned().await;
                let note = match tidy_after_acceptance(&directory, &change).await {
                    Ok(Tidied::UpToDate) => "Accepted on GitHub.",
                    Ok(Tidied::Behind) => {
                        "Accepted on GitHub. Your project folder does not have it yet."
                    }
                    Err(problem) => {
                        warn!(
                            "could not tidy away the accepted change {}: {problem}",
                            change.name
                        );
                        return;
                    }
                };
                if let Err(problem) = self.forget_change(thread_id, Finish::Accepted, note) {
                    error!("could not forget an accepted change: {problem}");
                }
                self.look_at_all(project_id).await;
            }
        }
    }

    /// Stores how a thread's review is going.
    fn store_review(&self, thread_id: Uuid, review: Review) -> Result<(), WorkspaceError> {
        let mut state = self.state();
        let (store, _) = state.holding(thread_id)?;
        let thread = store.change(thread_id, |thread| {
            if let Some(change) = thread.change.as_mut() {
                change.review = Some(review);
            }
        })?;
        drop(state);
        self.announce(Event::ThreadChanged { thread });
        Ok(())
    }

    /// Adds a notice to a thread and announces it.
    fn append_notice(&self, thread_id: Uuid, note: &str) -> Result<(), WorkspaceError> {
        let mut state = self.state();
        let (store, _) = state.holding(thread_id)?;
        let notice = EntryDraft::notice(note);
        let entry = store.append(thread_id, EntryKind::Notice.name(), notice.body)?;
        drop(state);
        self.announce(Event::EntryAdded { entry });
        Ok(())
    }

    /// Looks closely at every change in a project again: finishing one moves the project, which the others start from.
    async fn look_at_all(&self, project_id: Uuid) {
        let threads: Vec<Uuid> = {
            let state = self.state();
            state
                .all_threads()
                .into_iter()
                .filter(|thread| thread.project_id == project_id && thread.change.is_some())
                .map(|thread| thread.id)
                .collect()
        };
        for thread_id in threads {
            self.look_at(thread_id, Look::Thorough).await;
        }
    }

    /// Gives a thread a change, unless it got one meanwhile, and tells everyone.
    fn adopt(&self, thread_id: Uuid, change: Change) -> Result<(), WorkspaceError> {
        let mut state = self.state();
        let (store, thread) = state.holding(thread_id)?;
        if thread.change.is_some() {
            return Ok(());
        }
        let thread = store.change(thread_id, |thread| {
            thread.change = Some(Box::new(change));
            thread.finished = None;
            // A new change gets a reviewer of its own: the last one's findings and conversation were about other work.
            thread.reviewer = None;
        })?;
        let moved = self.happen(
            store,
            &thread,
            Happening::ChangeStarted,
            Mover::Teryon("a change started".to_string()),
        )?;
        drop(state);
        self.announce(Event::ThreadChanged { thread });
        self.announce_moved(moved);
        Ok(())
    }

    /// Reads where a thread's change stands and stores it when it differs.
    ///
    /// A thread without a change takes over a copy its agent made by hand,
    /// when [`claimant`] says it is this thread's. Problems are logged: this
    /// runs on its own, with nobody to tell, and the next look tries again.
    pub(super) async fn look_at(&self, thread_id: Uuid, look: Look) {
        let found = {
            let mut state = self.state();
            let Ok((store, thread)) = state.holding(thread_id) else {
                return;
            };
            let candidates: Vec<Candidate> = store
                .threads()
                .iter()
                .filter(|other| other.change.is_none())
                .map(|other| Candidate {
                    thread_id: other.id,
                    running: other.status == Status::Running,
                })
                .collect();
            let taken: Vec<String> = store
                .threads()
                .iter()
                .filter_map(|other| other.change.as_ref().map(|change| change.name.clone()))
                .collect();
            let Ok(project) = state.project(thread.project_id) else {
                return;
            };
            (project.directory.clone(), thread, candidates, taken)
        };
        let (directory, thread, candidates, taken) = found;

        let change = match thread.change {
            Some(change) => *change,
            None => {
                self.claim_copies(thread.project_id, &directory, &candidates, &taken)
                    .await;
                let claimed = self
                    .state()
                    .holding(thread_id)
                    .ok()
                    .and_then(|(_, thread)| thread.change);
                match claimed {
                    Some(change) => *change,
                    None => return,
                }
            }
        };
        let (state, branch) = match read_state(&directory, &change, look).await {
            Ok(read) => read,
            Err(problem) => {
                warn!(
                    "could not read where the change {} stands: {problem}",
                    change.name
                );
                return;
            }
        };
        if !state.copy_present {
            let review_open = change
                .review
                .as_ref()
                .is_some_and(|review| review.state != ReviewState::Declined);
            match branch_present(&directory, &change.name).await {
                Ok(false) if !review_open => {
                    self.let_go(thread_id, &change);
                    return;
                }
                Ok(_) => {}
                Err(problem) => {
                    warn!(
                        "could not read whether the branch of {} is there: {problem}",
                        change.name
                    );
                    return;
                }
            }
        }
        let state = keep_known_fit(change.state.as_ref(), state, look);
        if change.state.as_ref() == Some(&state) && change.branch == branch {
            return;
        }

        let mut guarded = self.state();
        let Ok((store, current)) = guarded.holding(thread_id) else {
            return;
        };
        // Started, or taken over, since this look began: that look is the one to keep.
        if current
            .change
            .as_ref()
            .is_some_and(|known| known.name != change.name)
        {
            return;
        }
        let updated = Change {
            state: Some(state),
            branch,
            ..change
        };
        match store.change(thread_id, |thread| thread.change = Some(Box::new(updated))) {
            Ok(thread) => {
                drop(guarded);
                self.announce(Event::ThreadChanged { thread });
            }
            Err(problem) => error!("could not store where a change stands: {problem}"),
        }
    }

    /// Forgets a thread's `change` once nothing of it is left, unless Teryon is finishing it.
    ///
    /// Copies get removed for all kinds of reasons, by the person or by the
    /// agent, and none of them is a problem. Once the branch is gone too and
    /// no review on GitHub waits for it, there is nothing left to apply, so
    /// the thread simply no longer has a change. A change being applied or
    /// discarded loses its copy on purpose, and is forgotten by that.
    ///
    /// Checked and forgotten under one lock: two looks at the same thread
    /// can both find the copy gone, and only the first may forget, and only
    /// the change it looked at, not one started meanwhile.
    fn let_go(&self, thread_id: Uuid, change: &Change) {
        let still = |thread: &Thread| {
            !self.finishing_threads().contains(&thread_id)
                && thread
                    .change
                    .as_deref()
                    .is_some_and(|known| known.name == change.name && known.start == change.start)
        };
        if let Err(problem) =
            self.forget_change_when(thread_id, Finish::Removed, "Copy removed.", still)
        {
            error!("could not forget a change whose copy is gone: {problem}");
        }
    }

    /// Gives the copies in `.teryon/worktrees/` that no thread has to the threads they belong to, see [`claimant`].
    ///
    /// The first look at a project after Teryon starts only notes the copies
    /// already there: none of them is new, so only those named after a
    /// thread are given to it.
    async fn claim_copies(
        &self,
        project_id: Uuid,
        directory: &Path,
        candidates: &[Candidate],
        taken: &[String],
    ) {
        let copies = match copies_in(directory).await {
            Ok(copies) => copies,
            Err(problem) => {
                warn!(
                    "could not list the copies of {}: {problem}",
                    directory.display()
                );
                return;
            }
        };
        let owned: Vec<(Uuid, FoundCopy)> = {
            let mut noticed = self.noticed();
            let project = noticed.entry(project_id).or_default();
            let first_look = !project.looked;
            project.looked = true;
            copies
                .into_iter()
                .filter(|copy| !taken.contains(&copy.name))
                .filter_map(|copy| {
                    let new = project.names.insert(copy.name.clone()) && !first_look;
                    claimant(&copy.name, new, candidates).map(|owner| (owner, copy))
                })
                .collect()
        };
        for (owner, copy) in owned {
            let adopted = take_over(directory, &copy)
                .await
                .map_err(WorkspaceError::from)
                .and_then(|change| self.adopt(owner, change));
            if let Err(problem) = adopted {
                warn!(
                    "could not give the copy {} to its thread: {problem}",
                    copy.name
                );
            }
        }
    }

    /// Notes that a copy is not new, so it is never given to a thread by [`claimant`]'s guess.
    fn notice(&self, project_id: Uuid, name: &str) {
        self.noticed()
            .entry(project_id)
            .or_default()
            .names
            .insert(name.to_string());
    }

    fn noticed(&self) -> MutexGuard<'_, HashMap<Uuid, Noticed>> {
        self.noticed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The copies Teryon has noticed in one project since it started.
#[derive(Default)]
pub(super) struct Noticed {
    /// Whether the project's copies have been listed at all yet.
    looked: bool,
    names: HashSet<String>,
}

/// What a quick look says, with whether the change applies kept from the last thorough look.
///
/// A quick look does not check that, and saying "unknown" every second of a
/// turn would make the status line flicker.
fn keep_known_fit(known: Option<&ChangeState>, mut state: ChangeState, look: Look) -> ChangeState {
    if look == Look::Quick && state.copy_present && state.base_present {
        if let Some(known) = known {
            state.fit = known.fit;
        }
    }
    state
}

/// Starts a program in `directory` and leaves it running on its own.
fn start_detached(line: &CommandLine, directory: &Path) -> Result<(), WorkspaceError> {
    Command::new(&line.program)
        .args(&line.arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|problem| {
            WorkspaceError::Invalid(format!("Could not start `{}`: {problem}.", line.program))
        })
}
