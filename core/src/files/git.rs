//! Runs the `git` program and reads what it prints.
//!
//! Everything the file browser knows about a project comes from git: which
//! files there are, which branches, and what changed. Starting the program
//! keeps Teryon's answers identical to what a person sees in a terminal.
//!
//! The file browser and the history only read; changes, see `crate::changes`,
//! also create worktrees. Git takes no optional locks, because
//! an agent may be using the same repository at that moment, and it never
//! looks above the directory it is given, so a plain folder that happens to
//! sit inside some other repository is not mistaken for one.

use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use thiserror::Error;
use tokio::process::Command;

use crate::files::FileChange;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git could not be started: {0}")]
    NotStarted(#[from] IoError),
    #[error("`git {command}` failed: {complaint}")]
    Failed { command: String, complaint: String },
}

/// The `git` program, pointed at one working directory.
#[derive(Clone, Debug)]
pub struct Git {
    directory: PathBuf,
    environment: Vec<(String, String)>,
}

impl Git {
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            environment: Vec::new(),
        }
    }

    /// The same, with an environment variable set for every run.
    pub fn with_env(mut self, name: &str, value: impl Into<String>) -> Self {
        self.environment.push((name.to_string(), value.into()));
        self
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// What git printed. A run that ends in failure is an error.
    pub async fn output(&self, arguments: &[&str]) -> Result<Vec<u8>, GitError> {
        let (code, printed, complaint) = self.run(arguments).await?;
        if code == Some(0) {
            Ok(printed)
        } else {
            Err(GitError::Failed {
                command: arguments.join(" "),
                complaint,
            })
        }
    }

    /// What git printed, or nothing when git answered "no".
    ///
    /// Git reports a missing branch, file or repository by ending in failure,
    /// which for a question is an answer rather than an error.
    pub async fn attempt(&self, arguments: &[&str]) -> Result<Option<Vec<u8>>, GitError> {
        let (code, printed, _) = self.run(arguments).await?;
        Ok((code == Some(0)).then_some(printed))
    }

    /// How git ended: its exit code, or nothing when a signal ended it.
    ///
    /// For the commands that answer with their exit code, such as
    /// `merge-tree`, where 1 means conflicts and anything else above 0 a failure.
    pub async fn exit_code(&self, arguments: &[&str]) -> Result<Option<i32>, GitError> {
        let (code, _, _) = self.run(arguments).await?;
        Ok(code)
    }

    async fn run(&self, arguments: &[&str]) -> Result<(Option<i32>, Vec<u8>, String), GitError> {
        let mut command = Command::new("git");
        command
            // A repository can name a program to watch its files. Reading must
            // never start one.
            .args(["-c", "core.fsmonitor=false"])
            .args(arguments)
            .current_dir(&self.directory)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        if let Some(parent) = self.directory.parent() {
            command.env("GIT_CEILING_DIRECTORIES", parent);
        }

        let output = command.output().await?;
        Ok((
            output.status.code(),
            output.stdout,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// The first line of what git printed, without its line ending.
pub fn first_line(printed: &[u8]) -> String {
    String::from_utf8_lossy(printed)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The lines git printed, leaving out empty ones.
pub fn lines(printed: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(printed)
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

/// The parts of an answer asked for with `-z`, where each part ends in a zero byte.
///
/// That form is the only one in which git prints every file name untouched.
pub fn zero_separated(printed: &[u8]) -> Vec<String> {
    printed
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// Reads `git diff --name-status -z`: a letter, then the file it is about.
///
/// Renames are switched off where this is asked for, so every letter is
/// followed by exactly one file. Anything that is neither an addition nor a
/// removal counts as a modification.
pub fn changes(printed: &[u8]) -> Vec<(String, FileChange)> {
    let parts = zero_separated(printed);
    let (pairs, _) = parts.as_chunks::<2>();
    pairs
        .iter()
        .map(|[letter, path]| {
            let change = match letter.chars().next() {
                Some('A') => FileChange::Added,
                Some('D') => FileChange::Deleted,
                _ => FileChange::Modified,
            };
            (path.clone(), change)
        })
        .collect()
}

/// A place where a branch is checked out, as `git worktree list` reports it.
#[derive(Debug, PartialEq, Eq)]
pub struct Checkout {
    pub directory: PathBuf,
    /// Nothing when the directory is not on a branch.
    pub branch: Option<String>,
}

/// Reads `git worktree list --porcelain -z`.
///
/// Each checkout is a run of `name value` parts closed by an empty part.
pub fn checkouts(printed: &[u8]) -> Vec<Checkout> {
    let mut found = Vec::new();
    let mut directory: Option<PathBuf> = None;
    let mut branch: Option<String> = None;

    for part in printed.split(|byte| *byte == 0) {
        let part = String::from_utf8_lossy(part);
        if part.is_empty() {
            if let Some(directory) = directory.take() {
                found.push(Checkout {
                    directory,
                    branch: branch.take(),
                });
            }
            branch = None;
        } else if let Some(path) = part.strip_prefix("worktree ") {
            directory = Some(PathBuf::from(path));
        } else if let Some(name) = part.strip_prefix("branch refs/heads/") {
            branch = Some(name.to_string());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn checkout(directory: &str, branch: Option<&str>) -> Checkout {
        Checkout {
            directory: PathBuf::from(directory),
            branch: branch.map(String::from),
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    lets_expect! {
        expect(first_line(printed)) {
            when(printed = "main\n".as_bytes()) as one_line to drop_the_line_ending { equal("main".to_string()) }
            when(printed = "abc\ndef\n".as_bytes()) as several_lines to keep_the_first { equal("abc".to_string()) }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(String::new()) }
        }

        expect(lines(printed)) {
            when(printed = "main\nfeature/one\n".as_bytes()) as two_lines to list_both { equal(names(&["main", "feature/one"])) }
            when(printed = "main\n\n".as_bytes()) as an_empty_line to skip_it { equal(names(&["main"])) }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(names(&[])) }
        }

        expect(zero_separated(printed)) {
            when(printed = "a.txt\0dir/b c.txt\0".as_bytes()) as two_names to list_both { equal(names(&["a.txt", "dir/b c.txt"])) }
            when(printed = "line\nbreak.txt\0\u{17c}.txt\0".as_bytes()) as unusual_names to keep_them_untouched {
                equal(names(&["line\nbreak.txt", "\u{17c}.txt"]))
            }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(names(&[])) }
        }

        expect(changes(printed)) {
            when(printed = "M\0a.txt\0A\0b.txt\0D\0c.txt\0".as_bytes()) as each_letter to name_its_change {
                equal(vec![
                    ("a.txt".to_string(), FileChange::Modified),
                    ("b.txt".to_string(), FileChange::Added),
                    ("c.txt".to_string(), FileChange::Deleted),
                ])
            }
            when(printed = "T\0link\0U\0clash.txt\0".as_bytes()) as other_letters to count_as_modified {
                equal(vec![
                    ("link".to_string(), FileChange::Modified),
                    ("clash.txt".to_string(), FileChange::Modified),
                ])
            }
            when(printed = "M\0".as_bytes()) as a_letter_without_a_file to be_skipped { equal(Vec::new()) }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(Vec::new()) }
        }

        expect(checkouts(printed)) {
            when(printed = "worktree /work/teryon\0HEAD 1234\0branch refs/heads/main\0\0worktree /work/teryon--files\0HEAD 5678\0branch refs/heads/feature/files\0\0".as_bytes())
                as two_checkouts to list_both {
                equal(vec![
                    checkout("/work/teryon", Some("main")),
                    checkout("/work/teryon--files", Some("feature/files")),
                ])
            }
            when(printed = "worktree /work/teryon\0HEAD 1234\0detached\0\0".as_bytes()) as a_checkout_off_any_branch to have_no_branch {
                equal(vec![checkout("/work/teryon", None)])
            }
            when(printed = "worktree /work/a\0HEAD 1\0branch refs/heads/main\0\0worktree /work/b\0HEAD 2\0detached\0\0".as_bytes())
                as a_branch_followed_by_none to not_carry_the_branch_over {
                equal(vec![checkout("/work/a", Some("main")), checkout("/work/b", None)])
            }
            when(printed = "worktree /work/bare\0bare\0\0".as_bytes()) as a_bare_repository to have_no_branch {
                equal(vec![checkout("/work/bare", None)])
            }
            when(printed = "".as_bytes()) as nothing to be_empty { equal(Vec::new()) }
        }
    }
}
