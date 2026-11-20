//! Where a change stands: is its copy there, how many files changed, and would it apply.
//!
//! "Files changed" means the copy as it is on disk, whether the agent
//! committed or not, because the person does not know or care about commits.
//! So the copy's files are first recorded as a tree through a temporary index
//! of their own, which leaves the agent's index, files and branch untouched.
//!
//! Whether the change would apply is the question `git merge-tree` answers
//! without touching any files. It is slower than the rest, so it is only
//! asked for when looking thoroughly, see [`Look`].

use std::fs::{canonicalize, copy, remove_file};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::changes::{Change, ChangeError, ChangeState, Fit};
use crate::files::git::{checkouts, first_line, zero_separated, Git, GitError};

/// How closely to look at a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Everything but whether it would apply. For every second of a turn.
    Quick,
    /// Everything. When a turn ends, when the thread is opened.
    Thorough,
}

/// Files in a worktree's git folder that mean the agent left something half done.
const UNFINISHED_MARKS: [&str; 4] = [
    "rebase-merge",
    "rebase-apply",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
];

/// Reads where `change` stands. Also gives the branch checked out in its copy now.
pub async fn read_state(
    project: &Path,
    change: &Change,
    look: Look,
) -> Result<(ChangeState, Option<String>), ChangeError> {
    let git = Git::at(project);
    let listed = git
        .output(&["worktree", "list", "--porcelain", "-z"])
        .await?;
    let Ok(wanted) = canonicalize(&change.path) else {
        return Ok((ChangeState::gone(), None));
    };
    let Some(checkout) = checkouts(&listed)
        .into_iter()
        .find(|checkout| canonicalize(&checkout.directory).is_ok_and(|found| found == wanted))
    else {
        return Ok((ChangeState::gone(), None));
    };
    let branch = checkout.branch;
    let worktree = Git::at(&change.path);

    let base_ref = change
        .base
        .as_ref()
        .map(|base| format!("refs/heads/{base}"));
    let base_present = match &base_ref {
        Some(reference) => git
            .attempt(&["rev-parse", "--verify", "-q", reference])
            .await?
            .is_some(),
        None => false,
    };
    let unfinished = branch.as_deref() != Some(change.name.as_str())
        || half_done(&worktree, &change.path).await?;

    let fork = match (&base_ref, base_present) {
        (Some(reference), true) => worktree
            .attempt(&["merge-base", reference, "HEAD"])
            .await?
            .map(|printed| first_line(&printed))
            .unwrap_or_else(|| change.start.clone()),
        _ => change.start.clone(),
    };
    let tree = snapshot(&worktree, &change.path).await?;
    let files_changed = zero_separated(
        &worktree
            .output(&["diff", "--name-only", "--no-renames", "-z", &fork, &tree])
            .await?,
    )
    .len();

    let fit = match (look, &base_ref, base_present) {
        (Look::Thorough, Some(reference), true) => fit(&worktree, &tree, reference).await?,
        _ => Fit::Unknown,
    };

    Ok((
        ChangeState {
            copy_present: true,
            base_present,
            files_changed: u32::try_from(files_changed).unwrap_or(u32::MAX),
            fit,
            unfinished,
        },
        branch,
    ))
}

/// Whether the branch called `name` is in the project.
///
/// A copy removed with `git worktree remove` leaves its branch and the
/// agent's commits behind, so the change is not gone until the branch is.
pub async fn branch_present(project: &Path, name: &str) -> Result<bool, ChangeError> {
    let reference = format!("refs/heads/{name}");
    Ok(Git::at(project)
        .attempt(&["rev-parse", "--verify", "-q", &reference])
        .await?
        .is_some())
}

/// Whether a rebase, merge or cherry-pick was left unfinished in the worktree.
async fn half_done(worktree: &Git, path: &Path) -> Result<bool, GitError> {
    for mark in UNFINISHED_MARKS {
        let printed = worktree.output(&["rev-parse", "--git-path", mark]).await?;
        if path.join(first_line(&printed)).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Records every file in the worktree, committed or not, as a tree, and returns its name.
///
/// Works on a copy of the worktree's index, so git only reads the files that
/// changed since the agent last staged, and the agent's own index is untouched.
async fn snapshot(worktree: &Git, path: &Path) -> Result<String, ChangeError> {
    let index = path.join(first_line(
        &worktree
            .output(&["rev-parse", "--git-path", "index"])
            .await?,
    ));
    let temporary = temporary_index(&index);
    if index.exists() {
        copy(&index, &temporary)?;
    }
    let with_index = worktree
        .clone()
        .with_env("GIT_INDEX_FILE", temporary.to_string_lossy());
    let tree = async {
        with_index.output(&["add", "-A"]).await?;
        Ok::<_, GitError>(first_line(&with_index.output(&["write-tree"]).await?))
    }
    .await;
    let _ = remove_file(&temporary);
    Ok(tree?)
}

fn temporary_index(index: &Path) -> PathBuf {
    index.with_file_name(format!("teryon-index-{}", Uuid::new_v4()))
}

/// Whether the worktree's files, as recorded in `tree`, would merge into `base` without conflicts.
///
/// `merge-tree` needs commits, so the tree is wrapped in one that nothing
/// points to. Git throws such commits away by itself in time.
async fn fit(worktree: &Git, tree: &str, base: &str) -> Result<Fit, GitError> {
    let committer = worktree
        .clone()
        .with_env("GIT_AUTHOR_NAME", "Teryon")
        .with_env("GIT_AUTHOR_EMAIL", "teryon@localhost")
        .with_env("GIT_COMMITTER_NAME", "Teryon")
        .with_env("GIT_COMMITTER_EMAIL", "teryon@localhost");
    let commit = first_line(
        &committer
            .output(&[
                "commit-tree",
                tree,
                "-p",
                "HEAD",
                "-m",
                "Teryon's look at a worktree",
            ])
            .await?,
    );
    Ok(
        match worktree
            .exit_code(&["merge-tree", "--write-tree", base, &commit])
            .await?
        {
            Some(0) => Fit::Applies,
            Some(1) => Fit::Overlaps,
            _ => Fit::Unknown,
        },
    )
}
