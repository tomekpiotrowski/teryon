//! What a thread's agent is told about where to change files.
//!
//! Agents know git, so this speaks git, unlike anything the person reads.
//!
//! A thread without a change is told to start one through `teryon worktree
//! create`, which lets the agent choose a name a person can recognise while
//! Teryon decides where the copy goes and remembers which thread it belongs
//! to. A thread with a change is told where it is, so the next turn carries
//! on there instead of starting another.
//!
//! Either way the agent is told to commit its work with a message that
//! describes the whole change. Teryon offers the newest commit's message as
//! the starting point for the change's description when the person applies
//! it, and only the agent knows what it did; without a commit, all Teryon
//! has is the person's first message, which says what was asked, not done.
//!
//! The instruction is handed to the program beside the person's message,
//! never as part of it.

use std::path::Path;

use crate::changes::Change;
use crate::files::git::{first_line, Git};

/// What every agent that changes files is told about committing its work.
const COMMITTING: &str = concat!(
    "When you have changed files, keep all of your work on the branch in a single commit in the worktree before you finish, ",
    "with nothing left uncommitted or untracked. ",
    "On later turns, amend that commit and update its message rather than adding new commits. ",
    "Write the message for someone reading the project's history: a short first line saying what the change does, ",
    "then, when it helps, a few lines on why. Describe the whole change on the branch so far, not only this turn's request, ",
    "because that commit's message becomes the description of the change when it is applied."
);

/// The instruction for a thread that has no change yet.
pub fn starting() -> String {
    concat!(
        "If the work will change files, first run `teryon worktree create <name>`, where `<name>` is a short name for the work ",
        "in lowercase words joined by dashes, such as `fix-login-redirect`. ",
        "It creates a git worktree on a new branch with that name and prints its path. Change files only there. ",
        "When the message names a branch to start from, add `--from <branch>`. ",
        "A question that changes nothing needs no worktree. ",
        "Do not create worktrees any other way, and leave this directory on its current branch. "
    )
    .to_string()
        + COMMITTING
}

/// The instruction for a thread whose agent already has a change.
pub fn continuing(path: &Path, branch: &str) -> String {
    format!(
        "This thread's worktree is `{}`, on the branch `{branch}`. Change files only there. Do not create another worktree. {COMMITTING}",
        path.display()
    )
}

/// The instruction when Teryon starts an agent to resolve an overlap before applying automatically.
///
/// The agent rebases its branch onto the latest base and folds everything back into one commit,
/// leaving the thread's stage untouched so Teryon can apply the change as soon as the turn ends.
pub fn rebasing(path: &Path, branch: &str, base: &str) -> String {
    format!(
        "Teryon is about to apply this change to `{base}`, but `{branch}` overlaps with newer work on `{base}`. \
        In the worktree `{}`, rebase `{branch}` onto `{base}`, resolve all conflicts, and run relevant tests to check the result. \
        Keep all of your changes on `{branch}` in a single commit with nothing left uncommitted or untracked, and update the commit message to cover the whole change. \
        Do not change the thread's stage: Teryon will apply the change automatically once your turn ends.",
        path.display()
    )
}

/// The instruction for one turn of a thread, in `directory`.
///
/// Nothing when `directory` is not a git repository: there is nowhere to make
/// a copy, and a plain folder is left untouched.
pub async fn instruction_for(directory: &Path, change: Option<&Change>) -> Option<String> {
    let top = Git::at(directory)
        .attempt(&["rev-parse", "--show-toplevel"])
        .await
        .ok()??;
    if first_line(&top).is_empty() {
        return None;
    }
    Some(match change {
        Some(change) => continuing(
            &change.path,
            change.branch.as_deref().unwrap_or(&change.name),
        ),
        None => starting(),
    })
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        expect(starting()) {
            to name_the_command { have(contains("`teryon worktree create <name>`")) be_true }
            to give_an_example_name { have(contains("`fix-login-redirect`")) be_true }
            to say_how_to_start_elsewhere { have(contains("`--from <branch>`")) be_true }
            to let_questions_go_without { have(contains("A question that changes nothing needs no worktree.")) be_true }
            to forbid_other_worktrees { have(contains("Do not create worktrees any other way")) be_true }
            to ask_for_a_commit { have(ends_with(COMMITTING)) be_true }
            to ask_for_a_single_commit { have(contains("single commit")) be_true }
            to ask_to_amend_later { have(contains("amend that commit")) be_true }
            to forbid_leftovers { have(contains("nothing left uncommitted or untracked")) be_true }
        }

        expect(continuing(Path::new("/work/app/.teryon/worktrees/fix-login"), branch)) {
            let branch = "fix-login";

            to name_the_path_and_the_branch {
                equal(format!("This thread's worktree is `/work/app/.teryon/worktrees/fix-login`, on the branch `fix-login`. Change files only there. Do not create another worktree. {COMMITTING}"))
            }
            when(branch = "other") as when_the_agent_switched_branches to name_the_branch_checked_out {
                have(contains("on the branch `other`")) be_true
            }
        }

        expect(rebasing(Path::new("/work/app/.teryon/worktrees/fix-login"), "fix-login", "main")) {
            to name_the_worktree { have(contains("`/work/app/.teryon/worktrees/fix-login`")) be_true }
            to ask_to_rebase_onto_base { have(contains("rebase `fix-login` onto `main`")) be_true }
            to ask_to_resolve_conflicts { have(contains("resolve all conflicts")) be_true }
            to ask_to_run_tests { have(contains("run relevant tests")) be_true }
            to ask_for_a_single_commit { have(contains("single commit")) be_true }
            to forbid_leftovers { have(contains("nothing left uncommitted or untracked")) be_true }
            to ask_to_update_the_message { have(contains("update the commit message")) be_true }
            to forbid_stage_changes { have(contains("Do not change the thread's stage")) be_true }
        }
    }
}
