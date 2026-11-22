//! The messages between the local service and its clients.
//!
//! The desktop app, the terminal app and, through a Teryon server, the web
//! and mobile apps all speak this. It is written once, here, and the
//! TypeScript the Angular app uses is generated from it (see the test at the
//! bottom), so the two sides cannot drift apart.
//!
//! Every message is one JSON object with a `type`. A client first sends
//! [`ClientMessage::Hello`] with the service's token. After that it sends
//! requests, each with a number of its choosing, and the service answers each
//! with a [`ServiceMessage::Reply`] carrying the same number. Between replies
//! the service pushes [`Event`]s: first a snapshot of every project and
//! thread, then every change, and the new entries of the threads the client
//! follows.

use std::path::PathBuf;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::{Config, TS};
use uuid::Uuid;

use crate::agents::asking::{Choice, Choosing, Question, Questions};
use crate::agents::models::ModelLists;
use crate::agents::Voice;
use crate::changes::{Change, ChangeState, Finish, Fit, Review, ReviewState};
use crate::directory::DirectoryListing;
use crate::files::diff::{Line, LineChange};
use crate::files::{Content, FileChange, FileEntry, FileView, Listing};
use crate::history::{ChangedFile, Changes, Commit, History};
use crate::plans::PlanInfo;
use crate::settings::{
    Configuration, ConfiguredStage, ConfiguredWorkflow, StageProvider, Strength, StrengthModels,
};
use crate::store::{Entry, Project, Reviewer, Status, Thread, Verdict};
use crate::usage::{Limits, UsageLine, UsageReport, Window};
use crate::workflow::{Stage, StageLabel, Workflow, Workflows};

/// A message from a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// The first message: the token from `service.json`.
    Hello { token: String },
    /// Anything after that. The reply carries the same `id`.
    Call {
        #[ts(type = "number")]
        id: u64,
        request: Request,
    },
}

/// What a client can ask for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    AddProject {
        name: String,
        directory: String,
    },
    /// The directories inside a directory, for choosing where a project lives.
    ///
    /// Only from this machine: someone signed in elsewhere must not see this
    /// machine's folders. An empty path is the home directory. A path that is
    /// only partly typed lists the nearest directory that exists.
    Directories {
        path: String,
    },
    /// Forgets a project. Its directory, threads and worktrees stay.
    RemoveProject {
        project_id: Uuid,
    },
    StartThread {
        project_id: Uuid,
        /// Nothing lets the workflow's first stage choose, see `crate::workflow::picking`.
        #[serde(default)]
        agent: Option<String>,
        /// Nothing leaves the model to the agent program.
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        title: String,
    },
    /// Sends a message to the thread's agent, or to its reviewer.
    SendMessage {
        thread_id: Uuid,
        text: String,
        #[serde(default)]
        to: Voice,
    },
    /// Asks a second agent to review the thread's change, in the same thread.
    ReviewChange {
        thread_id: Uuid,
        /// The agent that reviews, which may not be the thread's own.
        agent: String,
        /// Nothing leaves the model to the agent program.
        #[serde(default)]
        model: Option<String>,
    },
    /// Switches the thread's agent, or its reviewer, to another agent or model, from the next message on.
    ChangeAgent {
        thread_id: Uuid,
        #[serde(default)]
        to: Voice,
        agent: String,
        /// Nothing leaves the model to the agent program.
        model: Option<String>,
    },
    Stop {
        thread_id: Uuid,
    },
    /// The person has looked at the thread, which clears its "needs attention" mark.
    MarkSeen {
        thread_id: Uuid,
    },
    /// Starts a change for a thread: sent by `teryon worktree create`, run by
    /// the thread's agent during `turn`. Answers with where the copy is.
    StartChange {
        thread_id: Uuid,
        turn: Uuid,
        name: String,
        from: Option<String>,
    },
    /// Gives a thread the title its agent chose: sent by `teryon thread title`,
    /// run by the thread's agent during `turn`.
    NameThread {
        thread_id: Uuid,
        turn: Uuid,
        title: String,
    },
    /// Moves a thread to another stage: sent by `teryon thread stage`, run by the
    /// thread's own agent during `turn`. The person never sets a stage directly.
    ReportStage {
        thread_id: Uuid,
        turn: Uuid,
        stage: String,
    },
    /// Puts a thread on another workflow: sent by `teryon thread workflow` during
    /// `turn`, or by the person without one.
    ChooseWorkflow {
        thread_id: Uuid,
        #[serde(default)]
        turn: Option<Uuid>,
        workflow: String,
    },
    /// Asks the person questions: sent by `teryon thread ask`, run by the
    /// thread's own agent during `turn`. `asked` is the JSON the agent wrote,
    /// read by the service so every refusal is in its log.
    AskQuestions {
        thread_id: Uuid,
        turn: Uuid,
        asked: String,
    },
    /// A starting point for the description of a thread's change, for the person to edit.
    DraftDescription {
        thread_id: Uuid,
    },
    /// Applies a thread's change to the project as one entry in its history, and forgets it.
    ApplyChange {
        thread_id: Uuid,
        description: String,
    },
    /// Brings a thread's change up to date with newer work in the project, unless they overlap.
    CatchUp {
        thread_id: Uuid,
    },
    /// Throws a thread's change away, stopping its agent first.
    DiscardChange {
        thread_id: Uuid,
    },
    /// Sends a thread's change for review on GitHub, described by `description`.
    SendForReview {
        thread_id: Uuid,
        description: String,
    },
    /// Opens a thread's review on GitHub in the browser. Only from this machine.
    OpenReview {
        thread_id: Uuid,
    },
    /// Opens a thread's copy in another program. Only from this machine.
    OpenCopy {
        thread_id: Uuid,
        with: Opener,
    },
    /// Answers with the thread's entries so far; new ones follow as events.
    FollowThread {
        thread_id: Uuid,
    },
    UnfollowThread {
        thread_id: Uuid,
    },
    /// Every file of a thread's copy, a branch or a commit. Without any, the project's directory as it is.
    ///
    /// A commit wins over a thread's copy, and a copy over a branch.
    Files {
        project_id: Uuid,
        thread_id: Option<Uuid>,
        branch: Option<String>,
        commit: Option<String>,
    },
    File {
        project_id: Uuid,
        thread_id: Option<Uuid>,
        branch: Option<String>,
        commit: Option<String>,
        path: String,
    },
    /// The commits of a branch. Without one, of whatever the project's directory is on.
    History {
        project_id: Uuid,
        branch: Option<String>,
    },
    /// What a commit changed, or with `from`, what changed between two commits.
    Changes {
        project_id: Uuid,
        commit: String,
        from: Option<String>,
    },
    Change {
        project_id: Uuid,
        commit: String,
        from: Option<String>,
        path: String,
    },
    /// Every agent's latest limits and the turns of the last 14 days, for the usage screen.
    Usage,
    /// The text of a thread's plan, see `crate::plans`.
    Plan {
        thread_id: Uuid,
    },
    /// The choices the settings screen shows. Only from this machine.
    Configuration,
    /// Replaces those choices. Only from this machine. A running agent is left as it is.
    SaveConfiguration {
        configuration: Configuration,
    },
    /// Stops the service, after stopping every agent. Only from this machine.
    Shutdown,
}

/// A message from the service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceMessage {
    /// The answer to the call with the same `id`.
    Reply {
        #[ts(type = "number")]
        id: u64,
        outcome: Outcome,
    },
    /// Boxed, because an event can carry a whole thread, much larger than a reply.
    Event { event: Box<Event> },
}

/// How a call went.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    Done {
        answer: Answer,
    },
    /// Refused or failed; the message is meant for the person.
    Failed {
        message: String,
    },
}

/// What a call gives back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// Nothing to give back beyond having done it.
    Nothing,
    Project {
        project: ProjectView,
    },
    /// The directories inside one directory.
    Directories {
        listing: DirectoryListing,
    },
    /// Boxed, because a thread is several times larger than any other answer.
    Thread {
        thread: Box<Thread>,
    },
    Entry {
        entry: Entry,
    },
    Entries {
        entries: Vec<Entry>,
    },
    Listing {
        listing: Listing,
    },
    File {
        file: FileView,
    },
    History {
        history: History,
    },
    Changes {
        changes: Changes,
    },
    /// Where something is on this machine.
    Path {
        path: String,
    },
    /// A text for the person to edit.
    Draft {
        text: String,
    },
    /// How catching up went: done, or left as it was because the change overlaps with newer work.
    CaughtUp {
        overlaps: bool,
    },
    Usage {
        report: UsageReport,
    },
    /// A thread's plan, as markdown.
    Plan {
        text: String,
    },
    /// The choices the settings screen shows, after reading or saving them.
    Configuration {
        configuration: Configuration,
    },
}

/// What a thread's copy can be opened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Opener {
    /// The system's file manager.
    Folder,
    /// The editor set up in `config.toml`.
    Editor,
    /// The terminal set up in `config.toml`.
    Terminal,
}

/// Which of the openers that need setting up are set up on this machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Opens {
    pub editor: bool,
    pub terminal: bool,
}

/// Something that happened, pushed to clients as it happens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Every project and thread. Sent first, and again when a client fell behind.
    Snapshot {
        projects: Vec<ProjectView>,
        threads: Vec<Thread>,
        opens: Opens,
        /// The models each agent offers besides its default.
        models: ModelLists,
        /// The workflows threads can follow, and what each stage is called.
        workflows: Workflows,
    },
    /// The models on offer changed, as when Grok has listed its own after the service started.
    ModelsChanged {
        models: ModelLists,
    },
    ProjectAdded {
        project: ProjectView,
    },
    ProjectRemoved {
        project_id: Uuid,
    },
    ThreadChanged {
        thread: Thread,
    },
    /// A new entry of a thread the client follows.
    EntryAdded {
        entry: Entry,
    },
    /// An agent's program said something new about its limits.
    LimitsChanged {
        limits: Limits,
    },
    /// A turn ended and was added to the usage log.
    UsageRecorded {
        line: UsageLine,
    },
    /// The settings screen saved. Clients use the new workflows, labels and openers without asking again.
    SettingsChanged {
        opens: Opens,
        workflows: Workflows,
    },
}

/// A project as clients see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProjectView {
    pub id: Uuid,
    pub name: String,
    #[ts(type = "string")]
    pub directory: PathBuf,
    pub created_at: DateTime<Utc>,
    pub state: ProjectState,
    /// Whether its `origin` is on GitHub, so changes can be sent for review there.
    pub on_github: bool,
}

impl ProjectView {
    pub fn new(project: &Project, state: ProjectState, on_github: bool) -> Self {
        Self {
            id: project.id,
            name: project.name.clone(),
            directory: project.directory.clone(),
            created_at: project.created_at,
            state,
            on_github,
        }
    }
}

/// Whether a project can be worked in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    Ready,
    /// Its directory has gone. It can only be removed.
    Missing,
    /// Another Teryon service has it open.
    InUse,
}

/// The TypeScript for every type a message can hold, in one file.
///
/// The Angular app uses it, and its fingerprint tells two versions of
/// Teryon apart (see [`fingerprint`]).
pub fn typescript() -> String {
    let config = Config::new().with_large_int("number");
    let declarations = [
        ClientMessage::decl(&config),
        Request::decl(&config),
        ServiceMessage::decl(&config),
        Outcome::decl(&config),
        Answer::decl(&config),
        Event::decl(&config),
        ProjectView::decl(&config),
        ProjectState::decl(&config),
        DirectoryListing::decl(&config),
        Thread::decl(&config),
        PlanInfo::decl(&config),
        Status::decl(&config),
        Stage::decl(&config),
        Workflows::decl(&config),
        Workflow::decl(&config),
        StageLabel::decl(&config),
        Change::decl(&config),
        ChangeState::decl(&config),
        Review::decl(&config),
        ReviewState::decl(&config),
        Finish::decl(&config),
        Reviewer::decl(&config),
        Verdict::decl(&config),
        Voice::decl(&config),
        Fit::decl(&config),
        Opener::decl(&config),
        Opens::decl(&config),
        ModelLists::decl(&config),
        Entry::decl(&config),
        Questions::decl(&config),
        Question::decl(&config),
        Choice::decl(&config),
        Choosing::decl(&config),
        Listing::decl(&config),
        FileEntry::decl(&config),
        FileChange::decl(&config),
        FileView::decl(&config),
        Content::decl(&config),
        Line::decl(&config),
        LineChange::decl(&config),
        History::decl(&config),
        Commit::decl(&config),
        Changes::decl(&config),
        ChangedFile::decl(&config),
        UsageReport::decl(&config),
        Limits::decl(&config),
        Window::decl(&config),
        UsageLine::decl(&config),
        Configuration::decl(&config),
        ConfiguredWorkflow::decl(&config),
        ConfiguredStage::decl(&config),
        StageProvider::decl(&config),
        StrengthModels::decl(&config),
        Strength::decl(&config),
    ];
    let mut text = String::from(
        "// Generated from core/src/protocol.rs by `UPDATE_PROTOCOL=1 cargo test -p teryon-core protocol`.\n\
         // Do not edit by hand.\n\n\
         export type JsonValue = number | string | boolean | null | JsonValue[] | { [key: string]: JsonValue };\n",
    );
    for declaration in declarations {
        text.push_str(&format!("\nexport {declaration}\n"));
    }
    text
}

/// A short text that differs whenever the messages here change.
///
/// A client compares it with the one the service wrote to `service.json`
/// before talking to it. Without it, a desktop app built after the running
/// service would not understand the service's messages and could only wait,
/// with nothing on screen saying why.
///
/// It is the start of the SHA-256 of [`typescript`], which covers every
/// field of every message, so a change to them changes it, and a change
/// elsewhere in Teryon does not force a restart that stops running agents.
pub fn fingerprint() -> &'static str {
    static FINGERPRINT: LazyLock<String> = LazyLock::new(|| {
        Sha256::digest(typescript().as_bytes())[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    });
    &FINGERPRINT
}

#[cfg(test)]
mod tests {
    use std::env::var_os;
    use std::fs::{read_to_string, write};
    use std::path::Path;

    use lets_expect::lets_expect;
    use serde_json::{from_str, json, to_value};

    use super::*;

    /// Where the Angular app keeps the generated types.
    const TYPESCRIPT: &str = "../app/src/app/service/protocol.ts";

    /// Whether the generated TypeScript in the app matches the Rust here.
    ///
    /// With `UPDATE_PROTOCOL` set, writes it instead.
    fn typescript_is_current() -> bool {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TYPESCRIPT);
        let fresh = typescript();
        if var_os("UPDATE_PROTOCOL").is_some() {
            write(&path, &fresh).expect("the generated TypeScript could not be written");
        }
        read_to_string(&path).is_ok_and(|current| current == fresh)
    }

    fn lowercase_hexadecimal(text: &str) -> bool {
        text.chars()
            .all(|digit| digit.is_ascii_digit() || ('a'..='f').contains(&digit))
    }

    fn hello() -> ClientMessage {
        ClientMessage::Hello {
            token: "abc".to_string(),
        }
    }

    fn stop_call() -> ClientMessage {
        ClientMessage::Call {
            id: 7,
            request: Request::Stop {
                thread_id: Uuid::nil(),
            },
        }
    }

    const UNTITLED_THREAD: &str = r#"{"type":"call","id":1,"request":{"type":"start_thread","project_id":"00000000-0000-0000-0000-000000000000","agent":"claude"}}"#;
    const MESSAGE_WITHOUT_TEXT: &str = r#"{"type":"call","id":1,"request":{"type":"send_message","thread_id":"00000000-0000-0000-0000-000000000000"}}"#;
    const UNKNOWN_REQUEST: &str = r#"{"type":"call","id":1,"request":{"type":"dance"}}"#;
    const CALL_WITHOUT_NUMBER: &str = r#"{"type":"call","request":{"type":"shutdown"}}"#;

    lets_expect! {
        expect(typescript_is_current()) as the_generated_typescript {
            to be_current { be_true }
        }

        expect(fingerprint().len()) as the_fingerprint_length to equal(16)
        expect(lowercase_hexadecimal(fingerprint())) as the_fingerprint_digits to be_true

        expect(to_value(&message).unwrap()) as the_json_of_a_message {
            when(message = hello()) as a_hello {
                to name_its_type { equal(json!({ "type": "hello", "token": "abc" })) }
            }
            when(message = stop_call()) as a_call {
                to nest_the_request {
                    equal(json!({ "type": "call", "id": 7, "request": { "type": "stop", "thread_id": Uuid::nil() } }))
                }
            }
        }

        expect(from_str::<ClientMessage>(text).is_ok()) as reading_a_message {
            when(text = UNTITLED_THREAD) as a_thread_without_a_title {
                to be_read { be_true }
            }
            when(text = MESSAGE_WITHOUT_TEXT) as a_message_without_text {
                to be_refused { be_false }
            }
            when(text = UNKNOWN_REQUEST) as an_unknown_request to be_refused { be_false }
            when(text = CALL_WITHOUT_NUMBER) as a_call_without_a_number to be_refused { be_false }
            when(text = "not json") as text_that_is_not_json to be_refused { be_false }
        }
    }
}
