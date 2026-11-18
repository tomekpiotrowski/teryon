//! Finishing a change on GitHub: sending it for review, following the review, and tidying up once it is accepted.
//!
//! Most projects live on GitHub, where work goes in through a review rather
//! than straight into the project. Underneath, a review is a pull request:
//! the change's branch is pushed to `origin`, and GitHub's own `gh` program
//! opens the pull request and later says how it went. `gh` uses the sign-in
//! the person already has, so Teryon stores no GitHub credentials.
//!
//! A project is on GitHub when its `origin` points there. Such a project can
//! still have changes applied locally; sending for review is an addition.

use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command as BlockingCommand, Stdio};

use serde_json::Value;
use tokio::process::Command;
use tracing::error;

use crate::agents::runner::program_path;
use crate::changes::finish::{ready, remove, save_leftovers, FinishError};
use crate::changes::{Change, Review, ReviewState};
use crate::files::git::{checkouts, first_line, Git};

/// `gh`'s exit code when it needs the person to sign in.
const SIGNED_OUT: i32 = 4;

/// What becomes of a change once its review was accepted on GitHub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tidied {
    /// The project folder has the accepted change too.
    UpToDate,
    /// The project folder could not be brought up to date without touching
    /// the person's own edits, so it was left behind GitHub.
    Behind,
}

/// Whether a project's `origin` is on GitHub, read from its configuration as written.
///
/// Read once, when the project is opened, which is why this waits for git
/// rather than running alongside other work.
pub fn on_github(project: &Path) -> bool {
    BlockingCommand::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(project)
        .env(
            "GIT_CEILING_DIRECTORIES",
            project.parent().unwrap_or(project),
        )
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| is_github_url(&first_line(&output.stdout)))
}

/// Whether a remote's address is on GitHub, in any of the forms git takes.
pub fn is_github_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split_once('@').map_or(rest, |(_, rest)| rest);
    let host = rest.split([':', '/']).next().unwrap_or_default();
    host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("www.github.com")
}

/// Pushes `change` to `origin` and opens a review for it, described by `description`.
pub async fn send_for_review(
    project: &Path,
    change: &Change,
    description: &str,
    gh: &str,
) -> Result<Review, FinishError> {
    if change
        .review
        .as_ref()
        .is_some_and(|review| review.state == ReviewState::InReview)
    {
        return Err(FinishError::AlreadyInReview);
    }
    if !on_github(project) {
        return Err(FinishError::NotOnGitHub);
    }
    let (title, body) = split_description(description).ok_or(FinishError::NoDescription)?;
    ready(project, change).await?;
    let base = change.base.clone().ok_or(FinishError::BaseGone)?;

    let copy = Git::at(&change.path);
    save_leftovers(&copy).await?;
    if let Err(problem) = copy
        .output(&["push", "-q", "-u", "origin", &change.name])
        .await
    {
        error!("could not push {}: {problem}", change.name);
        return Err(FinishError::NotSent);
    }
    run_gh(
        gh,
        project,
        &[
            "pr",
            "create",
            "--head",
            &change.name,
            "--base",
            &base,
            "--title",
            &title,
            "--body",
            &body,
        ],
    )
    .await?;
    let viewed = run_gh(
        gh,
        project,
        &["pr", "view", &change.name, "--json", "number,url,state"],
    )
    .await?;
    parse_review(&viewed).ok_or(FinishError::NotSent)
}

/// How the review of `change` is going now, as GitHub says.
pub async fn review_now(project: &Path, review: &Review, gh: &str) -> Result<Review, FinishError> {
    let number = review.number.to_string();
    let viewed = run_gh(
        gh,
        project,
        &["pr", "view", &number, "--json", "number,url,state"],
    )
    .await?;
    parse_review(&viewed).ok_or(FinishError::NotSent)
}

/// Removes an accepted change, and brings the project folder up to date with GitHub when that touches nothing of the person's.
pub async fn tidy_after_acceptance(project: &Path, change: &Change) -> Result<Tidied, FinishError> {
    remove(project, change).await?;
    let Some(base) = &change.base else {
        return Ok(Tidied::Behind);
    };
    let git = Git::at(project);
    if let Err(problem) = git.output(&["fetch", "-q", "origin", base]).await {
        error!("could not fetch {base} after a review was accepted: {problem}");
        return Ok(Tidied::Behind);
    }
    let accepted = format!("refs/remotes/origin/{base}");
    let local = format!("refs/heads/{base}");
    let listed = git
        .output(&["worktree", "list", "--porcelain", "-z"])
        .await?;
    let checked_out = checkouts(&listed)
        .into_iter()
        .find(|checkout| checkout.branch.as_deref() == Some(base.as_str()));
    let moved = match checked_out {
        Some(checkout) => Git::at(&checkout.directory)
            .output(&["merge", "--ff-only", "--quiet", &accepted])
            .await
            .is_ok(),
        None => {
            git.exit_code(&["merge-base", "--is-ancestor", &local, &accepted])
                .await?
                == Some(0)
                && git.output(&["update-ref", &local, &accepted]).await.is_ok()
        }
    };
    Ok(if moved {
        Tidied::UpToDate
    } else {
        Tidied::Behind
    })
}

/// The title and the body of a review: the description's first line, and the rest.
fn split_description(description: &str) -> Option<(String, String)> {
    let description = description.trim();
    let (title, body) = description.split_once('\n').unwrap_or((description, ""));
    let title = title.trim();
    (!title.is_empty()).then(|| (title.to_string(), body.trim().to_string()))
}

/// Reads `gh pr view --json number,url,state`.
fn parse_review(printed: &str) -> Option<Review> {
    let viewed: Value = serde_json::from_str(printed).ok()?;
    let state = match viewed.get("state")?.as_str()? {
        "OPEN" => ReviewState::InReview,
        "MERGED" => ReviewState::Accepted,
        "CLOSED" => ReviewState::Declined,
        _ => return None,
    };
    Some(Review {
        number: viewed.get("number")?.as_u64()?,
        url: viewed.get("url")?.as_str()?.to_string(),
        state,
    })
}

/// Runs `gh` in the project and gives what it printed.
async fn run_gh(gh: &str, project: &Path, arguments: &[&str]) -> Result<String, FinishError> {
    let output = Command::new(program_path(gh))
        .args(arguments)
        .current_dir(project)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|problem| match problem.kind() {
            ErrorKind::NotFound => FinishError::GhMissing,
            _ => {
                error!("could not start {gh}: {problem}");
                FinishError::NotSent
            }
        })?;
    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        Some(SIGNED_OUT) => Err(FinishError::GhSignedOut),
        _ => {
            error!(
                "gh {arguments:?} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Err(FinishError::NotSent)
        }
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn review(state: ReviewState) -> Option<Review> {
        Some(Review {
            number: 12,
            url: "https://github.com/owner/app/pull/12".to_string(),
            state,
        })
    }

    lets_expect! {
        expect(is_github_url(url)) {
            when(url = "https://github.com/owner/app.git") as an_https_address to be_on_github { be_true }
            when(url = "https://github.com/owner/app") as an_https_address_without_ending to be_on_github { be_true }
            when(url = "git@github.com:owner/app.git") as a_short_ssh_address to be_on_github { be_true }
            when(url = "ssh://git@github.com/owner/app.git") as a_long_ssh_address to be_on_github { be_true }
            when(url = "https://GitHub.com/owner/app") as capitals to be_on_github { be_true }
            when(url = "https://gitlab.com/owner/app.git") as another_host to be_elsewhere { be_false }
            when(url = "https://github.com.evil.example/owner/app") as a_lookalike to be_elsewhere { be_false }
            when(url = "/srv/git/app.git") as a_folder to be_elsewhere { be_false }
            when(url = "") as nothing to be_elsewhere { be_false }
        }

        expect(split_description(description)) {
            when(description = "Fix the login") as one_line to be_the_title { equal(Some(("Fix the login".to_string(), String::new()))) }
            when(description = "  Fix the login\n\nIt redirected twice.\n") as several_lines to split_after_the_first {
                equal(Some(("Fix the login".to_string(), "It redirected twice.".to_string())))
            }
            when(description = " \n ") as nothing_written to give_nothing { equal(None) }
        }

        expect(parse_review(printed)) {
            when(printed = r#"{"number":12,"url":"https://github.com/owner/app/pull/12","state":"OPEN"}"#) as an_open_one to be_in_review { equal(review(ReviewState::InReview)) }
            when(printed = r#"{"number":12,"url":"https://github.com/owner/app/pull/12","state":"MERGED"}"#) as a_merged_one to be_accepted { equal(review(ReviewState::Accepted)) }
            when(printed = r#"{"number":12,"url":"https://github.com/owner/app/pull/12","state":"CLOSED"}"#) as a_closed_one to be_declined { equal(review(ReviewState::Declined)) }
            when(printed = r#"{"number":12,"url":"https://github.com/owner/app/pull/12","state":"DRAFTY"}"#) as an_unknown_state to give_nothing { equal(None) }
            when(printed = r#"{"url":"https://github.com/owner/app/pull/12","state":"OPEN"}"#) as no_number to give_nothing { equal(None) }
            when(printed = "not json") as not_json to give_nothing { equal(None) }
        }
    }
}
