//! How much of each agent's subscription is used, and what Teryon's agents used it on.
//!
//! Agents run on subscriptions with usage limits, so what a person needs to
//! know before starting work is how much room each agent has left and when
//! it comes back, not what a turn cost. Each company reports that in its own
//! way, and some only inside their own program, so the limits are kept as a
//! general list of windows, whatever the program calls them.
//!
//! Teryon never asks a company's servers on its own. It learns the limits
//! when an agent runs: Claude Code reports them in every turn, Codex writes
//! them to its own session file, and Grok keeps them to its own window for
//! now. Beside the limits, every turn adds a line to a log of tokens read and
//! written, which the usage screen splits by day, project and model.
//!
//! Both live in the data folder: `limits.json` holds the latest reading per
//! agent, `usage.jsonl` one line per turn.

pub mod limits;
pub mod log;

use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{to_string_pretty, Error as JsonError, Value};
use thiserror::Error;
use tracing::warn;
use ts_rs::TS;
use uuid::Uuid;

use crate::agents::Agent;
use crate::store::{replace_file, StoreError};
use crate::usage::limits::{marked_reached, unknown_limits};

/// The file with the latest limits of every agent.
const LIMITS_FILE: &str = "limits.json";

/// Why usage could not be read or kept.
#[derive(Debug, Error)]
pub enum UsageError {
    #[error("could not read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not write {path}: {source}")]
    Unwritable {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("{path} is not valid: {source}")]
    Invalid {
        path: PathBuf,
        #[source]
        source: JsonError,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// One usage limit of a subscription, such as "5 hours" or "week".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Window {
    /// What the program calls it, such as `five_hour` or `primary`.
    pub name: String,
    /// What a person calls it, such as "5 hours".
    pub label: String,
    /// How long the window is, when known. Windows are shown shortest first.
    pub minutes: Option<u32>,
    /// How much of it is used, from 0 to 100.
    pub used: f64,
    /// When it starts over. Nothing when the program did not say.
    pub resets_at: Option<DateTime<Utc>>,
    /// Whether the program said it is used up, so the agent cannot work until it resets.
    pub reached: bool,
}

/// What an agent's program last said about its subscription's limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Limits {
    /// The agent's name, as on a thread.
    pub agent: String,
    /// The subscription, such as "Plus", when the program names it.
    pub plan: Option<String>,
    /// Shortest first.
    pub windows: Vec<Window>,
    /// When the program reported them. They are only as fresh as the agent's last turn.
    pub taken_at: DateTime<Utc>,
    /// Whether reading them after the last turn failed, so they may be older than that turn.
    #[serde(default)]
    pub failed: bool,
    /// The program's own answer, kept whole while Teryon cannot read it yet.
    ///
    /// Grok will say its limits outside its window one day, in a form not
    /// known today. Keeping what it says is what makes reading it possible later.
    #[serde(default)]
    pub raw: Option<Value>,
}

/// What reading an agent's limits after a turn gave.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    /// The program said what they are.
    Fresh(Limits),
    /// They should have been readable and were not. The reason is logged where it happened.
    Failed,
    /// The program said nothing about them this time.
    Nothing,
}

/// One agent turn, as the usage log keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UsageLine {
    /// When the turn ended.
    pub at: DateTime<Utc>,
    pub agent: String,
    /// The model that ran, when the program said. Nothing means the program's own default.
    pub model: Option<String>,
    pub project_id: Uuid,
    /// The project's name when the turn ran, for when the project has since been removed.
    pub project_name: String,
    pub thread_id: Uuid,
    pub read: u64,
    pub written: u64,
}

/// Everything the usage screen shows, as of when it was asked for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct UsageReport {
    /// Every agent's latest limits, in the order the application offers agents. Agents never read are left out.
    pub limits: Vec<Limits>,
    /// The turns of the last 14 days, oldest first.
    pub lines: Vec<UsageLine>,
}

/// The limits and the usage log of one data folder.
pub struct UsageBook {
    folder: PathBuf,
    limits: BTreeMap<String, Limits>,
}

impl UsageBook {
    /// Opens the usage kept in `folder`.
    ///
    /// Unreadable limits are logged and forgotten: the next turn of each
    /// agent reads them again, and the screen is the only thing that uses them.
    pub fn open(folder: &Path) -> Self {
        let limits = match read_limits(&folder.join(LIMITS_FILE)) {
            Ok(limits) => limits,
            Err(problem) => {
                warn!("forgot the agents' limits: {problem}");
                Vec::new()
            }
        };
        Self {
            folder: folder.to_path_buf(),
            limits: limits
                .into_iter()
                .map(|limits| (limits.agent.clone(), limits))
                .collect(),
        }
    }

    /// Every agent's latest limits, in the order the application offers agents.
    pub fn limits(&self) -> Vec<Limits> {
        let known = Agent::ALL.map(Agent::name);
        let mut limits: Vec<Limits> = self.limits.values().cloned().collect();
        limits.sort_by_key(|limits| {
            known
                .iter()
                .position(|name| *name == limits.agent)
                .unwrap_or(known.len())
        });
        limits
    }

    /// Takes in what a turn told about an agent's limits, and keeps it.
    ///
    /// Returns the agent's limits when they changed.
    pub fn note(
        &mut self,
        agent: Agent,
        reading: Reading,
        hit_limit: bool,
        now: DateTime<Utc>,
    ) -> Result<Option<Limits>, UsageError> {
        let next = next_limits(
            self.limits.get(agent.name()),
            agent,
            reading,
            hit_limit,
            now,
        );
        let Some(next) = next else {
            return Ok(None);
        };
        self.limits.insert(next.agent.clone(), next.clone());
        let text = to_string_pretty(&self.limits()).map_err(|source| UsageError::Invalid {
            path: self.folder.join(LIMITS_FILE),
            source,
        })?;
        replace_file(&self.folder.join(LIMITS_FILE), &text)?;
        Ok(Some(next))
    }

    /// Adds a turn to the usage log.
    pub fn record(&self, line: &UsageLine) -> Result<(), UsageError> {
        log::append(&self.folder, line)
    }

    /// Everything the usage screen shows.
    pub fn report(&self, now: DateTime<Utc>) -> Result<UsageReport, UsageError> {
        Ok(UsageReport {
            limits: self.limits(),
            lines: log::recent(&self.folder, now)?,
        })
    }
}

/// An agent's limits after a turn, from those before and what the turn told. Nothing when they did not change.
///
/// A turn that ended on a "usage limit" notice used a window up, even when
/// the program gave no reading that says so. The shortest window is the one
/// that runs out first, so it is marked.
fn next_limits(
    previous: Option<&Limits>,
    agent: Agent,
    reading: Reading,
    hit_limit: bool,
    now: DateTime<Utc>,
) -> Option<Limits> {
    let mark = |limits: Limits| {
        if hit_limit {
            marked_reached(limits)
        } else {
            limits
        }
    };
    match reading {
        Reading::Fresh(limits) => Some(mark(limits)),
        Reading::Failed => Some(Limits {
            failed: true,
            ..previous
                .cloned()
                .unwrap_or_else(|| unknown_limits(agent, now))
        }),
        Reading::Nothing => match previous {
            Some(previous) if hit_limit => {
                let marked = marked_reached(previous.clone());
                (marked != *previous).then_some(marked)
            }
            _ => None,
        },
    }
}

/// The limits kept in `path`. None when there is no such file yet.
fn read_limits(path: &Path) -> Result<Vec<Limits>, UsageError> {
    let text = match read_to_string(path) {
        Ok(text) => text,
        Err(problem) if problem.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(UsageError::Unreadable {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    serde_json::from_str(&text).map_err(|source| UsageError::Invalid {
        path: path.to_path_buf(),
        source,
    })
}

/// Whether a notice says the agent's usage limit was hit.
///
/// Claude Code's notice is Teryon's own, "The subscription's usage limit was
/// reached."; Codex's is its own, "You've hit your usage limit.".
pub fn says_limit_hit(text: &str) -> bool {
    text.to_lowercase().contains("usage limit")
}

#[cfg(test)]
mod tests {
    use std::fs::write;

    use chrono::TimeZone;
    use lets_expect::lets_expect;
    use tempfile::tempdir;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, hour, 0, 0).unwrap()
    }

    fn window(name: &str, minutes: Option<u32>, reached: bool) -> Window {
        Window {
            name: name.to_string(),
            label: name.to_string(),
            minutes,
            used: 40.0,
            resets_at: None,
            reached,
        }
    }

    fn limits(agent: &str, windows: Vec<Window>) -> Limits {
        Limits {
            agent: agent.to_string(),
            plan: None,
            windows,
            taken_at: at(10),
            failed: false,
            raw: None,
        }
    }

    fn claude() -> Limits {
        limits(
            "claude",
            vec![
                window("five_hour", Some(300), false),
                window("seven_day", Some(10080), false),
            ],
        )
    }

    fn reached_first(limits: Option<Limits>) -> Option<Vec<bool>> {
        limits.map(|limits| limits.windows.iter().map(|window| window.reached).collect())
    }

    /// The limits a book opens with, from a fresh folder whose `limits.json` holds `text` when given.
    fn opened_with(text: Option<&str>) -> Vec<Limits> {
        let folder = tempdir().unwrap();
        if let Some(text) = text {
            write(folder.path().join(LIMITS_FILE), text).unwrap();
        }
        UsageBook::open(folder.path()).limits()
    }

    /// The limits a book opens with after Grok's and then Claude's were noted.
    fn reopened() -> Vec<Limits> {
        let folder = tempdir().unwrap();
        let mut book = UsageBook::open(folder.path());
        book.note(
            Agent::Grok,
            Reading::Fresh(limits("grok", vec![])),
            false,
            at(12),
        )
        .unwrap();
        book.note(Agent::Claude, Reading::Fresh(claude()), false, at(12))
            .unwrap();
        UsageBook::open(folder.path()).limits()
    }

    lets_expect! {
        // requirement: a fresh reading replaces the last one, a failed one keeps it and says so, no reading changes nothing
        expect(next_limits(previous.as_ref(), Agent::Claude, reading.clone(), hit_limit, at(12))) as the_next_limits {
            let previous = Some(claude());
            let reading = Reading::Nothing;
            let hit_limit = false;

            when(reading = Reading::Fresh(limits("claude", vec![]))) as a_fresh_reading to replace_them {
                equal(Some(limits("claude", vec![])))
            }
            when(reading = Reading::Failed) as a_failed_reading to keep_them_marked_failed {
                equal(Some(Limits { failed: true, ..claude() }))
            }
            when(reading = Reading::Failed, previous = None) as a_failed_first_reading to give_empty_failed_limits {
                equal(Some(Limits { agent: "claude".to_string(), plan: None, windows: vec![], taken_at: at(12), failed: true, raw: None }))
            }
            when(reading = Reading::Nothing) as no_reading to change_nothing { equal(None) }
            when(reading = Reading::Nothing, previous = None, hit_limit = true) as a_hit_limit_without_anything_known to change_nothing { equal(None) }
        }

        // requirement: a "usage limit reached" notice marks the shortest window as reached when no window is
        expect(reached_first(next_limits(previous.as_ref(), Agent::Claude, reading.clone(), true, at(12)))) as limits_after_a_hit_limit {
            let previous = Some(claude());
            let reading = Reading::Nothing;

            when(reading = Reading::Nothing) as without_a_reading to mark_the_shortest_window { equal(Some(vec![true, false])) }
            when(reading = Reading::Fresh(claude())) as with_a_reading_saying_nothing_is_reached to mark_the_shortest_window { equal(Some(vec![true, false])) }
            when(reading = Reading::Fresh(limits("claude", vec![window("five_hour", Some(300), false), window("seven_day", Some(10080), true)]))) as with_a_reading_naming_the_window to keep_it {
                equal(Some(vec![false, true]))
            }
            when(previous = Some(limits("claude", vec![window("five_hour", Some(300), true)]))) as when_already_marked to change_nothing { equal(None) }
        }

        expect(says_limit_hit(text)) {
            when(text = "The subscription's usage limit was reached.") as claudes_notice to be_true
            when(text = "You've hit your usage limit.") as codexs_notice to be_true
            when(text = "Permission denied for Bash.") as another_notice to be_false
        }

        // requirement: the latest limits per agent are kept in limits.json, so the screen has them after a restart
        expect(reopened()) as limits_kept_across_restarts {
            to be_in_the_order_agents_are_offered { equal(vec![claude(), limits("grok", vec![])]) }
        }

        expect(opened_with(text)) as limits_read_from_the_data_folder {
            let text = None;

            when(text = None) as without_a_file to be_empty { equal(vec![]) }
            when(text = Some("not json")) as from_an_unreadable_file to be_forgotten { equal(vec![]) }
        }
    }
}
