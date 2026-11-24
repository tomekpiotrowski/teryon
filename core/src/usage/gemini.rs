//! Reading Gemini's limits from Antigravity.
//!
//! Antigravity reports them only through its own program. After a Gemini
//! turn Teryon asks `agy -p /usage`. Version 1.5.16 answers that without
//! running a turn. An older program is not asked, because it may send the
//! text to the model instead.
//!
//! The answer also lists other providers. Teryon runs only Gemini through
//! Antigravity, so the rest is left out. Every failure is soft: the last
//! reading stays, marked failed.

use std::env::temp_dir;
use std::fs::{create_dir, remove_dir_all};
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{from_slice, from_str, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::warn;
use uuid::Uuid;

use crate::agents::process::{ProcessTree, Signal};
use crate::agents::runner::program_path;
use crate::agents::Agent;
use crate::usage::limits::sorted;
use crate::usage::{Limits, Reading, Window};

/// How many times to start Antigravity when Linux says its program is busy.
///
/// The same pause as a process-tree start. A freshly written test program
/// can still be open for writing in a process that was forked while the
/// file was written, and that process drops the handle when it starts.
const BUSY_TRIES: u32 = 40;
const BUSY_PAUSE: Duration = Duration::from_millis(5);

/// How long Antigravity may take to report Gemini's limits.
///
/// `agy -p /usage` starts the same way `agy models` does, which can take
/// several seconds, so the wait matches that listing.
pub const GEMINI_PATIENCE: Duration = Duration::from_secs(30);

/// What to do when Antigravity's answer says its sign-in is missing.
const SIGN_IN_ADVICE: &str =
    "Run agy in a terminal and sign in so Teryon can read Gemini's limits.";

/// What to tell a person whose Antigravity is too old to ask, or names no version.
///
/// The usage screen shows this beside the failed reading, the same way a
/// missing sign-in shows its own sentence.
fn update_advice() -> String {
    format!(
        "Update agy to {} or newer so Teryon can read Gemini's limits.",
        version_text(MINIMUM_VERSION)
    )
}

/// The first Antigravity known to treat `/usage` as a command.
///
/// 1.5.16 answers `agy -p /usage` with `num_turns` 0. An older one was not
/// checked live, and might spend a turn on that text.
const MINIMUM_VERSION: (u64, u64, u64) = (1, 5, 16);

/// Gemini's limits, from what `agy -p /usage --output-format json` prints.
///
/// Antigravity answers that without running a turn, for every provider it
/// offers. Teryon only runs Gemini through it, so only the Gemini group is
/// kept: the one whose bucket ids start with `gemini-`, or, failing that,
/// whose name starts with "Gemini".
///
/// Each bucket that names a remaining share becomes a window, shortest
/// first. Nothing when the answer is not a successful usage report, or
/// names no such bucket.
///
/// `num_turns` must be 0, which is how Antigravity says it did not run a
/// turn. A model reply would already have spent one; leaving it out of the
/// limits does not give that back.
pub fn gemini_limits(text: &str, now: DateTime<Utc>) -> Option<Limits> {
    let answer: Value = from_str(text).ok()?;
    if answer["status"].as_str() != Some("SUCCESS") {
        return None;
    }
    if answer["num_turns"].as_u64() != Some(0) {
        return None;
    }
    if answer["command"]["name"].as_str() != Some("usage") {
        return None;
    }
    let groups = answer["command"]["data"]["groups"].as_array()?;
    let group = gemini_group(groups)?;
    let windows = group["buckets"]
        .as_array()?
        .iter()
        .filter_map(window_of)
        .collect::<Vec<_>>();
    if windows.is_empty() {
        return None;
    }
    Some(Limits {
        agent: Agent::Gemini.name().to_string(),
        plan: None,
        windows: sorted(windows),
        taken_at: now,
        failed: false,
        advice: None,
    })
}

/// The Gemini group: ids starting with `gemini-` win over a group only named Gemini.
fn gemini_group(groups: &[Value]) -> Option<&Value> {
    groups
        .iter()
        .find(|group| bucket_ids(group).any(|id| id.starts_with("gemini-")))
        .or_else(|| {
            groups.iter().find(|group| {
                group["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("Gemini"))
            })
        })
}

fn bucket_ids(group: &Value) -> impl Iterator<Item = &str> {
    group["buckets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|bucket| bucket["id"].as_str())
}

/// One bucket as a window. Nothing when it names no id or no remaining share.
fn window_of(bucket: &Value) -> Option<Window> {
    let name = bucket["id"].as_str().filter(|id| !id.is_empty())?;
    let fraction = bucket["remaining_fraction"].as_f64()?;
    if !fraction.is_finite() {
        return None;
    }
    let (minutes, label) = described(bucket);
    Some(Window {
        name: name.to_string(),
        label,
        minutes,
        used: ((1.0 - fraction) * 100.0).clamp(0.0, 100.0),
        resets_at: bucket["reset_time"].as_str().and_then(reset_time),
        reached: fraction <= 0.0,
    })
}

/// How long a bucket's window is, and what a person calls it.
///
/// `5h` and `weekly` are the windows Antigravity names for Gemini, and they
/// use the same words as Claude's and Codex's windows of those lengths.
/// Anything else keeps the bucket's own name and no length, so a new window
/// still shows up.
fn described(bucket: &Value) -> (Option<u32>, String) {
    match bucket["window"].as_str() {
        Some("5h") => (Some(300), "5 hours".to_string()),
        Some("weekly") => (Some(10080), "week".to_string()),
        _ => (None, bucket["name"].as_str().unwrap_or("").to_string()),
    }
}

fn reset_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// Gemini's limits, asked of Antigravity within [`GEMINI_PATIENCE`].
///
/// Run after a Gemini turn has finished, so the few seconds it takes never
/// hold the thread up. The program runs in a temporary folder, so it cannot
/// write into the project. `agy --version` and `/usage` share the one wait.
/// Any failure is soft. When the answer says to sign in, or the program is
/// too old to ask, the reading says what to do.
pub async fn gemini_reading(program: &str, now: DateTime<Utc>) -> Reading {
    read_within(program, now, GEMINI_PATIENCE).await
}

async fn read_within(program: &str, now: DateTime<Utc>, patience: Duration) -> Reading {
    match timeout(patience, ask(program)).await {
        Ok(Ok(text)) => match gemini_limits(&text, now) {
            Some(limits) => Reading::Fresh(limits),
            None => {
                warn!("could not read Gemini's limits: Antigravity's answer names none");
                failed_reading(&text)
            }
        },
        Ok(Err(problem)) => {
            warn!("could not read Gemini's limits: {problem}");
            failed_reading(&problem)
        }
        Err(_) => {
            warn!("could not read Gemini's limits: Antigravity took longer than {patience:?}");
            Reading::Failed { advice: None }
        }
    }
}

async fn ask(program: &str) -> Result<String, String> {
    ensure_recent(program).await?;
    let folder = TemporaryFolder::new()
        .map_err(|problem| format!("no folder to run Antigravity in: {problem}"))?;
    let mut command = Command::new(program_path(program));
    command
        .args(["-p", "/usage", "--output-format", "json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(folder.path());
    let mut running = Running::start(command)
        .map_err(|problem| format!("could not start `{program}`: {problem}"))?;
    read_answer(&mut running).await
}

/// Refuses to ask `/usage` unless `program --version` is at least [`MINIMUM_VERSION`].
///
/// The version and the usage answer share the caller's wait. An older program,
/// or one that names no version, is not asked. The error is the advice shown
/// on the usage screen.
async fn ensure_recent(program: &str) -> Result<(), String> {
    let mut command = Command::new(program_path(program));
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = output_when_free(&mut command)
        .await
        .map_err(|problem| format!("could not start `{program}`: {problem}"))?;
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    match version_number(&printed) {
        Some(version) if version >= MINIMUM_VERSION => Ok(()),
        _ => Err(update_advice()),
    }
}

/// The first `major.minor.patch` in `text`.
fn version_number(text: &str) -> Option<(u64, u64, u64)> {
    let mut rest = text;
    while let Some(start) = rest.find(|letter: char| letter.is_ascii_digit()) {
        rest = &rest[start..];
        let Some((major, after)) = take_number(rest) else {
            rest = rest.get(1..).unwrap_or("");
            continue;
        };
        if let Some(version) = dotted(major, after) {
            return Some(version);
        }
        rest = rest.get(1..).unwrap_or("");
    }
    None
}

fn dotted(major: u64, text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('.')?;
    let (minor, text) = take_number(text)?;
    let text = text.strip_prefix('.')?;
    let (patch, _) = take_number(text)?;
    Some((major, minor, patch))
}

fn take_number(text: &str) -> Option<(u64, &str)> {
    let end = text
        .find(|letter: char| !letter.is_ascii_digit())
        .unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    Some((text[..end].parse().ok()?, &text[end..]))
}

fn version_text(version: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// Runs `command` to the end, trying again when Linux says the program is busy.
async fn output_when_free(command: &mut Command) -> Result<std::process::Output, IoError> {
    let mut tries = 0;
    loop {
        match command.output().await {
            Err(error) if tries < BUSY_TRIES && error.kind() == ErrorKind::ExecutableFileBusy => {
                tries += 1;
                tokio::time::sleep(BUSY_PAUSE).await;
            }
            result => return result,
        }
    }
}

/// Reads Antigravity's answer, and stops once that answer is complete.
///
/// Waiting for the pipes to close would sit out the whole patience when a
/// helper kept them open after the answer was printed. The whole process
/// group is ended then, and again if this is dropped while it is still running.
async fn read_answer(running: &mut Running) -> Result<String, String> {
    let mut stdout = running
        .tree
        .child()
        .stdout
        .take()
        .ok_or_else(|| "Antigravity gave no output".to_string())?;
    let mut stderr = running
        .tree
        .child()
        .stderr
        .take()
        .ok_or_else(|| "Antigravity gave no error output".to_string())?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut out_buf = [0u8; 8192];
    let mut err_buf = [0u8; 8192];
    let mut stdout_open = true;
    let mut stderr_open = true;

    loop {
        if !stdout_open && !stderr_open {
            break;
        }
        tokio::select! {
            read = stdout.read(&mut out_buf), if stdout_open => {
                match read {
                    Ok(0) => stdout_open = false,
                    Ok(n) => {
                        out.extend_from_slice(&out_buf[..n]);
                        if from_slice::<Value>(&out).is_ok() {
                            running.stop().await;
                            return Ok(String::from_utf8_lossy(&out).into_owned());
                        }
                    }
                    Err(problem) => {
                        return Err(format!("could not read Antigravity's answer: {problem}"));
                    }
                }
            }
            read = stderr.read(&mut err_buf), if stderr_open => {
                match read {
                    Ok(0) => stderr_open = false,
                    Ok(n) => err.extend_from_slice(&err_buf[..n]),
                    Err(problem) => {
                        return Err(format!("could not read Antigravity's errors: {problem}"));
                    }
                }
            }
        }
    }

    let status = running
        .tree
        .child()
        .wait()
        .await
        .map_err(|problem| format!("could not wait for Antigravity: {problem}"))?;
    running.disarm();
    let stdout = String::from_utf8_lossy(&out);
    let stderr = String::from_utf8_lossy(&err);
    if status.success() {
        return Ok(stdout.into_owned());
    }
    let code = status
        .code()
        .map_or_else(|| "a signal".to_string(), |code| code.to_string());
    Err(format!("agy exited with {code}: {stderr}{stdout}"))
}

/// Antigravity, ended together with anything it starts.
struct Running {
    tree: ProcessTree,
    /// Set once the process has been waited for, so dropping it does not signal a pid that may have been reused.
    done: bool,
}

impl Running {
    fn start(command: Command) -> Result<Self, IoError> {
        Ok(Self {
            tree: ProcessTree::start(command)?,
            done: false,
        })
    }

    async fn stop(&mut self) {
        if self.done {
            return;
        }
        self.tree.signal(Signal::Kill);
        let _ = self.tree.child().wait().await;
        self.done = true;
    }

    fn disarm(&mut self) {
        self.done = true;
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.tree.signal(Signal::Kill);
        let _ = self.tree.child().try_wait();
    }
}

/// A failed reading, advising a sign-in or an update when `text` says that is the problem.
fn failed_reading(text: &str) -> Reading {
    let advice = if mentions_sign_in(text) {
        Some(SIGN_IN_ADVICE.to_string())
    } else if text == update_advice() {
        Some(update_advice())
    } else {
        None
    };
    Reading::Failed { advice }
}

/// Whether Antigravity's answer asks for a sign-in.
///
/// The only signed-out sentences known from the program are "You are
/// currently not signed in." and "You are not logged into Antigravity."
/// The match is those phrases. A bare "login" or "authentication" shows up
/// in unrelated errors, and must not send the person to sign in.
fn mentions_sign_in(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "sign in",
        "signed in",
        "signing in",
        "sign-in",
        "log in",
        "logged in",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

/// A folder under the system temporary directory, removed when dropped.
struct TemporaryFolder(PathBuf);

impl TemporaryFolder {
    fn new() -> Result<Self, IoError> {
        let path = temp_dir().join(format!("teryon-gemini-{}", Uuid::new_v4()));
        create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryFolder {
    fn drop(&mut self) {
        let _ = remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{set_permissions, File};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use chrono::TimeZone;
    use lets_expect::lets_expect;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    const RECORDED: &str = include_str!("../../tests/samples/gemini-usage.json");

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap()
    }

    fn instant(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn failed(advice: Option<&str>) -> Reading {
        Reading::Failed {
            advice: advice.map(str::to_string),
        }
    }

    fn failed_update() -> Reading {
        Reading::Failed {
            advice: Some(update_advice()),
        }
    }

    /// The windows' names, shares used (rounded) and whether they are reached.
    fn shares(limits: Option<Limits>) -> Option<Vec<(String, i64, bool)>> {
        limits.map(|limits| {
            limits
                .windows
                .into_iter()
                .map(|window| (window.name, window.used.round() as i64, window.reached))
                .collect()
        })
    }

    fn named(name: &str, used: i64, reached: bool) -> (String, i64, bool) {
        (name.to_string(), used, reached)
    }

    /// What a reading says besides the shares used.
    #[derive(Debug, PartialEq)]
    struct Facts {
        agent: String,
        plan: Option<String>,
        labels: Vec<String>,
        minutes: Vec<Option<u32>>,
        resets: Vec<Option<DateTime<Utc>>>,
        taken_at: DateTime<Utc>,
        failed: bool,
    }

    fn facts(limits: Option<Limits>) -> Option<Facts> {
        limits.map(|limits| Facts {
            agent: limits.agent,
            plan: limits.plan,
            labels: limits
                .windows
                .iter()
                .map(|window| window.label.clone())
                .collect(),
            minutes: limits.windows.iter().map(|window| window.minutes).collect(),
            resets: limits
                .windows
                .iter()
                .map(|window| window.resets_at)
                .collect(),
            taken_at: limits.taken_at,
            failed: limits.failed,
        })
    }

    /// A successful usage answer whose only group is `group`.
    fn answer(group: Value) -> String {
        json!({
            "status": "SUCCESS",
            "num_turns": 0,
            "command": { "name": "usage", "data": { "groups": [group] } }
        })
        .to_string()
    }

    fn group(name: &str, buckets: Vec<Value>) -> Value {
        json!({ "name": name, "buckets": buckets })
    }

    fn bucket(
        id: &str,
        name: &str,
        window: Option<&str>,
        fraction: Option<f64>,
        reset: Option<&str>,
    ) -> Value {
        let mut bucket = json!({ "id": id, "name": name });
        if let Some(window) = window {
            bucket["window"] = json!(window);
        }
        if let Some(fraction) = fraction {
            bucket["remaining_fraction"] = json!(fraction);
        }
        if let Some(reset) = reset {
            bucket["reset_time"] = json!(reset);
        }
        bucket
    }

    /// A program that prints `body` only when asked for `/usage` as Teryon asks.
    fn answering(body: &str) -> String {
        usage_script(body, "")
    }

    /// Like [`answering`], then stays running, as a helper that keeps the pipes open would.
    fn answering_then_waiting(body: &str) -> String {
        usage_script(body, "sleep 30")
    }

    fn usage_script(body: &str, after: &str) -> String {
        format!(
            "if [ \"$1\" = -p ] && [ \"$2\" = /usage ] && [ \"$3\" = --output-format ] && [ \"$4\" = json ]; then\nprintf '%s\\n' '{body}'\n{after}\nelse\necho \"wrong arguments: $*\" >&2\nexit 2\nfi"
        )
    }

    /// Prints 1.5.16 for `--version`, then runs `body`.
    fn current(body: &str) -> String {
        format!("if [ \"$1\" = --version ]; then\nprintf '%s\\n' '1.5.16'\nexit 0\nfi\n{body}")
    }

    /// An older Antigravity that would claim a missing sign-in if `/usage` were run.
    fn old_antigravity() -> String {
        "if [ \"$1\" = --version ]; then\nprintf '%s\\n' '1.5.15'\nexit 0\nfi\necho 'You are currently not signed in.' >&2\nexit 1".to_string()
    }

    /// A program whose `--version` names no version, and whose `/usage` would claim a missing sign-in.
    fn unversioned() -> String {
        "if [ \"$1\" = --version ]; then\nprintf '%s\\n' 'antigravity'\nexit 0\nfi\necho 'You are currently not signed in.' >&2\nexit 1".to_string()
    }

    /// What `program` reports, within `patience`, with `--version` answered as 1.5.16.
    /// Nothing for `script` means the program is missing.
    async fn run(script: Option<String>, patience: Duration) -> Reading {
        run_exact(script.map(|script| current(&script)), patience).await
    }

    /// What `program` reports, within `patience`, running `script` unchanged.
    ///
    /// The script is closed before it is run. Renaming it does not make it a
    /// different file, so a start that still finds it open is retried by the
    /// reader.
    async fn run_exact(script: Option<String>, patience: Duration) -> Reading {
        let folder = tempdir().unwrap();
        let program = folder.path().join("agy");
        if let Some(script) = script {
            let staged = folder.path().join("agy.writing");
            let mut file = File::create(&staged).unwrap();
            write!(file, "#!/bin/sh\n{script}\n").unwrap();
            file.sync_all().unwrap();
            drop(file);
            set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::rename(&staged, &program).unwrap();
            File::open(folder.path()).unwrap().sync_all().unwrap();
        }
        read_within(&program.to_string_lossy(), now(), patience).await
    }

    lets_expect! {
        // requirement: Antigravity's usage answer becomes Gemini's two windows, 5 hours first
        expect(shares(gemini_limits(RECORDED, now()))) as the_recorded_answer {
            to give_the_five_hour_and_weekly_windows {
                equal(Some(vec![named("gemini-5h", 11, false), named("gemini-weekly", 3, false)]))
            }
        }

        expect(facts(gemini_limits(RECORDED, now()))) as the_recorded_windows {
            to name_them_like_the_other_agents {
                equal(Some(Facts {
                    agent: "gemini".to_string(),
                    plan: None,
                    labels: vec!["5 hours".to_string(), "week".to_string()],
                    minutes: vec![Some(300), Some(10080)],
                    resets: vec![Some(instant("2026-11-24T14:45:28Z")), Some(instant("2026-12-01T09:45:28Z"))],
                    taken_at: now(),
                    failed: false,
                }))
            }
        }

        // requirement: a share is kept between 0 and 100, and nothing left means the window is used up
        expect(shares(gemini_limits(&text, now()))) as a_remaining_share {
            let text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(0.0), None)]));

            when(text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(0.0), None)]))) as with_nothing_left to be_used_up {
                equal(Some(vec![named("gemini-5h", 100, true)]))
            }
            when(text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(-0.25), None)]))) as with_less_than_nothing to stay_in_range_and_be_used_up {
                equal(Some(vec![named("gemini-5h", 100, true)]))
            }
            when(text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(1.4), None)]))) as with_more_than_everything to stay_in_range {
                equal(Some(vec![named("gemini-5h", 0, false)]))
            }
        }

        // requirement: an unknown window keeps the bucket's name, has no length, and is shown last
        expect(facts(gemini_limits(&text, now()))) as an_unknown_window {
            let text = answer(group("Gemini Models", vec![
                bucket("gemini-session", "Session Limit Remaining", Some("session"), Some(0.5), None),
                bucket("gemini-5h", "Five Hour Limit Remaining", Some("5h"), Some(0.5), None),
            ]));

            to be_named_and_sorted_last {
                equal(Some(Facts {
                    agent: "gemini".to_string(),
                    plan: None,
                    labels: vec!["5 hours".to_string(), "Session Limit Remaining".to_string()],
                    minutes: vec![Some(300), None],
                    resets: vec![None, None],
                    taken_at: now(),
                    failed: false,
                }))
            }
        }

        // requirement: a bucket without a remaining share is skipped, and a bad reset time is left out
        expect(facts(gemini_limits(&text, now()))) as a_partial_bucket {
            let text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(0.5), Some("tomorrow"))]));

            when(text = answer(group("Gemini Models", vec![
                bucket("gemini-5h", "Five Hour", Some("5h"), None, None),
                bucket("gemini-weekly", "Weekly", Some("weekly"), Some(0.5), None),
            ]))) as without_a_remaining_share to skip_it {
                equal(Some(Facts {
                    agent: "gemini".to_string(),
                    plan: None,
                    labels: vec!["week".to_string()],
                    minutes: vec![Some(10080)],
                    resets: vec![None],
                    taken_at: now(),
                    failed: false,
                }))
            }
            when(text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), None, None)]))) as with_every_bucket_skipped to be_nothing {
                equal(None)
            }
            when(text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(0.5), None)]))) as without_a_reset_time to leave_it_out {
                equal(Some(Facts {
                    agent: "gemini".to_string(),
                    plan: None,
                    labels: vec!["5 hours".to_string()],
                    minutes: vec![Some(300)],
                    resets: vec![None],
                    taken_at: now(),
                    failed: false,
                }))
            }
            when(text = answer(group("Gemini Models", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(0.5), Some("tomorrow"))]))) as with_an_unreadable_reset_time to leave_it_out {
                equal(Some(Facts {
                    agent: "gemini".to_string(),
                    plan: None,
                    labels: vec!["5 hours".to_string()],
                    minutes: vec![Some(300)],
                    resets: vec![None],
                    taken_at: now(),
                    failed: false,
                }))
            }
        }

        // requirement: only the Gemini group is read, by its bucket ids, or by its name when the ids do not say
        expect(shares(gemini_limits(&text, now()))) as the_chosen_group {
            let text = answer(group("Claude and GPT models", vec![bucket("claude-5h", "Five Hour", Some("5h"), Some(0.5), None)]));

            when(text = answer(group("Claude and GPT models", vec![bucket("claude-5h", "Five Hour", Some("5h"), Some(0.5), None)]))) as with_only_other_providers to be_nothing {
                equal(None)
            }
            when(text = json!({
                "status": "SUCCESS",
                "num_turns": 0,
                "command": { "name": "usage", "data": { "groups": [
                    group("Gemini Models", vec![bucket("weekly", "Weekly Limit Remaining", Some("weekly"), Some(0.5), None)]),
                    group("Other", vec![bucket("gemini-5h", "Five Hour", Some("5h"), Some(0.8), None)]),
                ] } }
            }).to_string()) as with_gemini_ids_in_a_later_group to prefer_the_ids {
                equal(Some(vec![named("gemini-5h", 20, false)]))
            }
            when(text = answer(group("Gemini Models", vec![bucket("weekly", "Weekly Limit Remaining", Some("weekly"), Some(0.5), None)]))) as with_a_gemini_name_and_no_gemini_ids to use_the_name {
                equal(Some(vec![named("weekly", 50, false)]))
            }
        }

        // requirement: anything other than a successful usage answer is not a reading
        expect(gemini_limits(&text, now()).is_none()) as not_a_usage_answer {
            let text = String::new();

            when(text = r#"{"status":"ERROR","command":{"name":"usage","data":{"groups":[]}}}"#.to_string()) as with_an_error_status to be_nothing { equal(true) }
            when(text = r#"{"status":"SUCCESS","command":{"name":"credits","data":{"groups":[]}}}"#.to_string()) as with_another_command to be_nothing { equal(true) }
            when(text = r#"{"status":"SUCCESS"}"#.to_string()) as without_a_command to be_nothing { equal(true) }
            when(text = "not json".to_string()) as with_no_json to be_nothing { equal(true) }
            when(text = json!({"status":"SUCCESS","command":{"name":"usage","data":{"groups":[{"name":"Gemini Models","buckets":[{"id":"gemini-5h","remaining_fraction":0.5,"window":"5h"}]}]}}}).to_string()) as without_num_turns to be_nothing { equal(true) }
            when(text = json!({"status":"SUCCESS","num_turns":1,"command":{"name":"usage","data":{"groups":[{"name":"Gemini Models","buckets":[{"id":"gemini-5h","remaining_fraction":0.5,"window":"5h"}]}]}}}).to_string()) as after_a_real_turn to be_nothing { equal(true) }
        }

        expect(mentions_sign_in(text)) {
            when(text = "You are currently not signed in.") as not_signed_in to be_true
            when(text = "You are not logged into Antigravity.") as not_logged_into to be_true
            when(text = "Please sign in") as sign_in to be_true
            when(text = "signing in") as signing_in to be_true
            when(text = "sign-in required") as a_hyphenated_sign_in to be_true
            when(text = "log in to continue") as log_in to be_true
            when(text = "authentication failed at login.googleapis.com") as an_unrelated_login to be_false
            when(text = "signin") as a_glued_signin to be_false
            when(text = "login") as a_bare_login to be_false
        }

        expect(version_number(text).map(version_text)) as the_version_antigravity_reports {
            when(text = "1.5.16\n") as the_live_print to be_read { equal(Some("1.5.16".to_string())) }
            when(text = "antigravity 1.5.16") as with_a_name_before_it to be_read { equal(Some("1.5.16".to_string())) }
            when(text = "1.5") as without_a_patch to be_missing { equal(None) }
            when(text = "no version here") as with_no_number to be_missing { equal(None) }
        }

        expect(version_number(text).is_some_and(|version| version >= MINIMUM_VERSION)) as a_version_new_enough_to_ask {
            when(text = "1.5.16") as the_first_version_that_answers_usage to be_new_enough { equal(true) }
            when(text = "1.5.15") as one_patch_older to be_too_old { equal(false) }
            when(text = "1.6.0") as a_newer_minor to be_new_enough { equal(true) }
            when(text = "1.4.20") as an_older_minor to be_too_old { equal(false) }
            when(text = "2.0.0") as a_newer_major to be_new_enough { equal(true) }
            when(text = "") as nothing_printed to be_too_old { equal(false) }
        }
    }

    lets_expect! { #tokio_test
        // requirement: asking Antigravity for /usage gives Gemini's limits, and every failure is soft
        expect(run(script, patience).await) as asking_antigravity {
            let script = Some(answering(RECORDED.trim()));
            let patience = Duration::from_secs(5);

            when(script = Some(answering(RECORDED.trim()))) as when_it_answers to be_fresh {
                equal(Reading::Fresh(gemini_limits(RECORDED, now()).unwrap()))
            }
            when(script = Some(answering(r#"{"status":"SUCCESS","num_turns":0,"command":{"name":"usage","data":{"groups":[]}}}"#))) as when_the_answer_names_none to fail_without_advice {
                equal(failed(None))
            }
            when(script = Some("echo broken >&2\nexit 1".to_string())) as when_it_exits_with_an_error to fail_without_advice {
                equal(failed(None))
            }
            when(script = Some("echo 'You are currently not signed in.' >&2\nexit 1".to_string())) as when_the_error_says_to_sign_in to advise_signing_in {
                equal(failed(Some(SIGN_IN_ADVICE)))
            }
            when(script = Some("echo 'You are not logged into Antigravity.'".to_string())) as when_the_answer_says_to_sign_in to advise_signing_in {
                equal(failed(Some(SIGN_IN_ADVICE)))
            }
            when(script = Some("echo 'authentication failed at login.googleapis.com' >&2\nexit 1".to_string())) as when_the_error_is_not_about_signing_in to fail_without_advice {
                equal(failed(None))
            }
            when(script = Some(answering_then_waiting(RECORDED.trim())), patience = Duration::from_secs(20)) as when_it_answers_and_stays_open to be_fresh {
                equal(Reading::Fresh(gemini_limits(RECORDED, now()).unwrap()))
            }
            when(script = Some("exec sleep 30".to_string()), patience = Duration::from_millis(200)) as when_it_takes_too_long to fail {
                equal(failed(None))
            }
            when(script = None) as when_the_program_is_missing to fail {
                equal(failed(None))
            }
        }

        expect(run_exact(script, patience).await) as an_antigravity_that_must_not_be_asked {
            let patience = Duration::from_secs(5);

            when(script = Some(old_antigravity())) as when_it_is_older_than_1_5_16 to advise_updating {
                equal(failed_update())
            }
            when(script = Some(unversioned())) as when_it_reports_no_version to advise_updating {
                equal(failed_update())
            }
        }
    }
}
