//! Reading each program's limits, and naming their windows.
//!
//! - Claude Code prints a `rate_limit_event` line during every turn, with
//!   every window it counts (`unifiedWindows`). The last one of a turn wins.
//! - Codex prints nothing about them, but writes them into its own session
//!   file, `<codex home>/sessions/YYYY/MM/DD/rollout-<time>-<conversation>.jsonl`,
//!   on its `token_count` lines. That file is Codex's internal format, not a
//!   promise, so reading it fails softly.
//! - Grok shows its limits only inside its own window, so Teryon asks
//!   Grok's servers instead, see [`crate::usage::grok`].

use std::env::var_os;
use std::ffi::OsString;
use std::fs::{read_dir, read_to_string};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use directories::BaseDirs;
use serde_json::{from_str, Value};
use tracing::warn;

use crate::agents::Agent;
use crate::usage::{Limits, Reading, Window};

/// What a person calls a window, from its name or its length.
///
/// Claude Code names its windows; Codex only gives their length in minutes.
/// Names nobody has seen yet get their underscores turned into spaces, so a
/// new window shows up without a change to Teryon.
pub fn window_label(name: &str, minutes: Option<u32>) -> String {
    match name {
        "five_hour" => return "5 hours".to_string(),
        "seven_day" => return "week".to_string(),
        "seven_day_overage_included" => return "week, with extra usage".to_string(),
        _ => {}
    }
    match minutes {
        Some(10080) => "week".to_string(),
        Some(minutes) if minutes > 0 && minutes % 1440 == 0 => plural(minutes / 1440, "day"),
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => plural(minutes / 60, "hour"),
        Some(minutes) if minutes > 0 => plural(minutes, "minute"),
        _ => name.replace('_', " "),
    }
}

fn plural(count: u32, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

/// How long Claude Code's named windows are.
fn claude_minutes(name: &str) -> Option<u32> {
    match name {
        "five_hour" => Some(300),
        name if name.starts_with("seven_day") => Some(10080),
        _ => None,
    }
}

/// Puts windows shortest first, those of unknown length last, each kind by name.
pub(crate) fn sorted(mut windows: Vec<Window>) -> Vec<Window> {
    windows.sort_by(|one, other| {
        (one.minutes.is_none(), one.minutes, &one.name).cmp(&(
            other.minutes.is_none(),
            other.minutes,
            &other.name,
        ))
    });
    windows
}

/// A time given in seconds since 1970.
fn seconds(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|seconds| seconds as i64))
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
}

/// Whether Claude Code's status lets work go on: `allowed`, `allowed_warning` and the like.
fn allowed(status: Option<&str>) -> bool {
    status.is_none_or(|status| status.starts_with("allowed"))
}

/// The limits in one of Claude Code's `rate_limit_event` lines. Nothing when it names none.
///
/// Without `unifiedWindows`, only a window that is used up can be told, from
/// the line's own status, since the share used is not given.
pub fn claude_limits(report: &Value, now: DateTime<Utc>) -> Option<Limits> {
    let information = &report["rate_limit_info"];
    let windows: Vec<Window> = match information["unifiedWindows"].as_object() {
        Some(named) => named
            .iter()
            .filter_map(|(name, window)| {
                let used = window["utilization"].as_f64()?;
                let minutes = claude_minutes(name);
                Some(Window {
                    name: name.clone(),
                    label: window_label(name, minutes),
                    minutes,
                    used: (used * 100.0).clamp(0.0, 100.0),
                    resets_at: seconds(&window["resetsAt"]),
                    reached: !allowed(window["status"].as_str()),
                })
            })
            .collect(),
        None => {
            let name = information["rateLimitType"].as_str()?;
            if allowed(information["status"].as_str()) {
                return None;
            }
            let minutes = claude_minutes(name);
            vec![Window {
                name: name.to_string(),
                label: window_label(name, minutes),
                minutes,
                used: 100.0,
                resets_at: seconds(&information["resetsAt"]),
                reached: true,
            }]
        }
    };
    if windows.is_empty() {
        return None;
    }
    Some(Limits {
        agent: Agent::Claude.name().to_string(),
        plan: None,
        windows: sorted(windows),
        taken_at: now,
        failed: false,
        advice: None,
    })
}

/// Where Codex keeps its files: the setting, else `CODEX_HOME`, else `.codex` in the home folder.
pub fn codex_home(
    setting: Option<&Path>,
    variable: Option<OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    setting
        .map(Path::to_path_buf)
        .or_else(|| {
            variable
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| home.map(|home| home.join(".codex")))
}

/// [`codex_home`] for this machine.
pub fn codex_home_here(setting: Option<&Path>) -> Option<PathBuf> {
    codex_home(
        setting,
        var_os("CODEX_HOME"),
        BaseDirs::new().map(|base| base.home_dir().to_path_buf()),
    )
}

/// The session file of a Codex conversation: newest folders first, the last file by name within one.
///
/// A continued conversation goes on in the same file, wherever it started,
/// and Codex starts every file name with the time, so the last name is the newest.
pub fn codex_session_file(sessions: &Path, conversation: &str) -> Option<PathBuf> {
    let ending = format!("-{conversation}.jsonl");
    let newest_first = |folder: &Path| -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = read_dir(folder)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        entries.reverse();
        entries
    };
    for year in newest_first(sessions).iter().filter(|path| path.is_dir()) {
        for month in newest_first(year).iter().filter(|path| path.is_dir()) {
            for day in newest_first(month).iter().filter(|path| path.is_dir()) {
                let found = newest_first(day).into_iter().find(|file| {
                    file.file_name()
                        .is_some_and(|name| name.to_string_lossy().ends_with(&ending))
                });
                if found.is_some() {
                    return found;
                }
            }
        }
    }
    None
}

/// The limits on the last `token_count` line of a Codex session file that has them.
///
/// Lines that cannot be read, such as one cut off while Codex was writing
/// it, are passed over. The time is the line's own, else `now`.
pub fn codex_limits(text: &str, now: DateTime<Utc>) -> Option<Limits> {
    text.lines().rev().find_map(|line| {
        let line: Value = from_str(line).ok()?;
        let payload = &line["payload"];
        if payload["type"].as_str() != Some("token_count") {
            return None;
        }
        let limits = payload["rate_limits"].as_object()?;
        let reached_name = limits
            .get("rate_limit_reached_type")
            .and_then(Value::as_str);
        let windows: Vec<Window> = ["primary", "secondary"]
            .into_iter()
            .filter_map(|name| {
                let window = limits.get(name)?;
                let used = window["used_percent"].as_f64()?;
                let minutes = window["window_minutes"]
                    .as_u64()
                    .and_then(|minutes| u32::try_from(minutes).ok());
                Some(Window {
                    name: name.to_string(),
                    label: window_label(name, minutes),
                    minutes,
                    used: used.clamp(0.0, 100.0),
                    resets_at: seconds(&window["resets_at"]),
                    reached: used >= 100.0 || reached_name == Some(name),
                })
            })
            .collect();
        if windows.is_empty() {
            return None;
        }
        Some(Limits {
            agent: Agent::Codex.name().to_string(),
            plan: limits
                .get("plan_type")
                .and_then(Value::as_str)
                .map(plan_name),
            windows: sorted(windows),
            taken_at: line["timestamp"]
                .as_str()
                .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
                .map_or(now, |time| time.with_timezone(&Utc)),
            failed: false,
            advice: None,
        })
    })
}

/// A plan as Codex names it, `plus`, written as a person would: "Plus".
fn plan_name(plan: &str) -> String {
    plan.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut letters = word.chars();
            letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(letters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Codex's limits after a turn of the conversation `conversation`.
///
/// A missing or unreadable file is a failure, logged here. A file without
/// limits is not: the turn may have ended before Codex asked its model anything.
pub fn codex_reading(
    home: Option<&Path>,
    conversation: Option<&str>,
    now: DateTime<Utc>,
) -> Reading {
    let Some(conversation) = conversation else {
        return Reading::Nothing;
    };
    let Some(home) = home else {
        warn!("could not read Codex's limits: no home folder to find its sessions in");
        return Reading::Failed { advice: None };
    };
    let Some(file) = codex_session_file(&home.join("sessions"), conversation) else {
        warn!(
            "could not read Codex's limits: no session file for {conversation} in {}",
            home.join("sessions").display()
        );
        return Reading::Failed { advice: None };
    };
    match read_to_string(&file) {
        Ok(text) => codex_limits(&text, now).map_or(Reading::Nothing, Reading::Fresh),
        Err(problem) => {
            warn!(
                "could not read Codex's limits from {}: {problem}",
                file.display()
            );
            Reading::Failed { advice: None }
        }
    }
}

/// Marks the shortest window as reached, unless one already is.
pub(crate) fn marked_reached(mut limits: Limits) -> Limits {
    if !limits.windows.iter().any(|window| window.reached) {
        if let Some(shortest) = limits.windows.first_mut() {
            shortest.reached = true;
        }
    }
    limits
}

/// Limits nothing is known about, for an agent whose first reading failed.
pub(crate) fn unknown_limits(agent: Agent, now: DateTime<Utc>) -> Limits {
    Limits {
        agent: agent.name().to_string(),
        plan: None,
        windows: Vec::new(),
        taken_at: now,
        failed: false,
        advice: None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir_all, write};

    use chrono::{TimeDelta, TimeZone};
    use lets_expect::lets_expect;
    use serde_json::json;
    use tempfile::{tempdir, TempDir};

    use super::*;

    const SESSION: &str = include_str!("../../tests/samples/agents/codex-session.jsonl");
    const CLAUDE_FIRST: &str = include_str!("../../tests/samples/agents/claude-first.jsonl");

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap()
    }

    fn time(seconds: i64) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(seconds, 0)
    }

    /// The windows' names, shares used and whether they are reached.
    fn shape(limits: Option<Limits>) -> Option<Vec<(String, f64, bool)>> {
        limits.map(|limits| {
            limits
                .windows
                .into_iter()
                .map(|window| (window.name, window.used, window.reached))
                .collect()
        })
    }

    fn named(name: &str, used: f64, reached: bool) -> (String, f64, bool) {
        (name.to_string(), used, reached)
    }

    /// The first `rate_limit_event` line of the recorded Claude turn.
    fn recorded_event() -> Value {
        CLAUDE_FIRST
            .lines()
            .filter_map(|line| from_str::<Value>(line).ok())
            .find(|line| line["type"] == "rate_limit_event")
            .unwrap()
    }

    fn event(information: Value) -> Value {
        json!({ "type": "rate_limit_event", "rate_limit_info": information })
    }

    fn token_count(primary: Value, secondary: Value, plan: Value, reached: Value) -> String {
        json!({
            "timestamp": "2026-11-20T10:00:00.000Z",
            "type": "event_msg",
            "payload": { "type": "token_count", "rate_limits": {
                "primary": primary, "secondary": secondary, "plan_type": plan, "rate_limit_reached_type": reached
            }}
        })
        .to_string()
    }

    fn codex_window(used: f64, minutes: u32) -> Value {
        json!({ "used_percent": used, "window_minutes": minutes, "resets_at": 1795230073 })
    }

    /// A Codex home with session files at the given places, each holding the recorded session.
    fn codex_home_with(files: &[&str]) -> TempDir {
        let home = tempdir().unwrap();
        for file in files {
            let path = home.path().join("sessions").join(file);
            create_dir_all(path.parent().unwrap()).unwrap();
            write(path, SESSION).unwrap();
        }
        home
    }

    /// Where the session file of [`CONVERSATION`] is found among `files`, inside the sessions folder.
    fn found_among(files: &[&str]) -> Option<PathBuf> {
        let home = codex_home_with(files);
        let sessions = home.path().join("sessions");
        codex_session_file(&sessions, CONVERSATION)
            .map(|path| path.strip_prefix(&sessions).unwrap().to_path_buf())
    }

    /// Codex's reading after a turn, with a home holding the recorded session when `home` is set.
    fn reading_after(home: bool, conversation: Option<&str>) -> Reading {
        let folder = codex_home_with(&[
            "2026/11/20/rollout-2026-11-20T23-01-09-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl",
        ]);
        codex_reading(home.then_some(folder.path()), conversation, now())
    }

    fn failed(advice: Option<&str>) -> Reading {
        Reading::Failed {
            advice: advice.map(str::to_string),
        }
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
    }

    fn facts(limits: Limits) -> Facts {
        Facts {
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
        }
    }

    const CONVERSATION: &str = "01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6";

    lets_expect! {
        // requirement: known windows get labels; unknown names get their underscores turned into spaces; Codex's lengths become hours, days or a week
        expect(window_label(name, minutes)) {
            let minutes = None;

            when(name = "five_hour") as claudes_five_hours to equal("5 hours".to_string())
            when(name = "seven_day") as claudes_week to equal("week".to_string())
            when(name = "seven_day_overage_included") as claudes_week_with_extra_usage to equal("week, with extra usage".to_string())
            when(name = "seven_day_opus") as an_unknown_name to equal("seven day opus".to_string())
            when(name = "primary", minutes = Some(300)) as five_hours_in_minutes to equal("5 hours".to_string())
            when(name = "secondary", minutes = Some(10080)) as a_week_in_minutes to equal("week".to_string())
            when(name = "primary", minutes = Some(60)) as one_hour to equal("1 hour".to_string())
            when(name = "primary", minutes = Some(1440)) as one_day to equal("1 day".to_string())
            when(name = "primary", minutes = Some(43200)) as thirty_days to equal("30 days".to_string())
            when(name = "primary", minutes = Some(90)) as minutes_that_are_no_whole_hour to equal("90 minutes".to_string())
            when(name = "primary", minutes = Some(0)) as no_length to equal("primary".to_string())
        }

        // requirement: Claude's rate_limit_event becomes a reading of every window it names
        expect(shape(claude_limits(&report, now()))) as claudes_reading {
            let report = recorded_event();

            when(report = recorded_event()) as from_the_recording to give_every_window_shortest_first {
                equal(Some(vec![named("five_hour", 26.0, false), named("seven_day", 17.0, false), named("seven_day_overage_included", 31.0, false)]))
            }
            when(report = event(json!({ "unifiedWindows": { "five_hour": { "utilization": 0.5, "status": "allowed" } } }))) as with_one_window to give_it {
                equal(Some(vec![named("five_hour", 50.0, false)]))
            }
            when(report = event(json!({ "unifiedWindows": { "seven_day_opus": { "utilization": 0.1 }, "five_hour": { "utilization": 0.2 } } }))) as with_an_unknown_window to put_it_last {
                equal(Some(vec![named("five_hour", 20.0, false), named("seven_day_opus", 10.0, false)]))
            }
            when(report = event(json!({ "unifiedWindows": { "five_hour": { "utilization": 1.0, "status": "rejected" } } }))) as with_a_window_used_up to mark_it_reached {
                equal(Some(vec![named("five_hour", 100.0, true)]))
            }
            when(report = event(json!({ "unifiedWindows": { "five_hour": { "utilization": 0.9, "status": "allowed_warning" } } }))) as with_a_warning to leave_it_open {
                equal(Some(vec![named("five_hour", 90.0, false)]))
            }
            when(report = event(json!({ "unifiedWindows": { "five_hour": { "utilization": "much" }, "seven_day": { "utilization": 0.3 } } }))) as with_an_unreadable_window to skip_it {
                equal(Some(vec![named("seven_day", 30.0, false)]))
            }
            when(report = event(json!({ "unifiedWindows": {} }))) as with_no_windows to give_nothing { equal(None) }
            when(report = event(json!({ "status": "rejected", "rateLimitType": "five_hour", "resetsAt": 1794681000 }))) as without_windows_but_used_up to give_the_used_up_one {
                equal(Some(vec![named("five_hour", 100.0, true)]))
            }
            when(report = event(json!({ "status": "allowed", "rateLimitType": "five_hour" }))) as without_windows_and_allowed to give_nothing { equal(None) }
            when(report = json!({ "type": "rate_limit_event" })) as without_information to give_nothing { equal(None) }
        }

        expect(facts(claude_limits(&recorded_event(), now()).unwrap())) as the_recorded_claude_reading {
            to give_everything_else {
                equal(Facts {
                    agent: "claude".to_string(),
                    plan: None,
                    labels: vec!["5 hours".to_string(), "week".to_string(), "week, with extra usage".to_string()],
                    minutes: vec![Some(300), Some(10080), Some(10080)],
                    resets: vec![time(1794681000), time(1794733200), time(1794733200)],
                    taken_at: now(),
                })
            }
        }

        // requirement: Codex's reading is the last token_count line with rate_limits in its session file
        expect(shape(codex_limits(&text, now()))) as codexs_reading {
            let text = SESSION.to_string();

            when(text = SESSION.to_string()) as from_the_recording to give_both_windows {
                equal(Some(vec![named("primary", 0.0, false), named("secondary", 2.0, false)]))
            }
            when(text = format!("{}\n{}", token_count(codex_window(10.0, 300), codex_window(20.0, 10080), json!("plus"), json!(null)), token_count(codex_window(30.0, 300), Value::Null, json!("plus"), json!(null)))) as with_several_lines to take_the_last {
                equal(Some(vec![named("primary", 30.0, false)]))
            }
            when(text = format!("{}\n{{\"timestamp\":", token_count(codex_window(10.0, 300), Value::Null, json!("plus"), json!(null)))) as with_a_cut_off_last_line to take_the_one_before {
                equal(Some(vec![named("primary", 10.0, false)]))
            }
            when(text = token_count(codex_window(100.0, 300), codex_window(20.0, 10080), json!("plus"), json!(null))) as with_a_window_used_up to mark_it_reached {
                equal(Some(vec![named("primary", 100.0, true), named("secondary", 20.0, false)]))
            }
            when(text = token_count(codex_window(90.0, 300), codex_window(20.0, 10080), json!("plus"), json!("secondary"))) as naming_the_reached_window to mark_it {
                equal(Some(vec![named("primary", 90.0, false), named("secondary", 20.0, true)]))
            }
            when(text = json!({ "type": "event_msg", "payload": { "type": "token_count", "rate_limits": null } }).to_string()) as without_rate_limits to give_nothing { equal(None) }
            when(text = String::new()) as from_an_empty_file to give_nothing { equal(None) }
        }

        expect(facts(codex_limits(SESSION, now()).unwrap())) as the_recorded_codex_reading {
            to give_everything_else {
                equal(Facts {
                    agent: "codex".to_string(),
                    plan: Some("Plus".to_string()),
                    labels: vec!["5 hours".to_string(), "week".to_string()],
                    minutes: vec![Some(300), Some(10080)],
                    resets: vec![time(1795230073), time(1795780966)],
                    taken_at: DateTime::parse_from_rfc3339("2026-11-20T22:01:17.665Z").unwrap().with_timezone(&Utc),
                })
            }
        }

        expect(codex_limits(&text, now()).unwrap().plan) as codexs_plan {
            when(text = token_count(codex_window(1.0, 300), Value::Null, Value::Null, Value::Null)) as without_a_plan to be_none { equal(None) }
            when(text = token_count(codex_window(1.0, 300), Value::Null, json!("team_plus"), Value::Null)) as of_two_words to write_both { equal(Some("Team Plus".to_string())) }
        }

        // requirement: Codex's session file is found by the conversation at the end of its name, newest folders first
        expect(found_among(&files)) as finding_codexs_session_file {
            let files: Vec<&str> = vec![];

            when(files = vec![]) as without_sessions to find_none { equal(None) }
            when(files = vec!["2026/11/20/rollout-2026-11-20T23-01-09-other.jsonl"]) as with_other_conversations_only to find_none { equal(None) }
            when(files = vec!["2026/11/20/rollout-2026-11-20T23-01-09-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl"]) as with_one to find_it {
                equal(Some(PathBuf::from("2026/11/20/rollout-2026-11-20T23-01-09-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl")))
            }
            when(files = vec![
                "2026/10/31/rollout-2026-10-31T09-00-00-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl",
                "2026/11/02/rollout-2026-11-02T09-00-00-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl",
                "2026/11/02/rollout-2026-11-02T08-00-00-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl",
            ]) as with_several to find_the_newest {
                equal(Some(PathBuf::from("2026/11/02/rollout-2026-11-02T09-00-00-01a1fafa-4aea-78c3-b6f7-d8ad5da8abc6.jsonl")))
            }
        }

        // requirement: Codex's home is the setting, else CODEX_HOME, else ~/.codex
        expect(codex_home(setting.as_deref(), variable.clone(), Some(PathBuf::from("/home/someone")))) as codexs_home {
            let setting: Option<PathBuf> = None;
            let variable: Option<OsString> = None;

            when(variable = None) as by_default to be_in_the_home_folder { equal(Some(PathBuf::from("/home/someone/.codex"))) }
            when(variable = Some(OsString::from("/elsewhere"))) as with_codex_home_set to follow_it { equal(Some(PathBuf::from("/elsewhere"))) }
            when(variable = Some(OsString::new())) as with_codex_home_empty to ignore_it { equal(Some(PathBuf::from("/home/someone/.codex"))) }
            when(setting = Some(PathBuf::from("/set")), variable = Some(OsString::from("/elsewhere"))) as with_a_setting to follow_the_setting { equal(Some(PathBuf::from("/set"))) }
        }

        expect(reading_after(home, conversation)) as codexs_reading_after_a_turn {
            let home = true;
            let conversation = Some(CONVERSATION);

            when(conversation = Some(CONVERSATION)) as with_its_file to be_fresh { equal(Reading::Fresh(codex_limits(SESSION, now()).unwrap())) }
            when(conversation = None) as before_codex_named_the_conversation to be_nothing { equal(Reading::Nothing) }
            when(conversation = Some("another")) as without_its_file to fail { equal(failed(None)) }
            when(home = false) as without_a_home_folder to fail { equal(failed(None)) }
        }
    }
}
