//! The usage log: one line per agent turn, in `usage.jsonl` in the data folder.
//!
//! The usage screen splits the last 14 days by day, project and model.
//! Reading every thread of every project for that whenever the screen opens
//! would grow slow, so Teryon writes down what it needs as each turn ends.
//!
//! Lines are only ever added at the end. A line cut off by a power cut is
//! skipped when reading, and the next line starts on a fresh one.

use std::fs::{read, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde_json::{from_str, to_string};
use tracing::warn;

use crate::usage::{UsageError, UsageLine};

/// The file holding the log.
const LOG_FILE: &str = "usage.jsonl";

/// How far back the usage screen looks.
pub const DAYS_SHOWN: i64 = 14;

/// Adds a turn at the end of the log in `folder`.
pub fn append(folder: &Path, line: &UsageLine) -> Result<(), UsageError> {
    let path = folder.join(LOG_FILE);
    let unwritable = |source| UsageError::Unwritable {
        path: path.clone(),
        source,
    };
    let broken = match read(&path) {
        Ok(bytes) => bytes.last().is_some_and(|last| *last != b'\n'),
        Err(problem) if problem.kind() == ErrorKind::NotFound => false,
        Err(source) => {
            return Err(UsageError::Unreadable {
                path: path.clone(),
                source,
            })
        }
    };
    let text = to_string(line).map_err(|source| UsageError::Invalid {
        path: path.clone(),
        source,
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(unwritable)?;
    let start = if broken { "\n" } else { "" };
    file.write_all(format!("{start}{text}\n").as_bytes())
        .map_err(unwritable)
}

/// The turns of the last [`DAYS_SHOWN`] days before `now`, oldest first.
pub fn recent(folder: &Path, now: DateTime<Utc>) -> Result<Vec<UsageLine>, UsageError> {
    let path = folder.join(LOG_FILE);
    let bytes = match read(&path) {
        Ok(bytes) => bytes,
        Err(problem) if problem.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(UsageError::Unreadable { path, source }),
    };
    let since = now - Duration::days(DAYS_SHOWN);
    let mut lines: Vec<UsageLine> = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|text| !text.trim().is_empty())
        .filter_map(|text| match from_str::<UsageLine>(text) {
            Ok(line) => Some(line),
            Err(problem) => {
                warn!("skipped a line of the usage log that could not be read: {problem}");
                None
            }
        })
        .filter(|line| line.at >= since)
        .collect();
    lines.sort_by_key(|line| line.at);
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use std::fs::{read_to_string, write};

    use chrono::TimeZone;
    use lets_expect::lets_expect;
    use tempfile::{tempdir, TempDir};
    use uuid::Uuid;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap()
    }

    fn line(days_ago: i64) -> UsageLine {
        UsageLine {
            at: now() - Duration::days(days_ago),
            agent: "claude".to_string(),
            model: None,
            project_id: Uuid::nil(),
            project_name: "Teryon".to_string(),
            thread_id: Uuid::nil(),
            read: 100,
            written: 5,
        }
    }

    /// A data folder whose log starts with `text`, after which `added` are appended.
    fn folder_with(text: Option<&str>, added: &[UsageLine]) -> TempDir {
        let folder = tempdir().unwrap();
        if let Some(text) = text {
            write(folder.path().join(LOG_FILE), text).unwrap();
        }
        for line in added {
            append(folder.path(), line).unwrap();
        }
        folder
    }

    lets_expect! {
        // requirement: every turn adds a line, and the screen reads back the last 14 days
        expect(recent(folder.path(), now()).unwrap()) as reading_the_log {
            let folder = folder_with(text, &added);
            let text = None;
            let added = vec![line(1)];

            when(added = vec![]) as without_a_file to be_empty { equal(vec![]) }
            when(added = vec![line(1)]) as after_one_turn to give_it_back { equal(vec![line(1)]) }
            when(added = vec![line(1), line(3)]) as after_turns_out_of_order to give_them_oldest_first { equal(vec![line(3), line(1)]) }
            when(added = vec![line(15), line(14), line(2)]) as with_turns_older_than_14_days to leave_them_out { equal(vec![line(14), line(2)]) }
            when(text = Some("not json\n")) as with_a_broken_line to skip_it { equal(vec![line(1)]) }
            when(text = Some("{\"at\":")) as after_a_cut_off_line to start_on_a_new_line { equal(vec![line(1)]) }
        }

        expect(read_to_string(folder.path().join(LOG_FILE)).unwrap().lines().count()) as the_file_after_a_cut_off_line {
            let folder = folder_with(Some("{\"at\":"), &[line(1)]);
            to hold_both_lines_apart { equal(2) }
        }
    }
}
