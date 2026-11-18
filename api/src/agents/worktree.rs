//! Where a thread's agent may check out a branch, and how that checkout is recognised.
//!
//! The agent decides whether a task needs a worktree. A question does not.
//! What every agent must do the same way is the place: `.teryon/worktrees/<thread id>`
//! inside the project, on a new branch. Teryon cannot assume it may write next
//! to the project, so the checkout stays inside it.
//!
//! The instruction is handed to the program beside the person's message.
//! Afterwards the checkout is found by that path in `git worktree list`, and
//! the branch checked out there is stored on the thread.

use std::fs::canonicalize;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::files::git::{checkouts, first_line, Checkout, Git, GitError};

/// The line added to `.git/info/exclude` so a checkout is not an untracked directory.
const WORKTREES_IGNORE: &str = ".teryon/worktrees/";

/// The directory of a thread's worktree, relative to the project.
pub fn worktree_relative(thread_id: Uuid) -> PathBuf {
    Path::new(".teryon")
        .join("worktrees")
        .join(thread_id.to_string())
}

/// The instruction a thread's agent is given about worktrees.
///
/// `start_branch` is the branch the project directory is on. Nothing when
/// that directory is not on a branch: the agent then starts from the commit
/// it is on. A branch named in the message still wins over either.
pub fn instruction(thread_id: Uuid, start_branch: Option<&str>) -> String {
    let relative = worktree_relative(thread_id);
    let directory = relative.display();
    let start = match start_branch {
        Some(branch) => format!("the branch `{branch}`"),
        None => "the commit this directory is on".to_string(),
    };
    format!(
        concat!(
            "Change files only in a git worktree at `{directory}`, on a new branch. ",
            "Create it only when the work will change files. A question that changes nothing creates no worktree. ",
            "Start the new branch from the branch named in the message, or, when the message names none, from {start}. ",
            "Leave this directory on its current branch. Do not create a worktree anywhere else."
        ),
        directory = directory,
        start = start,
    )
}

/// The exclude file with the worktree line added, or nothing when the line is already there.
pub fn with_worktrees_ignored(exclude: &str) -> Option<String> {
    if exclude.lines().any(|line| line.trim() == WORKTREES_IGNORE) {
        return None;
    }
    let mut text = exclude.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(WORKTREES_IGNORE);
    text.push('\n');
    Some(text)
}

/// The branch checked out at a thread's worktree path, or nothing when that path is not one.
pub fn branch_checked_out(found: &[Checkout], project: &Path, thread_id: Uuid) -> Option<String> {
    let wanted = canonicalize(project.join(worktree_relative(thread_id))).ok()?;
    found.iter().find_map(|checkout| {
        let directory = canonicalize(&checkout.directory).ok()?;
        if directory != wanted {
            return None;
        }
        checkout.branch.clone()
    })
}

/// The instruction for one turn, after making sure the checkout directory is ignored.
///
/// Nothing when `directory` is not a git repository: there is nowhere to
/// check a branch out, and a plain folder is left untouched.
pub async fn instruction_for(directory: &Path, thread_id: Uuid) -> Option<String> {
    let git = Git::at(directory);
    let top = git
        .attempt(&["rev-parse", "--show-toplevel"])
        .await
        .ok()??;
    if first_line(&top).is_empty() {
        return None;
    }
    ignore_worktrees(&git).await;
    let start = git
        .attempt(&["symbolic-ref", "--short", "-q", "HEAD"])
        .await
        .ok()
        .flatten()
        .map(|printed| first_line(&printed))
        .filter(|name| !name.is_empty());
    Some(instruction(thread_id, start.as_deref()))
}

/// The branch at the thread's worktree, read from git.
///
/// An error means the list could not be read, which is not the same as the
/// checkout being gone: the caller keeps whatever it stored last time.
pub async fn observed_branch(
    directory: &Path,
    thread_id: Uuid,
) -> Result<Option<String>, GitError> {
    let git = Git::at(directory);
    let printed = git
        .output(&["worktree", "list", "--porcelain", "-z"])
        .await?;
    Ok(branch_checked_out(
        &checkouts(&printed),
        directory,
        thread_id,
    ))
}

/// Adds the worktree ignore line to this repository's exclude file, once.
async fn ignore_worktrees(git: &Git) {
    let Ok(Some(printed)) = git.attempt(&["rev-parse", "--git-common-dir"]).await else {
        return;
    };
    // Relative for an ordinary repository, absolute when this directory is a linked worktree.
    let mut common = PathBuf::from(first_line(&printed));
    if !common.is_absolute() {
        common = git.directory().join(common);
    }
    let exclude = common.join("info").join("exclude");
    let current = tokio::fs::read_to_string(&exclude)
        .await
        .unwrap_or_default();
    let Some(updated) = with_worktrees_ignored(&current) else {
        return;
    };
    if let Some(parent) = exclude.parent() {
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return;
        }
    }
    let _ = tokio::fs::write(&exclude, updated).await;
}

#[cfg(test)]
mod tests {
    use std::fs::create_dir_all;

    use lets_expect::lets_expect;
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::*;
    use crate::files::git::Checkout;

    fn checkout(directory: &Path, branch: Option<&str>) -> Checkout {
        Checkout {
            directory: directory.to_path_buf(),
            branch: branch.map(str::to_string),
        }
    }

    struct Folder {
        root: tempfile::TempDir,
    }

    impl Folder {
        fn new() -> Self {
            Self {
                root: tempdir().expect("a temporary directory"),
            }
        }

        fn project(&self) -> &Path {
            self.root.path()
        }

        fn make(&self, relative: &Path) -> PathBuf {
            let made = self.project().join(relative);
            create_dir_all(&made).expect("the directory is made");
            made
        }
    }

    /// The branch `branch_checked_out` reports for one arranged repository.
    fn checked(kind: &str) -> Option<String> {
        let folders = Folder::new();
        let id = Uuid::nil();
        let found = match kind {
            "none" => Vec::new(),
            "feature" => vec![checkout(
                &folders.make(&worktree_relative(id)),
                Some("feature"),
            )],
            "detached" => vec![checkout(&folders.make(&worktree_relative(id)), None)],
            "elsewhere" => vec![checkout(
                &folders.make(Path::new("elsewhere")),
                Some("feature"),
            )],
            other => panic!("there is no arrangement called {other}"),
        };
        branch_checked_out(&found, folders.project(), id)
    }

    lets_expect! {
        expect(instruction(id, start)) {
            let id = Uuid::nil();
            let start = Some("main");

            to name_the_directory_and_the_branch_the_project_is_on {
                equal(
                    "Change files only in a git worktree at `.teryon/worktrees/00000000-0000-0000-0000-000000000000`, on a new branch. \
                     Create it only when the work will change files. A question that changes nothing creates no worktree. \
                     Start the new branch from the branch named in the message, or, when the message names none, from the branch `main`. \
                     Leave this directory on its current branch. Do not create a worktree anywhere else."
                        .to_string(),
                )
            }

            when(start = None) as name_the_commit_when_the_directory_is_not_on_a_branch to hold {
                equal(
                    "Change files only in a git worktree at `.teryon/worktrees/00000000-0000-0000-0000-000000000000`, on a new branch. \
                     Create it only when the work will change files. A question that changes nothing creates no worktree. \
                     Start the new branch from the branch named in the message, or, when the message names none, from the commit this directory is on. \
                     Leave this directory on its current branch. Do not create a worktree anywhere else."
                        .to_string(),
                )
            }
        }

        expect(with_worktrees_ignored(text)) {
            when(text = "") as add_the_line to hold { equal(Some(".teryon/worktrees/\n".to_string())) }
            when(text = "# kept\n") as keep_what_is_already_there to hold {
                equal(Some("# kept\n.teryon/worktrees/\n".to_string()))
            }
            when(text = "other") as break_the_last_line_before_adding_it to hold {
                equal(Some("other\n.teryon/worktrees/\n".to_string()))
            }
            when(text = ".teryon/worktrees/\n") as leave_an_existing_line to hold { equal(None) }
            when(text = "  .teryon/worktrees/  \n") as recognise_the_line_with_spaces_around_it to hold { equal(None) }
            when(text = "# .teryon/worktrees/\n") as add_it_when_only_a_comment_mentions_it to hold {
                equal(Some("# .teryon/worktrees/\n.teryon/worktrees/\n".to_string()))
            }
        }

        expect(checked(kind)) {
            when(kind = "none") as when_the_thread_has_no_checkout to hold { equal(None) }
            when(kind = "feature") as when_the_checkout_is_this_threads_worktree to hold {
                equal(Some("feature".to_string()))
            }
            when(kind = "detached") as when_the_checkout_is_not_on_a_branch to hold { equal(None) }
            when(kind = "elsewhere") as when_another_directory_is_checked_out to hold { equal(None) }
        }
    }
}
