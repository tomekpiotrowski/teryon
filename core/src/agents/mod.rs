//! Running agent programs and understanding what they report.
//!
//! Teryon does not talk to any model provider. It starts the official `claude`
//! and `grok` programs, which use the sign-in already present on the machine,
//! and turns the lines they print into thread entries.
//!
//! Each program has a driver that knows two things: how to start a turn, and
//! how to read one line of the program's report. Everything else, including
//! processes and storage, lives in [`runner`].

pub mod claude;
pub mod grok;
pub mod process;
pub mod runner;
mod summary;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

use crate::agents::claude::ClaudeDriver;
use crate::agents::grok::GrokDriver;
use crate::settings::AgentSettings;

/// The agent programs Teryon can start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Grok,
}

impl Agent {
    /// Reads the name stored on a thread and sent by the application.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "grok" => Some(Self::Grok),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Grok => "grok",
        }
    }

    /// The name a person knows the agent by.
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Grok => "Grok",
        }
    }

    /// The agent that reviews this one's work: the other one, since two models catch more than one twice.
    pub fn other(self) -> Self {
        match self {
            Self::Claude => Self::Grok,
            Self::Grok => Self::Claude,
        }
    }
}

/// Which of a thread's two agents a turn belongs to.
///
/// A thread has the agent that does the work, and may have a second one
/// that reviews it, in a conversation of its own. Both write into the same
/// thread, one at a time, so that everything about one piece of work stays
/// in one place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Voice {
    /// The agent the thread was started with, which makes the change.
    #[default]
    Author,
    /// The agent that reviews the change and changes nothing.
    Reviewer,
}

/// What a thread entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    UserMessage,
    AgentMessage,
    Activity,
    Notice,
    Error,
    TurnSummary,
}

impl EntryKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::UserMessage => "user_message",
            Self::AgentMessage => "agent_message",
            Self::Activity => "activity",
            Self::Notice => "notice",
            Self::Error => "error",
            Self::TurnSummary => "turn_summary",
        }
    }
}

/// A thread entry that has not been stored yet.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryDraft {
    pub kind: EntryKind,
    pub body: Value,
}

impl EntryDraft {
    pub fn user_message(text: &str) -> Self {
        Self::text(EntryKind::UserMessage, text)
    }

    pub fn agent_message(text: &str) -> Self {
        Self::text(EntryKind::AgentMessage, text)
    }

    pub fn notice(text: &str) -> Self {
        Self::text(EntryKind::Notice, text)
    }

    pub fn error(text: &str) -> Self {
        Self::text(EntryKind::Error, text)
    }

    /// A tool the agent used, with a short description of what it was used on.
    pub fn activity(tool: &str, summary: &str) -> Self {
        Self {
            kind: EntryKind::Activity,
            body: json!({ "tool": tool, "summary": summary }),
        }
    }

    /// The end of a turn. The runner adds how long the turn took.
    pub fn turn_summary(cost_usd: Option<f64>, turns: Option<u64>) -> Self {
        Self {
            kind: EntryKind::TurnSummary,
            body: json!({ "cost_usd": cost_usd, "turns": turns }),
        }
    }

    /// Marks a turn's summary as the end of a turn the agent did not finish, such as one cut short by a refused command.
    pub fn ended_early(mut self) -> Self {
        self.body["early"] = json!(true);
        self
    }

    /// Whether this is the summary of a turn the agent did not finish.
    pub fn is_early_end(&self) -> bool {
        self.kind == EntryKind::TurnSummary && self.body["early"] == json!(true)
    }

    /// Marks the entry as the reviewer's, which the thread shows apart from the author's.
    pub fn by(mut self, voice: Voice) -> Self {
        if voice == Voice::Reviewer {
            self.body["reviewer"] = json!(true);
        }
        self
    }

    fn text(kind: EntryKind, text: &str) -> Self {
        Self {
            kind,
            body: json!({ "text": text }),
        }
    }
}

/// One message to hand to an agent program.
#[derive(Clone, Debug)]
pub struct Turn {
    pub message: String,
    pub conversation: Uuid,
    /// Whether this message starts the conversation.
    pub first: bool,
    /// What the agent is told about worktrees, beside the person's message.
    ///
    /// Nothing when the project is not a git repository. The programs take
    /// this as their own extra instruction, so it is not part of the message.
    pub brief: Option<String>,
    /// Whether the agent must leave files alone, as a reviewer must.
    ///
    /// Programs that can switch off their editing tools do; the brief asks
    /// the rest, and Teryon checks afterwards.
    pub read_only: bool,
}

/// How to start an agent program for one turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub arguments: Vec<String>,
    /// Environment variables the program must not inherit.
    pub removed_variables: Vec<&'static str>,
}

/// A line that is not part of the report format the driver knows.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("not a report line this program is known to print")]
pub struct NotUnderstood;

/// What Teryon needs to know about one agent program.
pub trait AgentDriver: Send {
    fn invocation(&self, turn: &Turn) -> Invocation;

    /// Turns one line of the program's report into thread entries.
    ///
    /// Most lines produce nothing. A driver may hold text back until it knows
    /// a message is complete.
    fn interpret(&mut self, line: &str) -> Result<Vec<EntryDraft>, NotUnderstood>;

    /// Hands over the slash commands the program said it takes, when the last line named them.
    ///
    /// Both programs name their commands at the start of a turn. The list
    /// depends on the project and on what the person has installed, so it
    /// is read from the program rather than kept in Teryon.
    fn take_commands(&mut self) -> Option<Vec<String>>;

    /// Returns whatever the driver was still holding when the program ended.
    fn finish(&mut self) -> Vec<EntryDraft> {
        Vec::new()
    }
}

pub fn driver_for(agent: Agent, settings: &AgentSettings) -> Box<dyn AgentDriver> {
    match agent {
        Agent::Claude => Box::new(ClaudeDriver::new(&settings.claude)),
        Agent::Grok => Box::new(GrokDriver::new(&settings.grok)),
    }
}

/// Reads a list of command names. Nothing when it is not a list; entries that are not text are left out.
fn command_names(list: &Value) -> Option<Vec<String>> {
    let names = list.as_array()?;
    Some(
        names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

/// Reads a report line as a JSON object and returns it with its `type`.
fn typed_report(line: &str) -> Result<(String, Value), NotUnderstood> {
    let report: Value = serde_json::from_str(line).map_err(|_| NotUnderstood)?;
    let kind = report
        .get("type")
        .and_then(Value::as_str)
        .ok_or(NotUnderstood)?
        .to_string();
    Ok((kind, report))
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        expect(Agent::parse(name)) {
            when(name = "claude") as be_claude to hold { equal(Some(Agent::Claude)) }
            when(name = "grok") as be_grok to hold { equal(Some(Agent::Grok)) }
            when(name = "Claude") as be_unknown_in_capitals to hold { equal(None) }
            when(name = "") as be_unknown_when_empty to hold { equal(None) }
            when(name = "gemini") as be_unknown to hold { equal(None) }
        }

        expect(Agent::Grok.label()) to be_capitalised { equal("Grok") }

        expect(EntryDraft::turn_summary(None, None).ended_early().is_early_end()) as an_early_summary to be_early { be_true }
        expect(EntryDraft::turn_summary(None, None).is_early_end()) as a_summary to be_on_time { be_false }
        expect(EntryDraft::notice("x").ended_early().is_early_end()) as another_entry to never_be_early { be_false }

        expect(Agent::Claude.other()) to be_grok { equal(Agent::Grok) }
        expect(Agent::Grok.other()) to be_claude { equal(Agent::Claude) }

        expect(EntryDraft::agent_message("hi").by(voice).body) {
            when(voice = Voice::Author) as the_author to be_left_unmarked { equal(json!({ "text": "hi" })) }
            when(voice = Voice::Reviewer) as the_reviewer to be_marked { equal(json!({ "text": "hi", "reviewer": true })) }
        }

        expect(Agent::parse(agent.name())) {
            when(agent = Agent::Claude) as read_back_claude to hold { equal(Some(Agent::Claude)) }
            when(agent = Agent::Grok) as read_back_grok to hold { equal(Some(Agent::Grok)) }
        }

        expect(typed_report(line).map(|(kind, _)| kind)) as the_type_of_a_report {
            when(line = r#"{"type":"text","data":"hi"}"#) as give_the_type to hold {
                equal(Ok("text".to_string()))
            }
            when(line = "not json") as refuse_text to hold { equal(Err(NotUnderstood)) }
            when(line = r#"{"data":"hi"}"#) as refuse_an_object_without_a_type to hold { equal(Err(NotUnderstood)) }
            when(line = r#"{"type":5}"#) as refuse_a_type_that_is_not_text to hold { equal(Err(NotUnderstood)) }
            when(line = "") as refuse_an_empty_line to hold { equal(Err(NotUnderstood)) }
        }
    }
}
