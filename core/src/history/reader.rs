//! Reads a project's history from its git repository.
//!
//! A branch's commits are listed from its last commit backwards. The commits
//! the branch has of its own come first and the ones the main branch has too
//! come after them, so what the branch added stays together above what it
//! started from and what it merged in. No commit is listed above one made on
//! top of it either way, because nothing on the main branch can have been
//! made on top of a commit the main branch does not have. Uncommitted work is
//! not history yet and is left to the file browser.
//!
//! A commit is compared with the commit it continues, which for a merge is
//! the branch that was merged into. That makes a merge show what it brought
//! in. Any two commits can be compared in its place.

use std::collections::BTreeSet;

use tokio::try_join;

use crate::files::git::{changes, lines, GitError};
use crate::files::repository::{clean_path, commit, main_branch, present, Repository};
use crate::files::{FileView, FilesError};
use crate::history::log::{changed_files, counts, logged, LOG_FORMAT};
use crate::history::{Changes, Commit, History};

/// The most commits listed at once.
pub const LISTED: usize = 200;

impl Repository {
    /// The commits of a branch. Without a branch, of whatever the project's directory is on.
    pub async fn history(&self, branch: Option<&str>) -> Result<History, FilesError> {
        let (branches, current) = self.branches().await?;
        let (branch, name) = match branch {
            None => (current, "HEAD".to_string()),
            Some(name) if branches.iter().any(|branch| branch == name) => {
                (Some(name.to_string()), format!("refs/heads/{name}"))
            }
            Some(_) => return Err(FilesError::UnknownBranch),
        };
        let base = main_branch(&branches).filter(|main| branch.as_deref() != Some(*main));

        // A repository without a commit has a branch and no history.
        let Some(tip) = commit(&self.git, &name).await? else {
            return Ok(History {
                branches,
                branch,
                base: None,
                ahead: 0,
                more: false,
                commits: Vec::new(),
            });
        };

        let limit = format!("--max-count={}", LISTED + 1);
        let listing = [
            "log",
            "-z",
            "--topo-order",
            "--name-only",
            "--no-renames",
            "--diff-merges=first-parent",
            &limit,
            LOG_FORMAT,
            &tip,
            "--",
        ];
        let (listed, own) = try_join!(self.git.output(&listing), self.own_commits(&tip, base))?;

        let mut commits: Vec<Commit> = logged(&listed)
            .into_iter()
            .map(|logged| Commit {
                on_base: base.is_some() && !own.contains(&logged.hash),
                hash: logged.hash,
                parents: logged.parents,
                subject: logged.subject,
                author: logged.author,
                email: logged.email,
                time: logged.time,
                thread: None,
                agent: None,
                files: logged.files,
            })
            .collect();
        let more = commits.len() > LISTED;
        commits.truncate(LISTED);
        // Keeps git's order within each of the two groups.
        commits.sort_by_key(|commit| commit.on_base);

        Ok(History {
            branches,
            branch,
            base: base.map(String::from),
            ahead: own.len(),
            more,
            commits,
        })
    }

    /// The files a commit changed. With `from`, what changed between that commit and this one.
    pub async fn changes(&self, name: &str, from: Option<&str>) -> Result<Changes, FilesError> {
        let (commit, from, before) = self.compared(name, from).await?;
        let naming = [
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            &before,
            &commit,
            "--",
        ];
        let counting = [
            "diff",
            "--numstat",
            "--no-renames",
            "-z",
            &before,
            &commit,
            "--",
        ];
        let message = ["log", "-1", "--format=%b", &commit, "--"];
        let (named, counted, body) = try_join!(
            self.git.output(&naming),
            self.git.output(&counting),
            self.git.output(&message),
        )?;

        Ok(Changes {
            files: changed_files(changes(&named), &counts(&counted)),
            body: String::from_utf8_lossy(&body).trim().to_string(),
            commit,
            from,
        })
    }

    /// One file a commit changed, line by line, next to the version before.
    pub async fn change(
        &self,
        name: &str,
        from: Option<&str>,
        path: &str,
    ) -> Result<FileView, FilesError> {
        let path = clean_path(path).ok_or(FilesError::UnknownFile)?;
        let (commit, _, before) = self.compared(name, from).await?;
        let (earlier, later) = try_join!(
            self.committed_file(&before, &path),
            self.committed_file(&commit, &path),
        )?;

        Ok(FileView {
            content: present(&earlier, &later)?,
            path,
        })
    }

    /// A commit, the commit it is compared with, and the tree that stands for the latter.
    ///
    /// Without `from` a commit is compared with the one it continues. A
    /// commit without a parent is compared with a tree that holds no file.
    async fn compared(
        &self,
        name: &str,
        from: Option<&str>,
    ) -> Result<(String, Option<String>, String), FilesError> {
        let commit_name = self.named_commit(name).await?;
        let from = match from {
            Some(from) => Some(self.named_commit(from).await?),
            None => commit(&self.git, &format!("{commit_name}^1")).await?,
        };
        let before = match &from {
            Some(from) => from.clone(),
            None => self.empty_tree().await?,
        };
        Ok((commit_name, from, before))
    }

    /// The commits a branch has that the main branch has not.
    ///
    /// Empty without a main branch to tell them apart from.
    async fn own_commits(
        &self,
        tip: &str,
        base: Option<&str>,
    ) -> Result<BTreeSet<String>, GitError> {
        let Some(base) = base else {
            return Ok(BTreeSet::new());
        };
        let main = format!("refs/heads/{base}");
        let listed = self
            .git
            .output(&["rev-list", tip, "--not", &main, "--"])
            .await?;
        Ok(lines(&listed).into_iter().collect())
    }
}
