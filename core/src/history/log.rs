//! Reads what git prints about commits.
//!
//! Everything is asked for with `-z`, where each part ends in a zero byte,
//! because that is the only form in which git prints every file name and
//! every message untouched.

use std::collections::BTreeMap;

use crate::files::git::zero_separated;
use crate::files::FileChange;
use crate::history::ChangedFile;

/// How `git log` is asked to print a commit, for `logged` to read.
///
/// The name of the commit comes after a `/`. A file is never printed with a
/// `/` in front, so the mark tells where one commit's files end and the next
/// commit begins. Five parts follow: the parents, the author's name and
/// address, the time and the first line of the message.
pub const LOG_FORMAT: &str = "--format=tformat:/%H%x00%P%x00%an%x00%ae%x00%ct%x00%s";

/// How many parts of a commit follow its name in `LOG_FORMAT`.
const DETAILS: usize = 5;

/// A commit as `git log` printed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logged {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub subject: String,
    pub files: Vec<String>,
}

/// Reads `git log -z --name-only` printed with `LOG_FORMAT`.
///
/// After a commit's details git prints the files it changed, the first of
/// them after a line break. A commit that changed nothing is followed by the
/// next commit straight away. A commit cut short, which git never prints, is
/// left out.
pub fn logged(printed: &[u8]) -> Vec<Logged> {
    let parts: Vec<&[u8]> = printed.split(|byte| *byte == 0).collect();
    let mut commits: Vec<Logged> = Vec::new();
    let mut at = 0;

    while at < parts.len() {
        let part = parts[at];
        at += 1;
        if let Some(hash) = part.strip_prefix(b"/") {
            let Some(details) = parts.get(at..at + DETAILS) else {
                break;
            };
            at += DETAILS;
            let text = |part: &[u8]| String::from_utf8_lossy(part).into_owned();
            commits.push(Logged {
                hash: text(hash),
                parents: text(details[0])
                    .split_whitespace()
                    .map(String::from)
                    .collect(),
                author: text(details[1]),
                email: text(details[2]),
                time: text(details[3]).trim().parse().unwrap_or_default(),
                subject: text(details[4]),
                files: Vec::new(),
            });
        } else if let Some(commit) = commits.last_mut() {
            let first = commit.files.is_empty();
            let name = match part.strip_prefix(b"\n") {
                Some(name) if first => name,
                _ => part,
            };
            if !name.is_empty() {
                commit
                    .files
                    .push(String::from_utf8_lossy(name).into_owned());
            }
        }
    }
    commits
}

/// Reads `git diff --numstat -z`: lines added, lines removed, then the file.
///
/// Renames are switched off where this is asked for, so every count is
/// followed by exactly one file. A file that is not text has `-` for both
/// counts and is left out.
pub fn counts(printed: &[u8]) -> BTreeMap<String, (u32, u32)> {
    zero_separated(printed)
        .into_iter()
        .filter_map(|part| {
            let mut pieces = part.splitn(3, '\t');
            let added = pieces.next()?.parse().ok()?;
            let removed = pieces.next()?.parse().ok()?;
            Some((pieces.next()?.to_string(), (added, removed)))
        })
        .collect()
}

/// Puts the files a commit changed together with their counts, sorted by path.
pub fn changed_files(
    changed: Vec<(String, FileChange)>,
    counts: &BTreeMap<String, (u32, u32)>,
) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = changed
        .into_iter()
        .map(|(path, change)| {
            let count = counts.get(&path);
            ChangedFile {
                added: count.map(|(added, _)| *added),
                removed: count.map(|(_, removed)| *removed),
                path,
                change,
            }
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn commit(hash: &str, parents: &[&str], subject: &str, files: &[&str]) -> Logged {
        Logged {
            hash: hash.to_string(),
            parents: parents.iter().map(|parent| parent.to_string()).collect(),
            author: "Ada".to_string(),
            email: "ada@example.com".to_string(),
            time: 1700000000,
            subject: subject.to_string(),
            files: files.iter().map(|file| file.to_string()).collect(),
        }
    }

    fn count(path: &str, added: u32, removed: u32) -> (String, (u32, u32)) {
        (path.to_string(), (added, removed))
    }

    fn file(
        path: &str,
        change: FileChange,
        added: Option<u32>,
        removed: Option<u32>,
    ) -> ChangedFile {
        ChangedFile {
            path: path.to_string(),
            change,
            added,
            removed,
        }
    }

    fn counted(counts: &[(String, (u32, u32))]) -> BTreeMap<String, (u32, u32)> {
        counts.iter().cloned().collect()
    }

    lets_expect! {
        expect(logged(printed)) {
            when(printed = "/abc\0def\0Ada\0ada@example.com\x001700000000\0Add the adapter\0\nsrc/a.txt\0b c.txt\0".as_bytes())
                as one_commit to read_its_details_and_files {
                equal(vec![commit("abc", &["def"], "Add the adapter", &["src/a.txt", "b c.txt"])])
            }
            when(printed = "/abc\0def\0Ada\0ada@example.com\x001700000000\0Second\0\na.txt\0/def\0\0Ada\0ada@example.com\x001700000000\0First\0\nb.txt\0".as_bytes())
                as two_commits to keep_their_files_apart {
                equal(vec![commit("abc", &["def"], "Second", &["a.txt"]), commit("def", &[], "First", &["b.txt"])])
            }
            when(printed = "/abc\0def ghi\0Ada\0ada@example.com\x001700000000\0Merge\0/def\0\0Ada\0ada@example.com\x001700000000\0First\0\nb.txt\0".as_bytes())
                as a_commit_that_changed_nothing to have_no_files {
                equal(vec![commit("abc", &["def", "ghi"], "Merge", &[]), commit("def", &[], "First", &["b.txt"])])
            }
            when(printed = "/abc\0\0Ada\0ada@example.com\x001700000000\0/usr is gone\0\n\u{17c}.txt\0\nodd.txt\0".as_bytes())
                as unusual_text to be_kept_untouched {
                equal(vec![commit("abc", &[], "/usr is gone", &["\u{17c}.txt", "\nodd.txt"])])
            }
            when(printed = "/abc\0\0Ada\0ada@example.com\x001700000000\0".as_bytes()) as a_commit_with_an_empty_message to have_no_subject {
                equal(vec![commit("abc", &[], "", &[])])
            }
            when(printed = "/abc\0def\0Ada\0ada@example.com\x00soon\0Odd time\0".as_bytes()) as a_time_that_is_no_number to count_from_the_start {
                equal(vec![Logged { time: 0, ..commit("abc", &["def"], "Odd time", &[]) }])
            }
            when(printed = "/abc\0def\0Ada".as_bytes()) as a_commit_cut_short to be_left_out { equal(Vec::new()) }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(Vec::new()) }
        }

        expect(counts(printed)) {
            when(printed = "3\t0\ta.txt\x001\t2\tsrc/b c.txt\0".as_bytes()) as two_files to count_both {
                equal(counted(&[count("a.txt", 3, 0), count("src/b c.txt", 1, 2)]))
            }
            when(printed = "-\t-\tpicture.bin\x001\t1\ta.txt\0".as_bytes()) as a_file_that_is_not_text to be_left_out {
                equal(counted(&[count("a.txt", 1, 1)]))
            }
            when(printed = "1\t1\ttab\there.txt\0".as_bytes()) as a_tab_in_a_name to stay_in_the_name {
                equal(counted(&[count("tab\there.txt", 1, 1)]))
            }
            when(printed = "1\t1\0".as_bytes()) as a_count_without_a_file to be_skipped { equal(counted(&[])) }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(counted(&[])) }
        }

        expect(changed_files(changed, &counts)) {
            let changed = vec![("b.txt".to_string(), FileChange::Modified), ("a.txt".to_string(), FileChange::Added)];
            let counts = counted(&[count("a.txt", 3, 0), count("b.txt", 1, 2)]);

            when every_file_is_counted to list_them_sorted_with_their_counts {
                equal(vec![
                    file("a.txt", FileChange::Added, Some(3), Some(0)),
                    file("b.txt", FileChange::Modified, Some(1), Some(2)),
                ])
            }
            when a_file_has_no_count {
                let counts = counted(&[count("a.txt", 3, 0)]);
                to list_it_without_one {
                    equal(vec![
                        file("a.txt", FileChange::Added, Some(3), Some(0)),
                        file("b.txt", FileChange::Modified, None, None),
                    ])
                }
            }
            when nothing_changed {
                let changed = Vec::new();
                to be_empty { equal(Vec::new()) }
            }
        }
    }
}
