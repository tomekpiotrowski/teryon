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

use crate::store::Entry;

/// The kind of entry that starts a turn: a message from the person.
const TURN_START: &str = "user_message";

/// One stretch of an agent working in a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    pub thread_id: Uuid,
    /// In seconds since 1970, like the time of a commit.
    pub started: i64,
    /// Nothing while the agent is still working.
    pub ended: Option<i64>,
}

/// The turns of one thread, from its conversation in order.
///
/// A turn runs from a message from the person to the last entry before the
/// next one. Entries before the first message belong to no turn. The last
/// turn has no end while the thread's agent is still working.
pub fn turns_in(thread_id: Uuid, running: bool, entries: &[Entry]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for entry in entries {
        let time = entry.created_at.timestamp();
        if entry.kind == TURN_START {
            turns.push(Turn {
                thread_id,
                started: time,
                ended: Some(time),
            });
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

/// The thread whose agent was working at a moment, or nothing when none was.
///
/// With several agents working at once, the one that started last is named:
/// its work is the freshest, and a commit usually closes a piece of work.
pub fn working_at(turns: &[Turn], time: i64) -> Option<Uuid> {
    turns
        .iter()
        .filter(|turn| turn.started <= time && turn.ended.is_none_or(|ended| time <= ended))
        .max_by_key(|turn| turn.started)
        .map(|turn| turn.thread_id)
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn thread(number: u128) -> Uuid {
        Uuid::from_u128(number)
    }

    fn turn(number: u128, started: i64, ended: Option<i64>) -> Turn {
        Turn {
            thread_id: thread(number),
            started,
            ended,
        }
    }

    fn entry(kind: &str, time: i64) -> Entry {
        Entry {
            id: Uuid::new_v4(),
            thread_id: thread(1),
            position: 0,
            kind: kind.to_string(),
            body: serde_json::Value::Null,
            created_at: chrono::DateTime::from_timestamp(time, 0).unwrap(),
        }
    }

    lets_expect! {
        expect(turns_in(thread(1), running, &entries)) {
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
        }

        expect(working_at(&turns, time)) {
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
