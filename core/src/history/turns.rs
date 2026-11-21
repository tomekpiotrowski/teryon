//! Tells which commits were made while an agent was working.
//!
//! The history screen sets apart what agents did from what people did, and
//! leads from a commit back to the thread it came from. Git records neither:
//! an agent commits under the name of the person whose machine it runs on.
//! What Teryon does know is when each agent was working, because every
//! message a person sends and everything the agent reports afterwards is
//! stored with its time.
//!
//! So a commit counts as an agent's when it came to be during one of its
//! turns: from a message to the last thing reported after it, or until now
//! for a turn still going. This is a judgement by time and can be wrong: a
//! commit a person makes by hand in the same repository while an agent is
//! working is taken for the agent's.

use uuid::Uuid;

use crate::agents::EntryKind;
use crate::store::Entry;

/// One stretch of an agent working in a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub thread_id: Uuid,
    /// The agent that worked, as it was then: a thread can move to another agent, or be reviewed by one.
    pub agent: String,
    /// In seconds since 1970, like the time of a commit.
    pub started: i64,
    /// Nothing while the agent is still working.
    pub ended: Option<i64>,
}

/// The turns of one thread, from its conversation in order.
///
/// A turn starts with a message from the person, or with a review asked
/// for, and runs to the last entry before the next start. Entries before
/// the first start belong to no turn, and so does a note that another agent
/// takes over: it is written while nobody works, and counting it would
/// stretch the turn before it over whatever a person did in between. The
/// last turn has no end while the thread is running.
///
/// A turn's agent is the one its first entry names. Messages stored before
/// they named one were all sent to `agent`, the thread's agent, since a
/// thread could not change agents then.
pub fn turns_in(thread_id: Uuid, agent: &str, running: bool, entries: &[Entry]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for entry in entries {
        let time = entry.created_at.timestamp();
        if starts_turn(entry) {
            turns.push(Turn {
                thread_id,
                agent: entry.body["agent"].as_str().unwrap_or(agent).to_string(),
                started: time,
                ended: Some(time),
            });
        } else if entry.body["takes_over"] == true {
            continue;
        } else if let Some(turn) = turns.last_mut() {
            turn.ended = Some(time);
        }
    }
    if running {
        if let Some(turn) = turns.last_mut() {
            turn.ended = None;
        }
    }
    turns
}

/// Whether an entry starts an agent working: a message from the person, or a review asked for.
fn starts_turn(entry: &Entry) -> bool {
    entry.kind == EntryKind::UserMessage.name() || entry.body["starts_review"] == true
}

/// The turn under way at a moment, or nothing when no agent was working.
///
/// With several agents working at once, the one that started last is named:
/// its work is the freshest, and a commit usually closes a piece of work.
pub fn working_at(turns: &[Turn], time: i64) -> Option<&Turn> {
    turns
        .iter()
        .filter(|turn| turn.started <= time && turn.ended.is_none_or(|ended| time <= ended))
        .max_by_key(|turn| turn.started)
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use lets_expect::lets_expect;
    use serde_json::{json, Value};

    use super::*;

    fn thread(number: u128) -> Uuid {
        Uuid::from_u128(number)
    }

    /// The thread of the turn under way at `time`.
    fn thread_at(turns: &[Turn], time: i64) -> Option<Uuid> {
        working_at(turns, time).map(|turn| turn.thread_id)
    }

    /// A turn of thread `number` by Claude.
    fn turn(number: u128, started: i64, ended: Option<i64>) -> Turn {
        Turn {
            thread_id: thread(number),
            agent: "claude".to_string(),
            started,
            ended,
        }
    }

    /// A turn of thread 1 by `agent`.
    fn by(agent: &str, started: i64, ended: Option<i64>) -> Turn {
        Turn {
            agent: agent.to_string(),
            ..turn(1, started, ended)
        }
    }

    fn entry(kind: &str, time: i64) -> Entry {
        with(kind, time, Value::Null)
    }

    fn with(kind: &str, time: i64, body: Value) -> Entry {
        Entry {
            id: Uuid::new_v4(),
            thread_id: thread(1),
            position: 0,
            kind: kind.to_string(),
            body,
            created_at: DateTime::from_timestamp(time, 0).unwrap(),
        }
    }

    lets_expect! {
        expect(turns_in(thread(1), "claude", running, &entries)) {
            let running = false;

            when(entries = Vec::new()) as an_empty_conversation to have_no_turns { equal(Vec::new()) }
            when(entries = vec![entry("notice", 10)]) as entries_before_any_message to belong_to_no_turn { equal(Vec::new()) }
            when(entries = vec![entry("user_message", 10)]) as a_message_alone to be_a_turn_of_a_moment {
                equal(vec![turn(1, 10, Some(10))])
            }
            when(entries = vec![entry("user_message", 10), entry("agent_message", 20), entry("turn_summary", 30)]) as one_answered_message {
                to end_at_its_last_entry { equal(vec![turn(1, 10, Some(30))]) }
            }
            when(entries = vec![entry("user_message", 10), entry("turn_summary", 20), entry("user_message", 50), entry("turn_summary", 60)]) as two_messages {
                to make_two_turns { equal(vec![turn(1, 10, Some(20)), turn(1, 50, Some(60))]) }

                when(running = true) as the_agent_is_still_working {
                    to leave_only_the_last_open { equal(vec![turn(1, 10, Some(20)), turn(1, 50, None)]) }
                }
            }
            // requirement: a commit a person makes after a turn is not the agent's because another agent took over later
            when(entries = vec![
                entry("user_message", 10),
                entry("turn_summary", 20),
                with("notice", 50, json!({ "text": "Codex takes over from Claude.", "takes_over": true, "agent": "codex" })),
            ]) as a_takeover_after_a_turn {
                to leave_the_turn_where_it_ended { equal(vec![turn(1, 10, Some(20))]) }
            }
            // requirement: each turn names the agent that worked then, so earlier work stays the earlier agent's
            when(entries = vec![
                with("user_message", 10, json!({ "agent": "claude" })),
                entry("turn_summary", 20),
                with("notice", 30, json!({ "takes_over": true, "agent": "codex" })),
                with("user_message", 40, json!({ "agent": "codex" })),
                entry("turn_summary", 50),
            ]) as a_thread_that_moved_to_another_agent {
                to name_each_turns_agent { equal(vec![by("claude", 10, Some(20)), by("codex", 40, Some(50))]) }
            }
            when(entries = vec![entry("user_message", 10), entry("turn_summary", 20)]) as messages_stored_before_naming_an_agent {
                to belong_to_the_threads_agent { equal(vec![turn(1, 10, Some(20))]) }
            }
            // requirement: a review asked for starts a turn of the reviewer, rather than stretching the author's
            when(entries = vec![
                entry("user_message", 10),
                entry("turn_summary", 20),
                with("notice", 50, json!({ "starts_review": true, "reviewer": true, "agent": "grok" })),
                with("agent_message", 60, json!({ "reviewer": true, "agent": "grok" })),
            ]) as a_review_after_a_turn {
                to make_a_turn_of_the_reviewer { equal(vec![turn(1, 10, Some(20)), by("grok", 50, Some(60))]) }
            }
        }

        expect(thread_at(&turns, time)) {
            let turns = vec![turn(1, 100, Some(200)), turn(2, 300, Some(400))];

            when(time = 150) as a_moment_inside_a_turn to name_its_thread { equal(Some(thread(1))) }
            when(time = 100) as the_moment_a_turn_started to name_its_thread { equal(Some(thread(1))) }
            when(time = 200) as the_moment_a_turn_ended to name_its_thread { equal(Some(thread(1))) }
            when(time = 99) as a_moment_before_any_turn to name_nobody { equal(None) }
            when(time = 250) as a_moment_between_two_turns to name_nobody { equal(None) }
            when(time = 401) as a_moment_after_every_turn to name_nobody { equal(None) }
            when(time = 350) as a_moment_inside_a_later_turn to name_that_thread { equal(Some(thread(2))) }

            when a_turn_is_still_going {
                let turns = vec![turn(1, 100, Some(200)), turn(2, 300, None)];

                when(time = 5000) as any_moment_after_it_started to name_its_thread { equal(Some(thread(2))) }
                when(time = 299) as a_moment_before_it_started to name_nobody { equal(None) }
            }

            when two_agents_were_working_at_once {
                let turns = vec![turn(1, 100, Some(400)), turn(2, 200, Some(300))];

                when(time = 250) as a_moment_inside_both to name_the_one_that_started_last { equal(Some(thread(2))) }
                when(time = 350) as a_moment_inside_the_longer_only to name_the_longer { equal(Some(thread(1))) }
            }

            when there_are_no_turns {
                let turns = Vec::new();

                when(time = 150) as any_moment to name_nobody { equal(None) }
            }
        }
    }
}
