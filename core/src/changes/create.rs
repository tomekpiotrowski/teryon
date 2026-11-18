//! Starting a change: creating the worktree an agent asked for, or taking over one it made itself.

use std::collections::HashSet;
use std::fs::{canonicalize, create_dir_all, read_dir};
use std::path::Path;

use crate::changes::name::{check_name, free_name};
use crate::changes::{copies_folder, Change, ChangeError, FoundCopy};
use crate::files::git::{first_line, lines, Git};

/// Creates `.teryon/worktrees/<name>` on a new branch `<name>` and returns the change.
///
/// It starts from `from` when given, else from the branch `project` is on,
/// else from the commit it is on.
pub async fn create(project: &Path, name: &str, from: Option<&str>) -> Result<Change, ChangeError> {
    check_name(name)?;
    let git = Git::at(project);
    let top = git.attempt(&["rev-parse", "--show-toplevel"]).await?;
    if top.is_none_or(|printed| first_line(&printed).is_empty()) {
        return Err(ChangeError::NotARepository);
    }

    let folder = copies_folder(project);
    let branches: HashSet<String> = lines(
        &git.output(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .await?,
    )
    .into_iter()
    .collect();
    let folders = folder_names(&folder);
    let taken = |candidate: &str| branches.contains(candidate) || folders.contains(candidate);
    if taken(name) {
        return Err(ChangeError::NameTaken {
            name: name.to_string(),
            suggestion: free_name(name, taken),
        });
    }

    let base = match from {
        Some(branch) if branches.contains(branch) => Some(branch.to_string()),
        Some(branch) => return Err(ChangeError::UnknownBranch(branch.to_string())),
        None => current_branch(&git).await?,
    };
    let starting_point = base
        .as_deref()
        .map_or("HEAD".to_string(), |branch| format!("refs/heads/{branch}"));
    let start = git
        .attempt(&[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{starting_point}^{{commit}}"),
        ])
        .await?
        .map(|printed| first_line(&printed))
        .filter(|commit| !commit.is_empty())
        .ok_or(ChangeError::NoCommits)?;

    create_dir_all(&folder)?;
    let path = folder.join(name);
    let path_text = path.to_string_lossy().into_owned();
    git.output(&["worktree", "add", "-q", "-b", name, &path_text, &start])
        .await?;
    Ok(Change {
        name: name.to_string(),
        path: canonicalize(&path)?,
        base,
        start,
        branch: Some(name.to_string()),
        state: None,
        review: None,
    })
}

/// The change for a worktree an agent created itself in the copies folder.
///
/// The agent did not say where it started, so the change is taken to start
/// from the branch the project is on now, where it last met that branch.
pub async fn take_over(project: &Path, copy: &FoundCopy) -> Result<Change, ChangeError> {
    let git = Git::at(project);
    let base = current_branch(&git).await?;
    let tip = first_line(&Git::at(&copy.path).output(&["rev-parse", "HEAD"]).await?);
    let start = git
        .attempt(&["merge-base", "HEAD", &tip])
        .await?
        .map(|printed| first_line(&printed))
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| tip.clone());
    Ok(Change {
        name: copy.name.clone(),
        path: copy.path.clone(),
        base,
        start,
        branch: copy.branch.clone(),
        state: None,
        review: None,
    })
}

/// The branch a directory is on. Nothing when it is not on one.
async fn current_branch(git: &Git) -> Result<Option<String>, ChangeError> {
    Ok(git
        .attempt(&["symbolic-ref", "--short", "-q", "HEAD"])
        .await?
        .map(|printed| first_line(&printed))
        .filter(|name| !name.is_empty()))
}

/// The names of everything in the copies folder, worktree or not.
fn folder_names(folder: &Path) -> HashSet<String> {
    read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}
