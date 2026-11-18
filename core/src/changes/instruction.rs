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
//! The instruction is handed to the program beside the person's message,
//! never as part of it.

use std::path::Path;

use crate::changes::Change;
use crate::files::git::{first_line, Git};

/// The instruction for a thread that has no change yet.
pub fn starting() -> String {
    concat!(
        "If the work will change files, first run `teryon worktree create <name>`, where `<name>` is a short name for the work ",
        "in lowercase words joined by dashes, such as `fix-login-redirect`. ",
        "It creates a git worktree on a new branch with that name and prints its path. Change files only there. ",
        "When the message names a branch to start from, add `--from <branch>`. ",
        "A question that changes nothing needs no worktree. ",
        "Do not create worktrees any other way, and leave this directory on its current branch."
    )
    .to_string()
}

/// The instruction for a thread whose agent already has a change.
pub fn continuing(path: &Path, branch: &str) -> String {
    format!(
        "This thread's worktree is `{}`, on the branch `{branch}`. Change files only there. Do not create another worktree.",
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
        }

        expect(continuing(Path::new("/work/app/.teryon/worktrees/fix-login"), branch)) {
            let branch = "fix-login";

            to name_the_path_and_the_branch {
                equal("This thread's worktree is `/work/app/.teryon/worktrees/fix-login`, on the branch `fix-login`. Change files only there. Do not create another worktree.".to_string())
            }
            when(branch = "other") as when_the_agent_switched_branches to name_the_branch_checked_out {
                have(contains("on the branch `other`")) be_true
            }
        }
    }
}
