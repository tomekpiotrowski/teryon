//! Finishing a change on this machine: applying it to the project, catching it up with newer work, or discarding it.
//!
//! These need no judgement when nothing overlaps, so Teryon does them itself
//! rather than spending an agent's turn on a few git commands. When the
//! change overlaps with newer work, Teryon touches nothing and says so; the
//! person can then ask the agent to sort it out.
//!
//! The person never sees git's words, so every refusal here is written in
//! the words of the screens: the change, its copy, the project, applying.
//!
//! Applying, step by step, where every step before moving the project can be
//! abandoned without a trace:
//!
//! 1. Anything the agent left uncommitted in the copy is committed, see [`save_leftovers`].
//! 2. `merge-tree` merges the change into its base without touching any files,
//!    giving the tree the project will have, or conflicts.
//! 3. `commit-tree` makes one commit of that tree on top of the base, with the
//!    person's description: the squash, made without a checkout.
//! 4. The base moves to that commit: with `merge --ff-only` in the folder it is
//!    checked out in, which refuses rather than overwrite the person's own
//!    edits, or with `update-ref` when it is checked out nowhere.
//! 5. The copy and its branch are removed. The branch is not part of the
//!    project's history after a squash, so it is deleted with `-D`.

use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::error;

use crate::changes::Change;
use crate::files::git::{checkouts, first_line, Git, GitError};

/// The description of a commit Teryon makes of what the agent left uncommitted.
pub const LEFTOVERS: &str = "Work in progress";

/// Why a change could not be finished. The text is meant for the person.
#[derive(Debug, Error)]
pub enum FinishError {
    #[error("This thread has no changes.")]
    NoChange,
    #[error("Wait for the agent to finish, or stop it.")]
    AgentRunning,
    #[error("This thread's copy was removed outside Teryon. Discard the changes to forget them.")]
    CopyGone,
    #[error("The version of the project this change started from is gone, so there is nothing to apply it to.")]
    BaseGone,
    #[error("The agent left this change unfinished. Ask it to finish, or discard the changes.")]
    Unfinished,
    #[error("Describe the change first.")]
    NoDescription,
    #[error("No files have changed, so there is nothing to apply.")]
    NothingToApply,
    #[error(
        "This change overlaps with newer work in the project. Ask the agent to catch up first."
    )]
    Overlaps,
    #[error("You have edits of your own to {} in the project folder. Applying would overwrite them.", listed(.0))]
    OwnEdits(Vec<String>),
    #[error("The project changed while the change was being applied. Try again.")]
    ProjectMoved,
    #[error("This change already has all the newer work in the project.")]
    UpToDate,
    #[error("Something went wrong. The service's log has the details.")]
    Git(#[from] GitError),
}

/// How catching up went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaughtUp {
    /// The change now starts from the project as it is.
    Done,
    /// It overlaps with newer work, so it was left exactly as it was.
    Overlaps,
}

/// Applies `change` to its base as one commit described by `description`, and removes its copy.
///
/// Returns the new commit. The caller has checked that no agent is working in the copy.
pub async fn apply(
    project: &Path,
    change: &Change,
    description: &str,
) -> Result<String, FinishError> {
    let description = description.trim();
    if description.is_empty() {
        return Err(FinishError::NoDescription);
    }
    let base = ready(project, change).await?;
    let git = Git::at(project);
    let copy = Git::at(&change.path);

    save_leftovers(&copy).await?;
    let tip = first_line(&copy.output(&["rev-parse", "HEAD"]).await?);
    let base_commit = first_line(&git.output(&["rev-parse", &base]).await?);
    let fork = first_line(&git.output(&["merge-base", &base, &tip]).await?);
    if git.exit_code(&["diff", "--quiet", &fork, &tip]).await? == Some(0) {
        return Err(FinishError::NothingToApply);
    }

    let tree = match git
        .answer(&["merge-tree", "--write-tree", &base_commit, &tip])
        .await?
    {
        (Some(0), printed) => first_line(&printed),
        (Some(1), _) => return Err(FinishError::Overlaps),
        (code, _) => {
            return Err(FinishError::Git(GitError::Failed {
                command: "merge-tree".to_string(),
                complaint: format!("ended with {code:?}"),
            }))
        }
    };
    let squashed = first_line(
        &signed(&git)
            .await?
            .output(&["commit-tree", &tree, "-p", &base_commit, "-m", description])
            .await?,
    );

    move_base(project, change, &base_commit, &squashed).await?;
    remove(project, change).await?;
    Ok(squashed)
}

/// Brings `change` up to date with newer work in its base, unless the two overlap.
pub async fn catch_up(project: &Path, change: &Change) -> Result<CaughtUp, FinishError> {
    let base = ready(project, change).await?;
    let copy = Git::at(&change.path);
    save_leftovers(&copy).await?;
    if copy
        .exit_code(&["merge-base", "--is-ancestor", &base, "HEAD"])
        .await?
        == Some(0)
    {
        return Err(FinishError::UpToDate);
    }
    let signed_copy = signed(&copy).await?;
    match signed_copy.output(&["rebase", "--quiet", &base]).await {
        Ok(_) => Ok(CaughtUp::Done),
        // Stopped part way, at a conflict: undone, so the copy is as it was.
        Err(GitError::Failed { .. }) if mid_way(&copy, &change.path).await? => {
            copy.output(&["rebase", "--abort"]).await?;
            Ok(CaughtUp::Overlaps)
        }
        Err(problem) => Err(problem.into()),
    }
}

/// Removes `change`'s copy with everything in it, and its branch. Whatever is already gone is fine.
pub async fn discard(project: &Path, change: &Change) -> Result<(), FinishError> {
    remove(project, change).await
}

/// A starting point for the person's description of `change`: what the agent said about its work.
///
/// The agent's own description when it committed once. When it committed
/// several times, the newest description, followed by the first lines of the
/// others, newest first. Commits Teryon made of leftovers are skipped.
/// Nothing when the agent committed nothing.
pub async fn draft_description(
    project: &Path,
    change: &Change,
) -> Result<Option<String>, FinishError> {
    let copy = Git::at(&change.path);
    if !change.path.is_dir() {
        return Err(FinishError::CopyGone);
    }
    let since = match &change.base {
        Some(base) => Git::at(project)
            .attempt(&["merge-base", &format!("refs/heads/{base}"), "HEAD"])
            .await?
            .map(|printed| first_line(&printed))
            .unwrap_or_else(|| change.start.clone()),
        None => change.start.clone(),
    };
    let printed = copy
        .output(&["log", "--format=%B%x00", &format!("{since}..HEAD")])
        .await?;
    Ok(draft_from(&String::from_utf8_lossy(&printed)))
}

/// The draft from what `git log --format=%B%x00` printed, newest first.
fn draft_from(printed: &str) -> Option<String> {
    let mut messages = printed
        .split('\0')
        .map(str::trim)
        .filter(|message| !message.is_empty() && *message != LEFTOVERS);
    let newest = messages.next()?.to_string();
    let others: Vec<String> = messages
        .filter_map(|message| message.lines().next())
        .map(|line| format!("- {line}"))
        .collect();
    if others.is_empty() {
        Some(newest)
    } else {
        Some(format!("{newest}\n\n{}", others.join("\n")))
    }
}

/// Commits everything uncommitted in a copy, so the change is the copy as it is. Says whether there was anything.
///
/// The person thinks of the change as the files in the copy, so files the
/// agent did not commit are part of it.
pub async fn save_leftovers(copy: &Git) -> Result<bool, FinishError> {
    copy.output(&["add", "-A"]).await?;
    if copy.exit_code(&["diff", "--cached", "--quiet"]).await? == Some(0) {
        return Ok(false);
    }
    signed(copy)
        .await?
        .output(&["commit", "-q", "--no-verify", "-m", LEFTOVERS])
        .await?;
    Ok(true)
}

/// Checks that a change can be applied or caught up, and gives its base as a full reference.
async fn ready(project: &Path, change: &Change) -> Result<String, FinishError> {
    if !change.path.is_dir() {
        return Err(FinishError::CopyGone);
    }
    let base = change
        .base
        .as_ref()
        .map(|base| format!("refs/heads/{base}"))
        .ok_or(FinishError::BaseGone)?;
    if Git::at(project)
        .attempt(&["rev-parse", "--verify", "-q", &base])
        .await?
        .is_none()
    {
        return Err(FinishError::BaseGone);
    }
    let copy = Git::at(&change.path);
    let branch = copy
        .attempt(&["symbolic-ref", "--short", "-q", "HEAD"])
        .await?
        .map(|printed| first_line(&printed));
    if branch.as_deref() != Some(change.name.as_str()) || mid_way(&copy, &change.path).await? {
        return Err(FinishError::Unfinished);
    }
    Ok(base)
}

/// Whether a rebase or merge is under way in the copy.
async fn mid_way(copy: &Git, path: &Path) -> Result<bool, GitError> {
    for mark in [
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
    ] {
        let printed = copy.output(&["rev-parse", "--git-path", mark]).await?;
        if path.join(first_line(&printed)).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Moves the base from `from` to `to`, a commit on top of it.
async fn move_base(
    project: &Path,
    change: &Change,
    from: &str,
    to: &str,
) -> Result<(), FinishError> {
    let base = change.base.as_deref().unwrap_or_default();
    let listed = Git::at(project)
        .output(&["worktree", "list", "--porcelain", "-z"])
        .await?;
    let checked_out: Option<PathBuf> = checkouts(&listed)
        .into_iter()
        .find(|checkout| checkout.branch.as_deref() == Some(base))
        .map(|checkout| checkout.directory);
    match checked_out {
        Some(directory) => fast_forward(&directory, from, to).await,
        None => {
            let reference = format!("refs/heads/{base}");
            Git::at(project)
                .output(&["update-ref", &reference, to, from])
                .await
                .map(|_| ())
                .map_err(|_| FinishError::ProjectMoved)
        }
    }
}

/// Moves the branch checked out in `directory` to `to`, and its files with it.
///
/// Git refuses rather than overwrite edits the person has not committed, and
/// names the files, which the refusal passes on.
async fn fast_forward(directory: &Path, from: &str, to: &str) -> Result<(), FinishError> {
    let git = Git::at(directory);
    let head = first_line(&git.output(&["rev-parse", "HEAD"]).await?);
    if head != from {
        return Err(FinishError::ProjectMoved);
    }
    match git.output(&["merge", "--ff-only", "--quiet", to]).await {
        Ok(_) => Ok(()),
        Err(GitError::Failed { complaint, .. }) => {
            let files = files_in_way(&complaint);
            if files.is_empty() {
                error!("could not move the project forward: {complaint}");
                Err(FinishError::ProjectMoved)
            } else {
                Err(FinishError::OwnEdits(files))
            }
        }
        Err(problem) => Err(problem.into()),
    }
}

/// The files git names, one per line after a tab, when it refuses to overwrite them.
fn files_in_way(complaint: &str) -> Vec<String> {
    complaint
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .map(|file| file.trim().to_string())
        .filter(|file| !file.is_empty())
        .collect()
}

/// Removes the copy and its branch, tolerating either being gone already.
async fn remove(project: &Path, change: &Change) -> Result<(), FinishError> {
    let git = Git::at(project);
    if change.path.is_dir() {
        let path = change.path.to_string_lossy().into_owned();
        git.output(&["worktree", "remove", "--force", &path])
            .await?;
    } else {
        git.output(&["worktree", "prune"]).await?;
    }
    let branch = format!("refs/heads/{}", change.name);
    if git
        .attempt(&["rev-parse", "--verify", "-q", &branch])
        .await?
        .is_some()
    {
        git.output(&["branch", "-D", &change.name]).await?;
    }
    Ok(())
}

/// The same git, with someone to make commits as.
///
/// The person's own name when git knows it, so the project's history says
/// who applied a change. Otherwise Teryon's, rather than failing.
async fn signed(git: &Git) -> Result<Git, GitError> {
    if git.attempt(&["config", "user.email"]).await?.is_some() {
        return Ok(git.clone());
    }
    Ok(git
        .clone()
        .with_env("GIT_AUTHOR_NAME", "Teryon")
        .with_env("GIT_AUTHOR_EMAIL", "teryon@localhost")
        .with_env("GIT_COMMITTER_NAME", "Teryon")
        .with_env("GIT_COMMITTER_EMAIL", "teryon@localhost"))
}

/// Names files the way a sentence would: `a`, `a and b`, `a, b and c`.
fn listed(files: &[String]) -> String {
    match files {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn names(files: &[&str]) -> Vec<String> {
        files.iter().map(|file| file.to_string()).collect()
    }

    /// Every refusal, for checking their words.
    fn refusals() -> Vec<String> {
        [
            FinishError::NoChange,
            FinishError::AgentRunning,
            FinishError::CopyGone,
            FinishError::BaseGone,
            FinishError::Unfinished,
            FinishError::NoDescription,
            FinishError::NothingToApply,
            FinishError::Overlaps,
            FinishError::OwnEdits(names(&["a.rs"])),
            FinishError::ProjectMoved,
            FinishError::UpToDate,
            FinishError::Git(GitError::Failed {
                command: "merge".to_string(),
                complaint: "branch".to_string(),
            }),
        ]
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    /// Whether a text holds one of git's words.
    fn speaks_git(text: &str) -> bool {
        let lower = text.to_lowercase();
        [
            "branch",
            "commit",
            "merge",
            "rebase",
            "worktree",
            "push",
            "pull request",
            "checkout",
            "stash",
        ]
        .iter()
        .any(|word| lower.contains(word))
    }

    lets_expect! {
        expect(files_in_way(complaint)) {
            when(complaint = "error: Your local changes to the following files would be overwritten by merge:\n\tREADME.md\n\tsrc/main.rs\nPlease commit your changes or stash them before you merge.\nAborting") as edits_in_the_way to name_them {
                equal(names(&["README.md", "src/main.rs"]))
            }
            when(complaint = "error: The following untracked working tree files would be overwritten by merge:\n\tnew.txt\nPlease move or remove them before you merge.\nAborting") as new_files_in_the_way to name_them {
                equal(names(&["new.txt"]))
            }
            when(complaint = "fatal: Not possible to fast-forward, aborting.") as another_refusal to name_nothing { equal(Vec::<String>::new()) }
        }

        expect(listed(&names(&files))) {
            when(files = vec!["a.rs"]) as one_file to be_named_alone { equal("a.rs".to_string()) }
            when(files = vec!["a.rs", "b.rs"]) as two_files to be_joined_with_and { equal("a.rs and b.rs".to_string()) }
            when(files = vec!["a.rs", "b.rs", "c.rs"]) as three_files to be_listed { equal("a.rs, b.rs and c.rs".to_string()) }
        }

        expect(draft_from(printed)) {
            when(printed = "") as no_commits to give_nothing { equal(None) }
            when(printed = "Work in progress\n\0") as only_leftovers to give_nothing { equal(None) }
            when(printed = "Fix the login\n\nIt redirected twice.\n\0") as one_commit to give_its_description {
                equal(Some("Fix the login\n\nIt redirected twice.".to_string()))
            }
            when(printed = "Add tests\n\nFor the redirect.\n\0Work in progress\n\0Fix the login\n\nIt redirected twice.\n\0") as several_commits {
                to give_the_newest_then_the_first_lines_of_the_others {
                    equal(Some("Add tests\n\nFor the redirect.\n\n- Fix the login".to_string()))
                }
            }
        }

        expect(refusals().into_iter().filter(|text| speaks_git(text)).collect::<Vec<_>>()) as the_refusals {
            to never_use_a_git_word { equal(Vec::<String>::new()) }
        }
    }
}
