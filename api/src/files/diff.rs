//! Lays a file out line by line, marking what changed.
//!
//! The file browser shows one list of lines for both of its views: the file
//! as it is, and the same file with its changes. Sending the lines once, each
//! marked as added, removed or untouched, lets the screen switch between the
//! two without asking again.

use std::time::Duration;

use serde::Serialize;
use similar::{ChangeTag, TextDiff};

/// How long comparing two versions may take before a rougher answer is accepted.
const PATIENCE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineChange {
    Added,
    Removed,
}

/// One line of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    /// Nothing for a line that is the same in both versions.
    pub change: Option<LineChange>,
    /// Where the line is in the file as it is now. A removed line has no place.
    pub number: Option<usize>,
    pub text: String,
}

/// The lines of `current`, with the lines only `base` has placed where they were.
pub fn compare(base: &str, current: &str) -> Vec<Line> {
    TextDiff::configure()
        .timeout(PATIENCE)
        .diff_lines(base, current)
        .iter_all_changes()
        .map(|change| Line {
            change: match change.tag() {
                ChangeTag::Equal => None,
                ChangeTag::Insert => Some(LineChange::Added),
                ChangeTag::Delete => Some(LineChange::Removed),
            },
            number: match change.tag() {
                ChangeTag::Delete => None,
                _ => change.new_index().map(|index| index + 1),
            },
            text: change.value().trim_end_matches(['\n', '\r']).to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn same(number: usize, text: &str) -> Line {
        Line {
            change: None,
            number: Some(number),
            text: text.to_string(),
        }
    }

    fn added(number: usize, text: &str) -> Line {
        Line {
            change: Some(LineChange::Added),
            number: Some(number),
            text: text.to_string(),
        }
    }

    fn removed(text: &str) -> Line {
        Line {
            change: Some(LineChange::Removed),
            number: None,
            text: text.to_string(),
        }
    }

    lets_expect! {
        expect(compare(base, current)) {
            when(base = "one\ntwo\n", current = "one\ntwo\n") as an_unchanged_file to number_every_line {
                equal(vec![same(1, "one"), same(2, "two")])
            }
            when(base = "", current = "one\ntwo\n") as a_new_file to mark_every_line_added {
                equal(vec![added(1, "one"), added(2, "two")])
            }
            when(base = "one\ntwo\n", current = "") as a_removed_file to mark_every_line_removed {
                equal(vec![removed("one"), removed("two")])
            }
            when(base = "one\ntwo\nthree\n", current = "one\nTWO\nthree\n") as a_changed_line to show_the_old_line_before_the_new {
                equal(vec![same(1, "one"), removed("two"), added(2, "TWO"), same(3, "three")])
            }
            when(base = "one\nthree\n", current = "one\ntwo\nthree\n") as an_inserted_line to shift_the_numbers_after_it {
                equal(vec![same(1, "one"), added(2, "two"), same(3, "three")])
            }
            when(base = "one\ntwo\nthree\n", current = "one\nthree\n") as a_dropped_line to keep_the_numbers_of_the_file_as_it_is {
                equal(vec![same(1, "one"), removed("two"), same(2, "three")])
            }
            when(base = "one\r\ntwo\r\n", current = "one\r\ntwo\r\n") as windows_line_endings to be_left_out_of_the_text {
                equal(vec![same(1, "one"), same(2, "two")])
            }
            when(base = "one", current = "one") as no_final_line_ending to still_be_one_line {
                equal(vec![same(1, "one")])
            }
            when(base = "one\n\ntwo\n", current = "one\n\ntwo\n") as an_empty_line to be_kept {
                equal(vec![same(1, "one"), same(2, ""), same(3, "two")])
            }
            when(base = "", current = "") as an_empty_file to have_no_lines {
                equal(Vec::new())
            }
        }
    }
}
