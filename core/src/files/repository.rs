//! Reads a project's git repository for the file browser.
//!
//! A branch is shown from the directory it is checked out in when there is
//! one, because that is where an agent's work shows before it is committed.
//! Agents usually work in a directory of their own, so those directories
//! count too. A branch checked out nowhere is shown as it was committed.
//!
//! Changes are counted from the point where the branch left the main branch.
//! For the main branch itself that point is its own last commit, which leaves
//! exactly the work that is not committed yet.
//!
//! A commit can be shown in place of a branch, which is how the history
//! screen lets a person browse the files as they were. It is read as it was
//! committed and compared with the main branch the same way.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::str::from_utf8;

use tokio::fs::{canonicalize, metadata, read};
use tokio::try_join;

use crate::directory::existing_directory;
use crate::files::diff::compare;
use crate::files::git::{changes, checkouts, first_line, lines, zero_separated, Git};
use crate::files::{Content, FileChange, FileEntry, FileView, FilesError, Listing};

/// The largest file shown, in bytes.
const LARGEST_SHOWN: u64 = 1024 * 1024;

/// The names a repository's main branch goes by, most likely first.
const MAIN_BRANCHES: [&str; 2] = ["main", "master"];

/// A project's repository.
pub struct Repository {
    pub(crate) git: Git,
}

/// What the file browser is asked to show.
#[derive(Clone, Copy, Debug)]
pub enum Place<'a> {
    /// The project's directory as it is.
    Directory,
    Branch(&'a str),
    /// A commit, by its name in hexadecimal digits, whole or shortened.
    Commit(&'a str),
}

impl<'a> Place<'a> {
    /// The place two optional parts of a request name. A commit wins over a branch.
    pub fn named(branch: Option<&'a str>, commit: Option<&'a str>) -> Self {
        match (commit, branch) {
            (Some(commit), _) => Self::Commit(commit),
            (None, Some(branch)) => Self::Branch(branch),
            (None, None) => Self::Directory,
        }
    }
}

/// Where the files of a branch are read from.
enum Tree {
    /// A directory the branch is checked out in.
    Working(Git),
    /// The branch as it was last committed, by its full name.
    Committed(String),
}

/// A branch or a commit chosen for showing, and what it is compared with.
struct View {
    branches: Vec<String>,
    branch: Option<String>,
    commit: Option<String>,
    tree: Tree,
    /// The commit, or the empty tree, that changes are counted from.
    base: String,
    base_branch: Option<String>,
}

/// A version of a file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Stored {
    Absent,
    TooLarge,
    Bytes(Vec<u8>),
}

impl Repository {
    /// Opens the repository whose top is `directory`.
    pub async fn open(directory: &Path) -> Result<Self, FilesError> {
        let git = Git::at(directory);
        match git.attempt(&["rev-parse", "--show-toplevel"]).await? {
            Some(_) => Ok(Self { git }),
            None => Err(FilesError::NotARepository),
        }
    }

    /// Every file of a place.
    pub async fn listing(&self, place: Place<'_>) -> Result<Listing, FilesError> {
        let view = self.view(place).await?;
        let files = match &view.tree {
            Tree::Working(git) => {
                let differing = [
                    "diff",
                    "--name-status",
                    "--no-renames",
                    "-z",
                    &view.base,
                    "--",
                ];
                let (tracked, untracked, missing, changed) = try_join!(
                    git.output(&["ls-files", "-z", "--cached"]),
                    git.output(&["ls-files", "-z", "--others", "--exclude-standard"]),
                    git.output(&["ls-files", "-z", "--deleted"]),
                    git.output(&differing),
                )?;
                gather(
                    zero_separated(&tracked),
                    zero_separated(&untracked),
                    zero_separated(&missing),
                    changes(&changed),
                )
            }
            Tree::Committed(revision) => {
                let listing = ["ls-tree", "-r", "-z", "--name-only", revision];
                let differing = [
                    "diff",
                    "--name-status",
                    "--no-renames",
                    "-z",
                    &view.base,
                    revision,
                    "--",
                ];
                let (tracked, changed) =
                    try_join!(self.git.output(&listing), self.git.output(&differing))?;
                gather(
                    zero_separated(&tracked),
                    Vec::new(),
                    Vec::new(),
                    changes(&changed),
                )
            }
        };

        Ok(Listing {
            branches: view.branches,
            branch: view.branch,
            commit: view.commit,
            base: view.base_branch,
            files,
        })
    }

    /// One file of a place, line by line, next to the version it started from.
    pub async fn file(&self, place: Place<'_>, path: &str) -> Result<FileView, FilesError> {
        let path = clean_path(path).ok_or(FilesError::UnknownFile)?;
        let view = self.view(place).await?;

        let current = match &view.tree {
            Tree::Working(git) => working_file(git, &path).await?,
            Tree::Committed(revision) => self.committed_file(revision, &path).await?,
        };
        let base = self.committed_file(&view.base, &path).await?;

        Ok(FileView {
            path,
            content: present(&base, &current)?,
        })
    }

    /// The branches there are, and the one the project's directory is on.
    pub(crate) async fn branches(&self) -> Result<(Vec<String>, Option<String>), FilesError> {
        let mut branches = lines(
            &self
                .git
                .output(&["for-each-ref", "--format=%(refname:lstrip=2)", "refs/heads"])
                .await?,
        );
        let current = self
            .git
            .attempt(&["symbolic-ref", "--short", "-q", "HEAD"])
            .await?
            .map(|printed| first_line(&printed));
        // A repository without a commit is on a branch that does not exist yet.
        if let Some(current) = &current {
            if !branches.contains(current) {
                branches.push(current.clone());
            }
        }
        Ok((branches, current))
    }

    /// The full name of the commit a person named, or a refusal when there is none.
    ///
    /// Only hexadecimal digits are taken for a name, so nothing a person
    /// sends can be read by git as an option or as some other way of naming
    /// a commit.
    pub(crate) async fn named_commit(&self, name: &str) -> Result<String, FilesError> {
        if !is_commit_name(name) {
            return Err(FilesError::UnknownCommit);
        }
        commit(&self.git, name)
            .await?
            .ok_or(FilesError::UnknownCommit)
    }

    /// The tree without a single file, which a first commit is compared with.
    pub(crate) async fn empty_tree(&self) -> Result<String, FilesError> {
        let nothing = self
            .git
            .output(&["hash-object", "-t", "tree", "/dev/null"])
            .await?;
        Ok(first_line(&nothing))
    }

    async fn view(&self, place: Place<'_>) -> Result<View, FilesError> {
        let (branches, current) = self.branches().await?;

        let (branch, shown_commit, tree) = match place {
            Place::Directory => (current, None, Tree::Working(self.git.clone())),
            Place::Branch(name) if current.as_deref() == Some(name) => {
                (current, None, Tree::Working(self.git.clone()))
            }
            Place::Branch(name) if branches.iter().any(|branch| branch == name) => {
                let tree = match self.checkout_of(name).await? {
                    Some(directory) => Tree::Working(Git::at(directory)),
                    None => Tree::Committed(format!("refs/heads/{name}")),
                };
                (Some(name.to_string()), None, tree)
            }
            Place::Branch(_) => return Err(FilesError::UnknownBranch),
            Place::Commit(name) => {
                let commit = self.named_commit(name).await?;
                (None, Some(commit.clone()), Tree::Committed(commit))
            }
        };

        let tip = match &tree {
            Tree::Working(git) => commit(git, "HEAD").await?,
            Tree::Committed(revision) => commit(&self.git, revision).await?,
        };
        let (base, base_branch) = match tip {
            None => (self.empty_tree().await?, None),
            Some(tip) => match self.fork_point(&branches, branch.as_deref(), &tip).await? {
                Some((fork, main)) => (fork, Some(main)),
                None => (tip, None),
            },
        };

        Ok(View {
            branches,
            branch,
            commit: shown_commit,
            tree,
            base,
            base_branch,
        })
    }

    /// Where a branch left the main branch, and the main branch's name.
    ///
    /// Nothing for the main branch itself, for a repository without one, and
    /// for a branch that shares no history with it.
    async fn fork_point(
        &self,
        branches: &[String],
        branch: Option<&str>,
        tip: &str,
    ) -> Result<Option<(String, String)>, FilesError> {
        let Some(main) = main_branch(branches) else {
            return Ok(None);
        };
        if branch == Some(main) {
            return Ok(None);
        }
        let fork = self
            .git
            .attempt(&["merge-base", &format!("refs/heads/{main}"), tip])
            .await?;
        Ok(fork.map(|printed| (first_line(&printed), main.to_string())))
    }

    /// The directory a branch is checked out in, when it is still there.
    async fn checkout_of(&self, branch: &str) -> Result<Option<PathBuf>, FilesError> {
        let listed = self
            .git
            .output(&["worktree", "list", "--porcelain", "-z"])
            .await?;
        Ok(checkouts(&listed)
            .into_iter()
            .filter(|checkout| checkout.branch.as_deref() == Some(branch))
            .find_map(|checkout| existing_directory(&checkout.directory.to_string_lossy()).ok()))
    }

    /// A file as a commit has it. `revision` is a commit, a branch or a tree.
    pub(crate) async fn committed_file(
        &self,
        revision: &str,
        path: &str,
    ) -> Result<Stored, FilesError> {
        let object = format!("{revision}:{path}");
        let Some(size) = self.git.attempt(&["cat-file", "-s", &object]).await? else {
            return Ok(Stored::Absent);
        };
        if first_line(&size).parse::<u64>().unwrap_or(u64::MAX) > LARGEST_SHOWN {
            return Ok(Stored::TooLarge);
        }
        // A folder has a size too, but only a file can be read as one.
        Ok(self
            .git
            .attempt(&["cat-file", "blob", &object])
            .await?
            .map_or(Stored::Absent, Stored::Bytes))
    }
}

/// The name a repository's main branch goes by, or nothing when it has none.
pub(crate) fn main_branch(branches: &[String]) -> Option<&'static str> {
    MAIN_BRANCHES
        .into_iter()
        .find(|main| branches.iter().any(|branch| branch == main))
}

/// Whether a text can only be the name of a commit: hexadecimal digits, whole or shortened.
fn is_commit_name(name: &str) -> bool {
    (4..=64).contains(&name.len()) && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The commit a name stands for, or nothing when there is no commit yet.
pub(crate) async fn commit(git: &Git, name: &str) -> Result<Option<String>, FilesError> {
    Ok(git
        .attempt(&["rev-parse", "--verify", "-q", &format!("{name}^{{commit}}")])
        .await?
        .map(|printed| first_line(&printed)))
}

/// A file as it is on disk in a checked out directory.
///
/// The file is found through its real location, so a link cannot lead out of
/// the directory or into the repository's own data. A file git ignores is
/// treated as not there: such files are left out of the listing, and often
/// hold secrets.
async fn working_file(git: &Git, path: &str) -> Result<Stored, FilesError> {
    let Ok(top) = canonicalize(git.directory()).await else {
        return Ok(Stored::Absent);
    };
    let Ok(real) = canonicalize(top.join(path)).await else {
        return Ok(Stored::Absent);
    };
    let Some(inside) = real
        .strip_prefix(&top)
        .ok()
        .and_then(Path::to_str)
        .and_then(clean_path)
    else {
        return Ok(Stored::Absent);
    };
    if git
        .attempt(&["check-ignore", "-q", "--", &inside])
        .await?
        .is_some()
    {
        return Ok(Stored::Absent);
    }

    match metadata(&real).await {
        Ok(details) if !details.is_file() => Ok(Stored::Absent),
        Ok(details) if details.len() > LARGEST_SHOWN => Ok(Stored::TooLarge),
        Ok(_) => Ok(read(&real).await.map_or(Stored::Absent, Stored::Bytes)),
        Err(_) => Ok(Stored::Absent),
    }
}

/// A path as git names it, or nothing when it could point somewhere else.
///
/// Refused are an empty path, one that starts at the top of the machine, one
/// with `.` or `..` in it, and anything inside the repository's own `.git`.
pub(crate) fn clean_path(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            _ => return None,
        }
    }
    let own_data = parts.first()?.eq_ignore_ascii_case(".git");
    let unchanged = parts.join("/") == path;
    (!own_data && unchanged && !path.contains('\0')).then(|| path.to_string())
}

/// Puts what git listed together into the files of a branch, sorted by path.
///
/// `tracked` and `untracked` are the files there are, `missing` those of them
/// that are gone from disk, and `changed` everything that differs from the
/// version compared with. A deleted file is in `changed` only, and is listed
/// so that what was removed can be looked at.
fn gather(
    tracked: Vec<String>,
    untracked: Vec<String>,
    missing: Vec<String>,
    changed: Vec<(String, FileChange)>,
) -> Vec<FileEntry> {
    let missing: BTreeSet<String> = missing.into_iter().collect();
    let mut files: BTreeMap<String, Option<FileChange>> = tracked
        .into_iter()
        .filter(|path| !missing.contains(path))
        .map(|path| (path, None))
        .collect();
    files.extend(
        untracked
            .into_iter()
            .map(|path| (path, Some(FileChange::Added))),
    );
    files.extend(
        changed
            .into_iter()
            .map(|(path, change)| (path, Some(change))),
    );

    files
        .into_iter()
        .map(|(path, change)| FileEntry { path, change })
        .collect()
}

/// Lays two versions of a file out as what the screen shows.
pub(crate) fn present(base: &Stored, current: &Stored) -> Result<Content, FilesError> {
    match (base, current) {
        (Stored::Absent, Stored::Absent) => Err(FilesError::UnknownFile),
        (Stored::TooLarge, _) | (_, Stored::TooLarge) => Ok(Content::TooLarge),
        _ => Ok(match (text_of(base), text_of(current)) {
            (Some(base), Some(current)) => Content::Text {
                lines: compare(base, current),
            },
            _ => Content::Binary,
        }),
    }
}

/// A version as text, or nothing when it is not text. A missing version is empty.
fn text_of(stored: &Stored) -> Option<&str> {
    match stored {
        Stored::Bytes(bytes) if bytes.contains(&0) => None,
        Stored::Bytes(bytes) => from_utf8(bytes).ok(),
        _ => Some(""),
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;
    use crate::files::diff::{Line, LineChange};

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    fn entry(path: &str, change: Option<FileChange>) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            change,
        }
    }

    fn bytes(text: &str) -> Stored {
        Stored::Bytes(text.as_bytes().to_vec())
    }

    fn line(change: Option<LineChange>, number: Option<usize>, text: &str) -> Line {
        Line {
            change,
            number,
            text: text.to_string(),
        }
    }

    fn text(lines: Vec<Line>) -> Result<Content, String> {
        Ok(Content::Text { lines })
    }

    /// The content of a file, or the name of the refusal.
    fn shown(base: Stored, current: Stored) -> Result<Content, String> {
        present(&base, &current).map_err(|problem| format!("{problem:?}"))
    }

    lets_expect! {
        expect(Place::named(branch, commit)) {
            when(branch = None, commit = None) as nothing_named to be_the_directory { match_pattern!(Place::Directory) }
            when(branch = Some("main"), commit = None) as a_branch to be_that_branch { match_pattern!(Place::Branch("main")) }
            when(branch = None, commit = Some("a3f9c21")) as a_commit to be_that_commit { match_pattern!(Place::Commit("a3f9c21")) }
            when(branch = Some("main"), commit = Some("a3f9c21")) as both to be_the_commit { match_pattern!(Place::Commit("a3f9c21")) }
        }

        expect(is_commit_name(name)) {
            when(name = "a3f9c21") as a_shortened_name to be_taken { equal(true) }
            when(name = "08fb0ad08ebe07fb6fa5f3f750649b2be9b21e31") as a_whole_name to be_taken { equal(true) }
            when(name = "A3F9C21") as capitals to be_taken { equal(true) }
            when(name = "a3f") as too_few_digits to be_refused { equal(false) }
            when(name = "00000000000000000000000000000000000000000000000000000000000000000") as too_many_digits to be_refused { equal(false) }
            when(name = "") as nothing to be_refused { equal(false) }
            when(name = "main") as a_branch_name to be_refused { equal(false) }
            when(name = "--all") as an_option to be_refused { equal(false) }
            when(name = "a3f9c21^") as a_way_to_a_parent to be_refused { equal(false) }
            when(name = "a3f9c21..main") as a_range to be_refused { equal(false) }
            when(name = "a3f9 c21") as a_space to be_refused { equal(false) }
        }

        expect(clean_path(path)) {
            when(path = "README.md") as a_file_at_the_top to be_kept { equal(Some("README.md".to_string())) }
            when(path = "src/ledger/adapter.ts") as a_file_in_folders to be_kept { equal(Some("src/ledger/adapter.ts".to_string())) }
            when(path = "notes/my file \u{17c}.txt") as an_unusual_name to be_kept { equal(Some("notes/my file \u{17c}.txt".to_string())) }
            when(path = ".github/workflows/ci.yml") as a_hidden_folder to be_kept { equal(Some(".github/workflows/ci.yml".to_string())) }
            when(path = ".gitignore") as a_name_that_starts_like_the_repository_data to be_kept { equal(Some(".gitignore".to_string())) }
            when(path = "") as an_empty_path to be_refused { equal(None) }
            when(path = "/etc/passwd") as a_path_from_the_top_of_the_machine to be_refused { equal(None) }
            when(path = "../outside.txt") as a_path_leading_up to be_refused { equal(None) }
            when(path = "src/../../outside.txt") as dots_in_the_middle to be_refused { equal(None) }
            when(path = "./README.md") as a_leading_dot to be_refused { equal(None) }
            when(path = "src//adapter.ts") as a_doubled_separator to be_refused { equal(None) }
            when(path = "src/") as a_trailing_separator to be_refused { equal(None) }
            when(path = ".git/config") as the_repository_data to be_refused { equal(None) }
            when(path = ".GIT/config") as the_repository_data_in_capitals to be_refused { equal(None) }
            when(path = "src/a\0b") as a_zero_byte to be_refused { equal(None) }
        }

        expect(gather(tracked, untracked, missing, changed)) {
            let tracked = paths(&["b.txt", "a.txt"]);
            let untracked = Vec::new();
            let missing = Vec::new();
            let changed = Vec::new();

            when nothing_changed to list_the_files_sorted {
                equal(vec![entry("a.txt", None), entry("b.txt", None)])
            }
            when a_file_was_modified {
                let changed = vec![("a.txt".to_string(), FileChange::Modified)];
                to mark_it { equal(vec![entry("a.txt", Some(FileChange::Modified)), entry("b.txt", None)]) }
            }
            when a_file_is_not_tracked_yet {
                let untracked = paths(&["new.txt"]);
                to list_it_as_added {
                    equal(vec![entry("a.txt", None), entry("b.txt", None), entry("new.txt", Some(FileChange::Added))])
                }
            }
            when a_file_was_deleted_and_the_deletion_recorded {
                let tracked = paths(&["b.txt"]);
                let changed = vec![("a.txt".to_string(), FileChange::Deleted)];
                to still_list_it { equal(vec![entry("a.txt", Some(FileChange::Deleted)), entry("b.txt", None)]) }
            }
            when a_file_was_deleted_from_disk_only {
                let missing = paths(&["a.txt"]);
                let changed = vec![("a.txt".to_string(), FileChange::Deleted)];
                to list_it_as_deleted { equal(vec![entry("a.txt", Some(FileChange::Deleted)), entry("b.txt", None)]) }
            }
            when a_file_new_on_the_branch_is_gone_from_disk {
                let missing = paths(&["a.txt"]);
                to leave_it_out { equal(vec![entry("b.txt", None)]) }
            }
            when a_file_is_listed_twice {
                let tracked = paths(&["a.txt", "a.txt"]);
                to list_it_once { equal(vec![entry("a.txt", None)]) }
            }
            when there_are_no_files {
                let tracked = Vec::new();
                to be_empty { equal(Vec::new()) }
            }
        }

        expect(shown(base, current)) {
            when(base = bytes("one\n"), current = bytes("one\ntwo\n")) as a_changed_text_file to be_laid_out_in_lines {
                equal(text(vec![
                    line(None, Some(1), "one"),
                    line(Some(LineChange::Added), Some(2), "two"),
                ]))
            }
            when(base = Stored::Absent, current = bytes("one\n")) as a_new_file to be_all_added {
                equal(text(vec![line(Some(LineChange::Added), Some(1), "one")]))
            }
            when(base = bytes("one\n"), current = Stored::Absent) as a_deleted_file to be_all_removed {
                equal(text(vec![line(Some(LineChange::Removed), None, "one")]))
            }
            when(base = Stored::Absent, current = bytes("")) as a_new_empty_file to have_no_lines {
                equal(text(Vec::new()))
            }
            when(base = Stored::Absent, current = Stored::Absent) as a_file_in_neither_version to be_unknown {
                equal(Err("UnknownFile".to_string()))
            }
            when(base = bytes("one\n"), current = Stored::Bytes(vec![137, 80, 78, 71, 0])) as a_file_with_a_zero_byte to be_binary {
                equal(Ok(Content::Binary))
            }
            when(base = Stored::Absent, current = Stored::Bytes(vec![255, 254, 253])) as bytes_that_are_not_text to be_binary {
                equal(Ok(Content::Binary))
            }
            when(base = Stored::Bytes(vec![0]), current = bytes("text now\n")) as a_file_that_used_to_be_binary to be_binary {
                equal(Ok(Content::Binary))
            }
            when(base = bytes("one\n"), current = Stored::TooLarge) as a_file_that_grew_too_large to be_too_large {
                equal(Ok(Content::TooLarge))
            }
            when(base = Stored::TooLarge, current = bytes("one\n")) as a_file_that_used_to_be_too_large to be_too_large {
                equal(Ok(Content::TooLarge))
            }
            when(base = Stored::TooLarge, current = Stored::Absent) as a_deleted_large_file to be_too_large {
                equal(Ok(Content::TooLarge))
            }
        }
    }
}
