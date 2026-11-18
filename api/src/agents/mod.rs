//! Running agent programs and understanding what they report.
//!
//! Teryon does not talk to any model provider. It starts the official `claude`
//! and `grok` programs, which use the sign-in already present on the machine,
//! and turns the lines they print into thread entries.
//!
//! Each program has a driver that knows two things: how to start a turn, and
//! how to read one line of the program's report. Everything else, including
//! processes and the database, lives in [`runner`].

pub mod claude;
pub mod grok;
pub mod runner;
mod summary;

use serde_json::{json, Value};
use thiserror::Error;
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
