//! A second agent reviewing a thread's change, without changing it.
//!
//! A review is only worth having when it is independent: the agent that
//! wrote the code is attached to its own decisions. So a different agent
//! reads the change with a fresh conversation, in the same thread, and
//! reports what it finds. The person decides what goes back to the author.
//!
//! Like the instruction for authors, what the reviewer is told speaks git.

use std::path::Path;
use std::time::UNIX_EPOCH;

use tokio::fs::{read_link, symlink_metadata};

use crate::changes::Change;
use crate::files::git::Git;

/// What the reviewer is asked when a review starts.
pub const REVIEW_FIRST: &str = "Review the change.";

/// What the reviewer is asked when it reviewed before.
pub const REVIEW_AGAIN: &str = "The author has worked on the change since your last review. Review it again: check whether what you found was addressed, and look for anything new.";

/// The words the reviewer ends a review with, before the number of problems.
const FINDINGS: &str = "Findings:";

/// The instruction a reviewer gets beside every message, its first or the person's.
pub fn reviewing(change: &Change) -> String {
    let branch = change.branch.as_deref().unwrap_or(&change.name);
    format!(
        concat!(
            "You review work another agent did in the git worktree `{path}`, on the branch `{branch}`. ",
            "It started from commit `{start}`. The change is `git diff {start}` run in the worktree, which covers files git tracks, committed or not, ",
            "together with the new files `git status --untracked-files=all` lists as untracked, which `git diff` leaves out: read those whole. ",
            "Do not create, change or delete any file, here or anywhere else, temporary folders included, and do not commit, stash or switch branches. ",
            "Run only commands that read, such as `git diff`, `git log`, `git status` and the project's own tests. ",
            "Nobody can approve a command for you, so a command that needs approval stops the review: when unsure, read the code instead of running something. ",
            "When asked for a review, report the problems that matter: bugs, cases left out, risky choices, missing tests. ",
            "Number them, most serious first, each with the file and line and why it matters. Leave out matters of taste. ",
            "End a review with a last line that is exactly `{findings} N`, where N is how many problems you reported, 0 when there are none. ",
            "When the person asks you something else, answer it without that line."
        ),
        path = change.path.display(),
        branch = branch,
        start = change.start,
        findings = FINDINGS,
    )
}

/// How many problems a review counted, from its last line. Nothing when it did not say.
pub fn findings_in(review: &str) -> Option<u32> {
    let last = review.lines().rev().find(|line| !line.trim().is_empty())?;
    let count = last
        .trim()
        .trim_matches(|character| character == '*' || character == '`')
        .strip_prefix(FINDINGS)?;
    count.trim().parse().ok()
}

/// Everything about a copy a reviewer could change: what is checked out, every tracked file that differs from it, and when every untracked file last changed.
///
/// Compared before and after a reviewer's turn, to catch one that edited
/// files despite being told not to. An agent's new files stay untracked
/// until it commits them, and `git diff` leaves those out, so each one's
/// size and modification time are noted, see [`stamp`]. Ignored files are left out: builds and tests
/// write there as a matter of course. Nothing when git could not say.
pub async fn fingerprint(change: &Change) -> Option<Vec<u8>> {
    let git = Git::at(&change.path);
    let mut print = git.output(&["rev-parse", "HEAD"]).await.ok()?;
    print.extend(
        git.output(&["status", "--porcelain=v1", "--untracked-files=all"])
            .await
            .ok()?,
    );
    print.extend(git.output(&["diff", "HEAD", "--binary"]).await.ok()?);
    let untracked = git
        .output(&["ls-files", "--others", "--exclude-standard", "-z"])
        .await
        .ok()?;
    for path in untracked
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        print.extend_from_slice(path);
        print.push(0);
        let file = change.path.join(String::from_utf8_lossy(path).as_ref());
        print.extend(stamp(&file).await);
        print.push(0);
    }
    Some(print)
}

/// A file's size and modification time, or where a link points, read without opening it.
///
/// Reading untracked files whole would hold the turn up for a large file,
/// and never end for a link to a device with no end. Writing a file
/// changes its modification time, which is what the check needs.
async fn stamp(file: &Path) -> Vec<u8> {
    let Ok(metadata) = symlink_metadata(file).await else {
        return Vec::new();
    };
    if metadata.file_type().is_symlink() {
        let target = read_link(file).await.unwrap_or_default();
        return target.as_os_str().as_encoded_bytes().to_vec();
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    format!("{} {modified}", metadata.len()).into_bytes()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lets_expect::lets_expect;

    use super::*;

    fn change() -> Change {
        Change {
            name: "fix-login".to_string(),
            path: PathBuf::from("/work/app/.teryon/worktrees/fix-login"),
            base: Some("main".to_string()),
            start: "abc123".to_string(),
            branch: Some("fix-login".to_string()),
            state: None,
            review: None,
        }
    }

    lets_expect! {
        expect(reviewing(&change())) {
            to name_the_worktree_and_the_branch { have(contains("`/work/app/.teryon/worktrees/fix-login`, on the branch `fix-login`")) be_true }
            to say_how_to_see_the_change { have(contains("`git diff abc123`")) be_true }
            // requirement: the reviewer is told that new files are part of the change though `git diff` leaves them out
            to include_new_files { have(contains("`git status --untracked-files=all` lists as untracked, which `git diff` leaves out")) be_true }
            to forbid_changing_files { have(contains("Do not create, change or delete any file, here or anywhere else, temporary folders included")) be_true }
            to keep_to_reading_commands { have(contains("Run only commands that read")) be_true }
            to warn_that_nobody_approves { have(contains("Nobody can approve a command for you")) be_true }
            to ask_for_the_count { have(contains("exactly `Findings: N`")) be_true }
        }

        // requirement: the number of problems is read from the review's last line, which the reviewer is asked for
        expect(findings_in(review)) {
            when(review = "1. A bug.\n2. Another.\n\nFindings: 2") as a_count to be_read { equal(Some(2)) }
            when(review = "Looks good.\nFindings: 0\n\n") as no_problems_and_a_trailing_blank to be_zero { equal(Some(0)) }
            when(review = "**Findings: 3**") as a_bold_line to be_read { equal(Some(3)) }
            when(review = "`Findings: 1`") as a_line_in_code to be_read { equal(Some(1)) }
            when(review = "Findings: 2\nThat is all.") as a_count_not_last to be_unknown { equal(None) }
            when(review = "Findings: some") as a_word_in_place_of_a_number to be_unknown { equal(None) }
            when(review = "It looks fine.") as no_count to be_unknown { equal(None) }
            when(review = "") as an_empty_review to be_unknown { equal(None) }
        }
    }
}
