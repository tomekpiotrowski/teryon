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

use chrono::NaiveDateTime;
use sea_orm::{DatabaseConnection, DbBackend, DbErr, FromQueryResult, Statement};
use uuid::Uuid;

use crate::agents::EntryKind;
use crate::workspace::thread::RUNNING;

/// The turns of every thread of a project.
///
/// A turn starts with a message from the person, so the entries of a thread
/// are numbered by how many such messages came before or with them. Takes
/// the project, the status of a thread whose agent is working, and the kind
/// of entry a person's message is.
const TURNS: &str = "
    SELECT turns.thread_id,
           MIN(turns.created_at) AS started,
           MAX(turns.created_at) AS ended,
           BOOL_OR(turns.running AND turns.turn = turns.last_turn) AS unfinished
    FROM (
        SELECT numbered.*,
               MAX(numbered.turn) OVER (PARTITION BY numbered.thread_id) AS last_turn
        FROM (
            SELECT entries.thread_id,
                   entries.created_at,
                   threads.status = $2 AS running,
                   COUNT(*) FILTER (WHERE entries.kind = $3)
                       OVER (PARTITION BY entries.thread_id ORDER BY entries.position) AS turn
            FROM thread_entries entries
            JOIN threads ON threads.id = entries.thread_id
            WHERE threads.project_id = $1
              AND threads.deleted_at IS NULL
              AND entries.deleted_at IS NULL
        ) numbered
    ) turns
    WHERE turns.turn > 0
    GROUP BY turns.thread_id, turns.turn";

/// One stretch of an agent working, as the database tells it.
#[derive(Debug, FromQueryResult)]
struct StoredTurn {
    thread_id: Uuid,
    started: NaiveDateTime,
    ended: NaiveDateTime,
    unfinished: bool,
}

/// One stretch of an agent working in a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    pub thread_id: Uuid,
    /// In seconds since 1970, like the time of a commit.
    pub started: i64,
    /// Nothing while the agent is still working.
    pub ended: Option<i64>,
}

/// Every turn agents have had in a project's threads.
pub async fn turns_of(db: &DatabaseConnection, project_id: Uuid) -> Result<Vec<Turn>, DbErr> {
    let statement = Statement::from_sql_and_values(
        DbBackend::Postgres,
        TURNS,
        [
            project_id.into(),
            RUNNING.into(),
            EntryKind::UserMessage.name().into(),
        ],
    );
    let stored = StoredTurn::find_by_statement(statement).all(db).await?;
    Ok(stored
        .into_iter()
        .map(|turn| Turn {
            thread_id: turn.thread_id,
            started: turn.started.and_utc().timestamp(),
            ended: (!turn.unfinished).then(|| turn.ended.and_utc().timestamp()),
        })
        .collect())
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

    lets_expect! {
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
