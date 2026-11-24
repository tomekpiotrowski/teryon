//! A project's threads, kept in the project's own `.teryon/threads/` folder.
//!
//! Each thread is a folder named after it, holding `thread.json` (everything
//! but the conversation) and `entries.jsonl` (the conversation, one entry per
//! line). A conversation only ever grows at its end, so entries are added to
//! the end of their file and never rewritten. `thread.json` is small and
//! replaced whole on every change.
//!
//! `.teryon/` holds a `.gitignore` that ignores everything in it, so threads
//! and worktrees never show up in git, whatever the project's own ignore
//! files say. While a store is open it holds a lock on `.teryon/service.lock`,
//! so two Teryon services never write the same threads.
//!
//! Damage is contained rather than fatal: a cut-off last line, as after a
//! power cut, and a thread whose `thread.json` cannot be read are skipped and
//! logged, and everything else is still shown.

use std::collections::{BTreeMap, HashMap};
use std::fs::{create_dir_all, read_dir, read_to_string, File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::store::{replace_file, Entry, StoreError, Thread};
use crate::workflow::Stage;

/// How often, and how far apart, a held lock is tried.
const LOCK_ATTEMPTS: u32 = 10;
const LOCK_PAUSE: Duration = Duration::from_millis(20);

/// What `.teryon/.gitignore` says: ignore everything here, this file included.
const IGNORE_EVERYTHING: &str = "*\n";

/// The threads of one project.
pub struct ThreadStore {
    project_id: Uuid,
    folder: PathBuf,
    /// Held for as long as the store is open; dropping it releases the lock.
    _lock: File,
    threads: BTreeMap<Uuid, Thread>,
    /// Where each thread's conversation ends, once it has been read.
    ends: HashMap<Uuid, End>,
}

/// The end of a conversation file.
#[derive(Clone, Copy)]
struct End {
    position: u64,
    /// Whether the file ends in the middle of a line, which happens when
    /// writing it was cut off. The next entry starts on a new line then.
    broken: bool,
}

impl ThreadStore {
    /// Opens the threads of the project in `directory`, preparing `.teryon/` when needed.
    pub fn open(project_id: Uuid, directory: &Path) -> Result<Self, StoreError> {
        let teryon = directory.join(".teryon");
        let folder = teryon.join("threads");
        create_dir_all(&folder).map_err(StoreError::io(&folder))?;

        let ignore = teryon.join(".gitignore");
        if !ignore.exists() {
            replace_file(&ignore, IGNORE_EVERYTHING)?;
        }

        let lock_path = teryon.join("service.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(StoreError::io(&lock_path))?;
        take_lock(&lock, &lock_path)?;

        let threads = read_threads(project_id, &folder)?;
        Ok(Self {
            project_id,
            folder,
            _lock: lock,
            threads,
            ends: HashMap::new(),
        })
    }

    /// Every thread, oldest first.
    pub fn threads(&self) -> Vec<Thread> {
        let mut threads: Vec<Thread> = self.threads.values().cloned().collect();
        threads.sort_by_key(|thread| thread.created_at);
        threads
    }

    pub fn thread(&self, id: Uuid) -> Option<&Thread> {
        self.threads.get(&id)
    }

    /// Stores a new thread, or a changed one.
    pub fn save(&mut self, mut thread: Thread) -> Result<Thread, StoreError> {
        thread.project_id = self.project_id;
        let folder = self.folder.join(thread.id.to_string());
        create_dir_all(&folder).map_err(StoreError::io(&folder))?;
        let path = folder.join("thread.json");
        let text =
            serde_json::to_string_pretty(&thread).map_err(|source| StoreError::Unreadable {
                path: path.clone(),
                source,
            })?;
        replace_file(&path, &text)?;
        self.threads.insert(thread.id, thread.clone());
        Ok(thread)
    }

    /// Changes a thread through `change` and stores it. Returns the thread as stored.
    pub fn change(
        &mut self,
        id: Uuid,
        change: impl FnOnce(&mut Thread),
    ) -> Result<Thread, StoreError> {
        let mut thread = self
            .threads
            .get(&id)
            .cloned()
            .ok_or(StoreError::UnknownThread)?;
        change(&mut thread);
        self.save(thread)
    }

    /// A thread's conversation, in order.
    pub fn entries(&self, id: Uuid) -> Result<Vec<Entry>, StoreError> {
        if !self.threads.contains_key(&id) {
            return Err(StoreError::UnknownThread);
        }
        Ok(read_entries(&self.entries_path(id))?.0)
    }

    /// Adds an entry to the end of a thread's conversation.
    pub fn append(&mut self, id: Uuid, kind: &str, body: Value) -> Result<Entry, StoreError> {
        if !self.threads.contains_key(&id) {
            return Err(StoreError::UnknownThread);
        }
        let path = self.entries_path(id);
        let end = match self.ends.get(&id) {
            Some(end) => *end,
            None => read_entries(&path)?.1,
        };

        let entry = Entry {
            id: Uuid::new_v4(),
            thread_id: id,
            position: end.position + 1,
            kind: kind.to_string(),
            body,
            created_at: Utc::now(),
        };
        let mut line = if end.broken {
            "\n".to_string()
        } else {
            String::new()
        };
        line.push_str(
            &serde_json::to_string(&entry).map_err(|source| StoreError::Unreadable {
                path: path.clone(),
                source,
            })?,
        );
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(StoreError::io(&path))?;
        file.write_all(line.as_bytes())
            .map_err(StoreError::io(&path))?;
        file.flush().map_err(StoreError::io(&path))?;

        self.ends.insert(
            id,
            End {
                position: entry.position,
                broken: false,
            },
        );
        Ok(entry)
    }

    fn entries_path(&self, id: Uuid) -> PathBuf {
        self.folder.join(id.to_string()).join("entries.jsonl")
    }
}

/// Takes the project's lock, waiting a moment if it is held.
///
/// A lock just released can still be held for an instant by a program this
/// same Teryon is starting elsewhere: between being created and becoming the
/// program it was asked to be, a new process shares its parent's open files.
/// So a held lock is tried again for a short while before deciding that
/// another Teryon has the project.
fn take_lock(lock: &File, path: &Path) -> Result<(), StoreError> {
    for attempt in 0..LOCK_ATTEMPTS {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if attempt + 1 < LOCK_ATTEMPTS => sleep(LOCK_PAUSE),
            Err(TryLockError::WouldBlock) => return Err(StoreError::InUse),
            Err(TryLockError::Error(source)) => {
                return Err(StoreError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        }
    }
    Err(StoreError::InUse)
}

/// Reads every thread folder, skipping the ones that cannot be read.
fn read_threads(project_id: Uuid, folder: &Path) -> Result<BTreeMap<Uuid, Thread>, StoreError> {
    let mut threads = BTreeMap::new();
    for item in read_dir(folder).map_err(StoreError::io(folder))? {
        let item = item.map_err(StoreError::io(folder))?;
        let path = item.path().join("thread.json");
        let thread = read_to_string(&path)
            .map_err(|problem| problem.to_string())
            .and_then(|text| thread_from_file(&text));
        match thread {
            Ok(mut thread) => {
                thread.project_id = project_id;
                threads.insert(thread.id, thread);
            }
            Err(problem) => warn!(
                "skipped a thread that could not be read, {}: {problem}",
                path.display()
            ),
        }
    }
    Ok(threads)
}

/// Reads one thread file.
///
/// A thread stored before the board remembered finishes has no `done_seen`.
/// One of those already in Done counts as seen, so the first open does not
/// fill the column with old work. Anything else missing the field counts as
/// not seen. `put_away` missing means the thread is still on the board. The
/// file is left as it is; the next ordinary save writes the flags down.
fn thread_from_file(text: &str) -> Result<Thread, String> {
    let value: Value = serde_json::from_str(text).map_err(|problem| problem.to_string())?;
    let mut thread: Thread =
        serde_json::from_value(value.clone()).map_err(|problem| problem.to_string())?;
    if value.get("done_seen").is_none() && thread.stage == Stage::Done {
        thread.done_seen = true;
    }
    Ok(thread)
}

/// Reads a conversation file, and where it ends. A missing file is an empty conversation.
fn read_entries(path: &Path) -> Result<(Vec<Entry>, End), StoreError> {
    let text = match read_to_string(path) {
        Ok(text) => text,
        Err(problem) if problem.kind() == ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };

    let mut entries = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<Entry>(line) {
            Ok(entry) => entries.push(entry),
            Err(problem) => warn!("skipped a damaged line of {}: {problem}", path.display()),
        }
    }
    let end = End {
        position: entries.last().map_or(0, |entry| entry.position),
        broken: !text.is_empty() && !text.ends_with('\n'),
    };
    Ok((entries, end))
}

#[cfg(test)]
mod tests {
    use std::fs::{read_to_string, write};

    use chrono::Utc;
    use lets_expect::lets_expect;
    use serde_json::json;
    use tempfile::{tempdir, TempDir};

    use std::path::PathBuf;

    use super::*;
    use crate::changes::Change;
    use crate::plans::PlanInfo;
    use crate::store::Status;
    use crate::workflow::Stage;

    fn thread() -> Thread {
        Thread {
            id: Uuid::new_v4(),
            project_id: Uuid::nil(),
            title: String::new(),
            named: false,
            agent: "claude".to_string(),
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
            put_away: false,
            done_seen: false,
            picks: BTreeMap::new(),
            ended: None,
            created_at: Utc::now(),
        }
    }

    /// A project directory with one thread in its store.
    struct Scene {
        directory: TempDir,
        project: Uuid,
        thread: Thread,
    }

    impl Scene {
        fn new() -> Self {
            let directory = tempdir().unwrap();
            let project = Uuid::new_v4();
            let mut store = ThreadStore::open(project, directory.path()).unwrap();
            let thread = store.save(thread()).unwrap();
            Self {
                directory,
                project,
                thread,
            }
        }

        /// Opens the store again, as Teryon does after a restart.
        fn reopen(&self) -> ThreadStore {
            ThreadStore::open(self.project, self.directory.path()).unwrap()
        }

        fn entries_file(&self) -> PathBuf {
            self.directory
                .path()
                .join(".teryon/threads")
                .join(self.thread.id.to_string())
                .join("entries.jsonl")
        }

        fn add(&self, text: &str) {
            let mut store = self.reopen();
            store
                .append(self.thread.id, "notice", json!({ "text": text }))
                .unwrap();
        }
    }

    fn words(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    fn texts(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| entry.body["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn positions(entries: &[Entry]) -> Vec<u64> {
        entries.iter().map(|entry| entry.position).collect()
    }

    /// The texts and positions of the scene's thread, read after a restart.
    fn read_back(scene: &Scene) -> (Vec<String>, Vec<u64>) {
        let entries = scene.reopen().entries(scene.thread.id).unwrap();
        (texts(&entries), positions(&entries))
    }

    /// Opens the scene's project a second time while it is still open.
    fn second_store(scene: &Scene) -> Option<String> {
        let _first = scene.reopen();
        ThreadStore::open(Uuid::new_v4(), scene.directory.path())
            .err()
            .map(|problem| problem.to_string())
    }

    /// Adds twenty entries to two threads, taking turns, and reads both back.
    fn interleaved(scene: &Scene) -> (Vec<u64>, Vec<u64>) {
        let mut store = scene.reopen();
        let other = store.save(thread()).unwrap();
        for number in 0..20 {
            let id = if number % 2 == 0 {
                scene.thread.id
            } else {
                other.id
            };
            store
                .append(id, "notice", json!({ "text": number.to_string() }))
                .unwrap();
        }
        (
            positions(&store.entries(scene.thread.id).unwrap()),
            positions(&store.entries(other.id).unwrap()),
        )
    }

    lets_expect! {
        expect(scene.reopen().threads()) as threads_read_back {
            let scene = Scene::new();

            to keep_the_thread { equal(vec![Thread { project_id: scene.project, ..scene.thread.clone() }]) }

            when a_thread_folder_is_damaged {
                before {
                    let damaged = scene.directory.path().join(".teryon/threads/broken");
                    create_dir_all(&damaged).unwrap();
                    write(damaged.join("thread.json"), "{ not json").unwrap();
                }
                to show_the_others { have(len()) equal(1) }
            }

            when the_project_was_added_again_under_a_new_identifier {
                let fresh = Uuid::new_v4();
                let threads = ThreadStore::open(fresh, scene.directory.path()).map(|store| store.threads());
                to follow_the_project { make(threads.unwrap()[0].project_id) equal(fresh) }
            }
        }

        expect(read_back(&scene)) as a_conversation_read_back {
            let scene = Scene::new();

            when nothing_was_said to be_empty { equal((Vec::<String>::new(), Vec::<u64>::new())) }

            when three_entries_were_added_across_restarts {
                before { scene.add("one"); scene.add("two"); scene.add("three"); }
                to keep_them_in_order_and_numbered { equal((words(&["one", "two", "three"]), vec![1, 2, 3])) }
            }

            when the_last_line_was_cut_off {
                before {
                    scene.add("one");
                    let mut text = read_to_string(scene.entries_file()).unwrap();
                    text.push_str("{\"id\":\"cut");
                    write(scene.entries_file(), text).unwrap();
                }
                to ignore_it { equal((words(&["one"]), vec![1])) }

                when another_entry_follows {
                    before { scene.add("two"); }
                    to start_it_on_a_line_of_its_own { equal((words(&["one", "two"]), vec![1, 2])) }
                }
            }
        }

        expect(second_store(&scene)) as a_second_store_for_the_same_project {
            let scene = Scene::new();
            to be_refused { equal(Some("Another Teryon is using this project.".to_string())) }
        }

        expect(read_to_string(scene.directory.path().join(".teryon/.gitignore")).unwrap()) as the_teryon_folder {
            let scene = Scene::new();
            to ignore_itself { equal("*\n".to_string()) }
        }

        expect(scene.reopen().append(Uuid::new_v4(), "notice", json!({})).err().map(|problem| problem.to_string())) as adding_to_an_unknown_thread {
            let scene = Scene::new();
            to be_refused { equal(Some("There is no such thread.".to_string())) }
        }

        expect(scene.reopen().change(scene.thread.id, |thread| thread.unread = true).map(|thread| thread.unread)) as changing_a_thread {
            let scene = Scene::new();
            to store_the_change { be_ok_and be_true }
        }

        expect(interleaved(&scene)) as two_threads_written_in_turns {
            let scene = Scene::new();
            to keep_their_own_numbering { equal(((1..=10).collect::<Vec<u64>>(), (1..=10).collect::<Vec<u64>>())) }
        }
    }

    /// Whether a thread with these properties is a quiet conversation.
    #[allow(clippy::too_many_arguments)]
    fn is_quiet(
        stage: Stage,
        running: bool,
        asking: bool,
        with_change: bool,
        with_plan: bool,
        put_away: bool,
        unread: bool,
        held: bool,
    ) -> bool {
        let mut thread = thread();
        thread.stage = stage;
        thread.status = if running {
            Status::Running
        } else {
            Status::Idle
        };
        thread.asking = asking;
        thread.change = with_change.then(|| {
            Box::new(Change {
                name: "fix".to_string(),
                path: PathBuf::from("/tmp/fix"),
                base: None,
                start: "abc".to_string(),
                branch: None,
                state: None,
                review: None,
            })
        });
        thread.plan = with_plan.then(|| PlanInfo {
            title: "Plan".to_string(),
            fingerprint: "abc".to_string(),
            too_large: false,
        });
        thread.put_away = put_away;
        thread.unread = unread;
        thread.held = held;
        thread.quiet()
    }

    /// Writes a stage and the two flags over the scene's thread and reads it back.
    ///
    /// `None` leaves a flag out of the file, as a thread stored before the
    /// board remembered finishes. The third value is whether reading left the
    /// file as it was written.
    fn read_flags(
        scene: &Scene,
        stage: &str,
        put_away: Option<bool>,
        done_seen: Option<bool>,
    ) -> (bool, bool, bool) {
        let path = scene
            .directory
            .path()
            .join(".teryon/threads")
            .join(scene.thread.id.to_string())
            .join("thread.json");
        let mut value: Value = serde_json::from_str(&read_to_string(&path).unwrap()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert("stage".to_string(), json!(stage));
        match put_away {
            Some(flag) => {
                object.insert("put_away".to_string(), json!(flag));
            }
            None => {
                object.remove("put_away");
            }
        }
        match done_seen {
            Some(flag) => {
                object.insert("done_seen".to_string(), json!(flag));
            }
            None => {
                object.remove("done_seen");
            }
        }
        let text = serde_json::to_string(&value).unwrap();
        write(&path, &text).unwrap();
        let read = scene.reopen().thread(scene.thread.id).unwrap().clone();
        let after = read_to_string(&path).unwrap();
        (read.put_away, read.done_seen, after == text)
    }

    lets_expect! {
        expect(is_quiet(stage, running, asking, with_change, with_plan, put_away, unread, held)) {
            let stage = Stage::Talking;
            let running = false;
            let asking = false;
            let with_change = false;
            let with_plan = false;
            let put_away = false;
            let unread = false;
            let held = false;

            to be_a_quiet_conversation { be_true }
            when(unread = true) as unread to still_be_quiet { be_true }
            when(held = true) as stopped to still_be_quiet { be_true }
            when(stage = Stage::Planning) as planning to stay_on_the_board { be_false }
            when(stage = Stage::Implementing) as implementing to stay_on_the_board { be_false }
            when(stage = Stage::Review) as review to stay_on_the_board { be_false }
            when(stage = Stage::Ready) as ready to stay_on_the_board { be_false }
            when(stage = Stage::Done) as done to stay_on_the_board { be_false }
            when(running = true) as running to stay_on_the_board { be_false }
            when(asking = true) as waiting_for_answers to stay_on_the_board { be_false }
            when(with_change = true) as holding_a_change to stay_on_the_board { be_false }
            when(with_plan = true) as holding_a_plan to stay_on_the_board { be_false }
            when(put_away = true) as already_put_away to not_be_quiet_again { be_false }
        }

        expect(read_flags(&scene, stage, put_away, done_seen)) {
            let scene = Scene::new();
            let put_away: Option<bool> = None;
            let done_seen: Option<bool> = None;

            when(stage = "done") as a_finished_thread_from_before to count_as_seen_without_rewriting {
                equal((false, true, true))
            }
            when(stage = "talking") as any_other_thread_from_before to count_as_still_on_the_board {
                equal((false, false, true))
            }
            when(stage = "planning") as a_thread_past_talking_from_before to count_as_not_seen {
                equal((false, false, true))
            }
            when(stage = "done", done_seen = Some(false)) as a_finish_stored_as_unseen to stay_unseen {
                equal((false, false, true))
            }
            when(stage = "talking", put_away = Some(true), done_seen = Some(true)) as flags_already_stored to keep_them {
                equal((true, true, true))
            }
        }
    }
}
