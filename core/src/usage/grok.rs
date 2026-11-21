//! Reading Grok's limits from Grok's servers.
//!
//! Grok shows its limits only inside its own window: `grok usage --json`
//! refuses anywhere else. So after a Grok turn Teryon asks Grok's servers
//! what Grok's own `/usage` asks them, with the sign-in Grok saved.
//!
//! Grok can be set up for another company's sign-in and server, and keeps one
//! sign-in per sign-in service in its `auth.json`. Teryon follows the same
//! settings Grok does, so it only ever sends the sign-in Grok itself would
//! use, to the server Grok itself would send it to. When Grok runs on an API
//! key instead (`XAI_API_KEY`), no subscription's limits apply, and none are read.
//!
//! The servers' addresses are not a promised interface, so every failure is
//! soft: the last reading stays, marked failed.

use std::env::var;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{DateTime, Utc};
use directories::BaseDirs;
use reqwest::header::ACCEPT;
use reqwest::{Client, Error as ReqwestError, StatusCode};
use serde_json::{from_slice, from_str, Error as JsonError, Value};
use thiserror::Error;
use tokio::join;
use tokio::process::Command;
use tokio::time::timeout;
use toml::Table;
use tracing::warn;

use crate::agents::runner::program_path;
use crate::agents::Agent;
use crate::usage::limits::sorted;
use crate::usage::{Limits, Reading, Window};

/// How long reading Grok's limits may take altogether before it is given up on.
pub const GROK_PATIENCE: Duration = Duration::from_secs(10);

/// Grok's own server, unless `GROK_CLI_CHAT_PROXY_BASE_URL` names another.
const DEFAULT_SERVER: &str = "https://cli-chat-proxy.grok.com/v1";

/// Grok's own sign-in service, unless Grok is set up for another company's.
const DEFAULT_ISSUER: &str = "https://auth.x.ai";

/// The version Grok's servers are told when `grok --version` does not say.
const KNOWN_VERSION: &str = "1.0.92";

/// What to do when Grok saved no sign-in Teryon can use.
pub const SIGN_IN_ADVICE: &str = "Run grok login in a terminal so Teryon can read Grok's limits.";

/// What to do when Grok's saved sign-in expired or was refused.
pub const EXPIRED_ADVICE: &str =
    "Grok's sign-in expired. Run grok login in a terminal to read its limits again.";

/// Where Grok keeps its files: the setting, else `GROK_HOME`, else `.grok` in the home folder.
pub fn grok_home(
    setting: Option<&Path>,
    variable: Option<String>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    setting
        .map(Path::to_path_buf)
        .or_else(|| {
            variable
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| home.map(|home| home.join(".grok")))
}

/// [`grok_home`] for this machine.
pub fn grok_home_here(setting: Option<&Path>) -> Option<PathBuf> {
    grok_home(
        setting,
        var("GROK_HOME").ok(),
        BaseDirs::new().map(|base| base.home_dir().to_path_buf()),
    )
}

/// Which server and sign-in Grok uses, as its settings say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrokSetup {
    /// Where Grok's servers are, such as `https://cli-chat-proxy.grok.com/v1`.
    pub server: String,
    /// The sign-in service Grok signs in with.
    pub issuer: String,
    /// Grok's name at that service, when its settings give one.
    pub client: Option<String>,
    /// Whether Grok runs on an API key, which takes precedence over any sign-in.
    pub api_key: bool,
}

impl GrokSetup {
    /// From Grok's environment variables and the text of its `config.toml`.
    ///
    /// A variable wins over the file. Only the file in Grok's own folder is
    /// read: a project's settings must not choose whose sign-in is sent.
    pub fn from(variable: impl Fn(&str) -> Option<String>, config: Option<&str>) -> Self {
        let variable = |name: &str| variable(name).filter(|value| !value.trim().is_empty());
        let config: Table = config
            .and_then(|text| text.parse().ok())
            .unwrap_or_default();
        let oidc = |key: &str| {
            config
                .get("grok_com_config")
                .and_then(|section| section.get("oidc"))
                .and_then(|oidc| oidc.get(key))
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        };
        Self {
            server: variable("GROK_CLI_CHAT_PROXY_BASE_URL")
                .unwrap_or_else(|| DEFAULT_SERVER.to_string()),
            issuer: variable("GROK_OIDC_ISSUER")
                .or_else(|| oidc("issuer"))
                .unwrap_or_else(|| DEFAULT_ISSUER.to_string()),
            client: variable("GROK_OIDC_CLIENT_ID").or_else(|| oidc("client_id")),
            api_key: variable("XAI_API_KEY").is_some(),
        }
    }

    /// [`GrokSetup::from`] for this machine, with Grok's folder at `home`.
    pub fn here(home: &Path) -> Self {
        let config = read_to_string(home.join("config.toml")).ok();
        Self::from(|name| var(name).ok(), config.as_deref())
    }

    /// The full address of one of the server's pages, such as `settings`.
    fn address(&self, page: &str) -> String {
        format!("{}/{page}", self.server.trim_end_matches('/'))
    }
}

/// The sign-in Grok saved, which Teryon borrows to ask for Grok's limits.
#[derive(Clone, Debug, PartialEq)]
pub struct GrokSignIn {
    pub token: String,
    pub user: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Why no sign-in could be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoSignIn {
    /// Grok saved none for its sign-in service.
    Missing,
    /// Grok saved several for its sign-in service, and its settings do not say which it uses.
    Several,
}

/// The sign-in Grok uses, from its `auth.json`.
///
/// Grok keeps one entry per sign-in service, named `<service>::<client>`
/// and repeating both inside as `oidc_issuer` and `oidc_client_id`. Only an
/// entry of the service in `setup` is taken, and of its client when the
/// setup names one. API keys (`sk-…`, `xai-…`) are passed over, since only a
/// signed-in subscriber is told their limits.
pub fn grok_sign_in(text: &str, setup: &GrokSetup) -> Result<GrokSignIn, NoSignIn> {
    let entries: Value = from_str(text).map_err(|_| NoSignIn::Missing)?;
    let entries = entries.as_object().ok_or(NoSignIn::Missing)?;
    let same = |one: &str, other: &str| one.trim_end_matches('/') == other.trim_end_matches('/');
    let mut found = entries.iter().filter_map(|(name, entry)| {
        let (named_issuer, named_client) = name.split_once("::").unwrap_or((name, ""));
        let issuer = entry["oidc_issuer"].as_str().unwrap_or(named_issuer);
        let client = entry["oidc_client_id"].as_str().unwrap_or(named_client);
        if !same(issuer, &setup.issuer)
            || setup
                .client
                .as_deref()
                .is_some_and(|wanted| wanted != client)
            || !matches!(
                entry["auth_mode"].as_str().unwrap_or(""),
                "" | "oidc" | "oauth"
            )
        {
            return None;
        }
        let token = entry["key"].as_str()?.trim();
        if token.is_empty() || token.starts_with("sk-") || token.starts_with("xai-") {
            return None;
        }
        Some(GrokSignIn {
            token: token.to_string(),
            user: ["user_id", "principal_id"]
                .into_iter()
                .filter_map(|field| entry[field].as_str())
                .find(|user| !user.is_empty())
                .map(str::to_string),
            expires_at: entry["expires_at"].as_str().and_then(rfc3339),
        })
    });
    match (found.next(), found.next()) {
        (Some(sign_in), None) => Ok(sign_in),
        (Some(_), Some(_)) => Err(NoSignIn::Several),
        (None, _) => Err(NoSignIn::Missing),
    }
}

/// A time written as in `2026-11-27T06:47:45.236130+00:00`.
fn rfc3339(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// An amount Grok gives in cents, as `{"val": 1250}`.
fn cents(value: &Value) -> Option<f64> {
    value["val"]
        .as_f64()
        .or_else(|| value["val"].as_str().and_then(|text| text.parse().ok()))
}

/// Grok's limits, from the answers of its billing and settings pages.
///
/// The billing answer's `config` has the share of the period's credits used
/// (`creditUsagePercent`, 0 to 100), the period (`currentPeriod`, with its
/// `type` and `end`), and, when extra usage is turned on, its cap and what is
/// used of it in cents. A period without a share means none is used yet.
/// The plan comes from the settings answer. Nothing when no limit is in it.
pub fn grok_limits(billing: &Value, settings: &Value, now: DateTime<Utc>) -> Option<Limits> {
    let config = billing.get("config").filter(|config| config.is_object())?;
    let period = &config["currentPeriod"];
    let resets_at = period["end"]
        .as_str()
        .or_else(|| config["billingPeriodEnd"].as_str())
        .and_then(rfc3339);
    let period_known = period.is_object() || config["billingPeriodEnd"].is_string();
    let share = match &config["creditUsagePercent"] {
        Value::Null if period_known => Some(0.0),
        value => value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok())),
    };
    let mut windows = Vec::new();
    if let Some(share) = share {
        let kind = period["type"].as_str().unwrap_or("").to_uppercase();
        let (name, label, minutes) = if kind.contains("DAY") || kind.contains("DAILY") {
            ("daily", "day", Some(1440))
        } else if kind.contains("MONTH") {
            ("monthly", "month", None)
        } else {
            ("weekly", "week", Some(10080))
        };
        let used = share.clamp(0.0, 100.0);
        windows.push(Window {
            name: name.to_string(),
            label: label.to_string(),
            minutes,
            used,
            resets_at,
            reached: used >= 100.0,
        });
    }
    if let (Some(cap), Some(spent)) = (
        cents(&config["onDemandCap"]),
        cents(&config["onDemandUsed"]),
    ) {
        if cap > 0.0 {
            let used = (spent / cap * 100.0).clamp(0.0, 100.0);
            windows.push(Window {
                name: "on_demand".to_string(),
                label: "extra usage".to_string(),
                minutes: None,
                used,
                resets_at,
                reached: used >= 100.0,
            });
        }
    }
    if windows.is_empty() {
        return None;
    }
    Some(Limits {
        agent: Agent::Grok.name().to_string(),
        plan: grok_plan(settings),
        windows: sorted(windows),
        taken_at: now,
        failed: false,
        advice: None,
    })
}

/// The plan in Grok's settings answer, `SuperGrokPlus`, written as a person would: "SuperGrok Plus".
fn grok_plan(settings: &Value) -> Option<String> {
    let plan = [
        "subscription_tier_display",
        "subscriptionTierDisplay",
        "subscription_tier",
    ]
    .into_iter()
    .filter_map(|key| settings[key].as_str())
    .map(str::trim)
    .find(|plan| !plan.is_empty() && !plan.chars().all(|letter| letter.is_ascii_digit()))?;
    Some(match plan.strip_prefix("SuperGrok") {
        Some(rest) if !rest.is_empty() && !rest.starts_with(' ') => format!("SuperGrok {rest}"),
        _ => plan.to_string(),
    })
}

/// The version `grok --version` prints, as in `grok 1.0.92 (b06756fd6256) [alpha]`.
fn grok_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .nth(1)
        .filter(|version| version.starts_with(|letter: char| letter.is_ascii_digit()))
        .map(str::to_string)
}

/// The version of the Grok program, which Grok's servers expect to be told.
async fn program_version(program: &str) -> Option<String> {
    let mut command = Command::new(program_path(program));
    command
        .arg("--version")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = command.output().await.ok()?;
    grok_version(&String::from_utf8_lossy(&output.stdout))
}

/// Why asking Grok's servers did not give an answer.
#[derive(Debug, Error)]
enum AskError {
    #[error("Grok's servers refused its saved sign-in")]
    Refused,
    #[error("Grok's servers answered {0}")]
    Status(StatusCode),
    #[error("could not reach Grok's servers: {0}")]
    Unreachable(#[from] ReqwestError),
    #[error("Grok's servers did not answer with JSON: {0}")]
    NotJson(#[from] JsonError),
}

/// What one of Grok's pages answers, asked as Grok itself asks.
async fn ask(
    client: &Client,
    address: &str,
    sign_in: &GrokSignIn,
    version: &str,
) -> Result<Value, AskError> {
    let mut request = client
        .get(address)
        .bearer_auth(&sign_in.token)
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header(ACCEPT, "application/json")
        .header("x-grok-client-version", version);
    if let Some(user) = &sign_in.user {
        request = request.header("x-userid", user);
    }
    let response = request.send().await?;
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(AskError::Refused);
    }
    if !status.is_success() {
        return Err(AskError::Status(status));
    }
    Ok(from_slice(&response.bytes().await?)?)
}

/// Grok's limits, asked of the server in `setup` with `sign_in`.
///
/// The billing and settings pages are asked at once. The plan is only a name
/// beside the limits, so the limits stand when the settings page fails.
pub async fn ask_limits(
    client: &Client,
    setup: &GrokSetup,
    sign_in: &GrokSignIn,
    version: &str,
    now: DateTime<Utc>,
) -> Reading {
    let billing_address = setup.address("billing?format=credits");
    let settings_address = setup.address("settings");
    let (billing, settings) = join!(
        ask(client, &billing_address, sign_in, version),
        ask(client, &settings_address, sign_in, version),
    );
    let billing = match billing {
        Ok(billing) => billing,
        Err(AskError::Refused) => {
            return Reading::Failed {
                advice: Some(EXPIRED_ADVICE.to_string()),
            }
        }
        Err(problem) => {
            warn!("could not read Grok's limits: {problem}");
            return Reading::Failed { advice: None };
        }
    };
    match grok_limits(&billing, &settings.unwrap_or(Value::Null), now) {
        Some(limits) => Reading::Fresh(limits),
        None => {
            warn!("could not read Grok's limits: its billing answer names none");
            Reading::Failed { advice: None }
        }
    }
}

/// Grok's limits, read with the sign-in Grok saved in `home`, within [`GROK_PATIENCE`].
///
/// When signing in again would help, the reading says so. Teryon runs this
/// after a Grok turn has finished, so a slow server never holds up the thread.
pub async fn grok_reading(program: &str, home: Option<&Path>, now: DateTime<Utc>) -> Reading {
    match timeout(GROK_PATIENCE, read(program, home, now)).await {
        Ok(reading) => reading,
        Err(_) => {
            warn!(
                "could not read Grok's limits: Grok's servers took longer than {} seconds",
                GROK_PATIENCE.as_secs()
            );
            Reading::Failed { advice: None }
        }
    }
}

async fn read(program: &str, home: Option<&Path>, now: DateTime<Utc>) -> Reading {
    let Some(home) = home else {
        warn!("could not read Grok's limits: no home folder to find its sign-in in");
        return Reading::Failed { advice: None };
    };
    let setup = GrokSetup::here(home);
    if setup.api_key {
        return Reading::Nothing;
    }
    let text = read_to_string(home.join("auth.json")).unwrap_or_default();
    let sign_in = match grok_sign_in(&text, &setup) {
        Ok(sign_in) => sign_in,
        Err(NoSignIn::Missing) => {
            return Reading::Failed {
                advice: Some(SIGN_IN_ADVICE.to_string()),
            }
        }
        Err(NoSignIn::Several) => {
            warn!(
                "could not read Grok's limits: Grok saved several sign-ins for {}, and its settings do not name the client it uses",
                setup.issuer
            );
            return Reading::Failed { advice: None };
        }
    };
    if sign_in.expires_at.is_some_and(|expiry| expiry <= now) {
        return Reading::Failed {
            advice: Some(EXPIRED_ADVICE.to_string()),
        };
    }
    let version = program_version(program)
        .await
        .unwrap_or_else(|| KNOWN_VERSION.to_string());
    match Client::builder().build() {
        Ok(client) => ask_limits(&client, &setup, &sign_in, &version, now).await,
        Err(problem) => {
            warn!("could not read Grok's limits: {problem}");
            Reading::Failed { advice: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::write;
    use std::sync::{Arc, Mutex};

    use chrono::{TimeDelta, TimeZone};
    use lets_expect::lets_expect;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::sleep;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap()
    }

    fn failed(advice: Option<&str>) -> Reading {
        Reading::Failed {
            advice: advice.map(str::to_string),
        }
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

    /// Grok's billing answer with the given `config`.
    fn billing(config: Value) -> Value {
        json!({ "config": config })
    }

    /// A week's period, ending on 27 November.
    fn week() -> Value {
        json!({ "type": "BILLING_PERIOD_TYPE_WEEKLY", "end": "2026-11-27T06:47:45.236130+00:00" })
    }

    fn resets() -> Option<DateTime<Utc>> {
        Some(
            Utc.with_ymd_and_hms(2026, 11, 27, 6, 47, 45).unwrap()
                + TimeDelta::microseconds(236130),
        )
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

    /// Grok's plan from a settings answer, through a billing answer with one limit.
    fn plan_of(settings: Value) -> Option<String> {
        grok_limits(
            &billing(json!({ "creditUsagePercent": 1, "currentPeriod": week() })),
            &settings,
            now(),
        )
        .and_then(|limits| limits.plan)
    }

    /// Grok's setup with the given environment variables and `config.toml`.
    fn setup_with(variables: &[(&str, &str)], config: Option<&str>) -> GrokSetup {
        let variables: HashMap<String, String> = variables
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        GrokSetup::from(|name| variables.get(name).cloned(), config)
    }

    /// Grok's setup when nothing is set: its own server and sign-in service.
    fn plain() -> GrokSetup {
        setup_with(&[], None)
    }

    fn setup(server: &str, issuer: &str, client: Option<&str>, api_key: bool) -> GrokSetup {
        GrokSetup {
            server: server.to_string(),
            issuer: issuer.to_string(),
            client: client.map(str::to_string),
            api_key,
        }
    }

    /// The setup of a company signing in with its own service.
    fn acme(client: Option<&str>) -> GrokSetup {
        setup(DEFAULT_SERVER, "https://acme.okta.com", client, false)
    }

    /// An entry of Grok's `auth.json`, the way Grok writes it.
    fn entry(issuer: &str, client: &str, token: &str, user: &str) -> (String, Value) {
        (
            format!("{issuer}::{client}"),
            json!({
                "auth_mode": "oidc", "key": token, "user_id": user,
                "oidc_issuer": issuer, "oidc_client_id": client,
                "expires_at": "2026-11-27T00:00:00Z",
            }),
        )
    }

    fn auth(entries: Vec<(String, Value)>) -> String {
        Value::Object(entries.into_iter().collect()).to_string()
    }

    fn sign_in(token: &str, user: Option<&str>, expires_at: Option<&str>) -> GrokSignIn {
        GrokSignIn {
            token: token.to_string(),
            user: user.map(str::to_string),
            expires_at: expires_at.and_then(rfc3339),
        }
    }

    /// Whose token is chosen from `entries` with `setup`.
    fn chosen(entries: Vec<(String, Value)>, setup: GrokSetup) -> Result<String, NoSignIn> {
        grok_sign_in(&auth(entries), &setup).map(|sign_in| sign_in.token)
    }

    /// Grok's reading with a home holding `file` as its `auth.json`, when `file` is set.
    async fn reading_with(home: bool, file: Option<String>) -> Reading {
        let folder = tempdir().unwrap();
        if let Some(file) = file {
            write(folder.path().join("auth.json"), file).unwrap();
        }
        grok_reading("teryon-no-such-grok", home.then_some(folder.path()), now()).await
    }

    /// How a stand-in for Grok's server answers one page.
    #[derive(Clone)]
    struct Answer {
        status: u16,
        body: String,
        delay: Duration,
    }

    fn answer(body: Value) -> Answer {
        Answer {
            status: 200,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16) -> Answer {
        Answer {
            status,
            body: String::new(),
            delay: Duration::ZERO,
        }
    }

    fn text(body: &str) -> Answer {
        Answer {
            status: 200,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    /// An answer that comes after the client has given up.
    fn slow(body: Value) -> Answer {
        Answer {
            delay: Duration::from_secs(5),
            ..answer(body)
        }
    }

    fn week_used(share: u32) -> Answer {
        answer(billing(
            json!({ "creditUsagePercent": share, "currentPeriod": week() }),
        ))
    }

    fn plus() -> Answer {
        answer(json!({ "subscription_tier_display": "SuperGrokPlus" }))
    }

    /// A stand-in for Grok's server on this machine, answering its billing
    /// and settings pages as told. Returns its address and the heads of the
    /// requests it got.
    async fn fake_server(billing: Answer, settings: Answer) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/v1/", listener.local_addr().unwrap());
        let heads = Arc::new(Mutex::new(Vec::new()));
        let kept = heads.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (billing, settings, kept) = (billing.clone(), settings.clone(), kept.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut buffer = [0; 1024];
                    while !head.windows(4).any(|end| end == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => head.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    let page = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    kept.lock().unwrap().push(head);
                    let answer = match page.as_str() {
                        "/v1/billing?format=credits" => billing,
                        "/v1/settings" => settings,
                        _ => status(404),
                    };
                    sleep(answer.delay).await;
                    let reply = format!(
                        "HTTP/1.1 {} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        answer.status,
                        answer.body.len(),
                        answer.body
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        (address, heads)
    }

    fn client() -> Client {
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(300))
            .build()
            .unwrap()
    }

    /// What asking a stand-in server that answers as told gives.
    async fn asked(billing: Answer, settings: Answer) -> Reading {
        let (address, _) = fake_server(billing, settings).await;
        let setup = setup(&address, DEFAULT_ISSUER, None, false);
        ask_limits(
            &client(),
            &setup,
            &sign_in("token", Some("me"), None),
            "1.0.92",
            now(),
        )
        .await
    }

    /// The headers the stand-in server got with each page, lowercase names
    /// sorted, leaving out those every client sends anyway.
    async fn headers_sent(user: Option<&str>) -> Vec<Vec<String>> {
        let (address, heads) = fake_server(week_used(10), plus()).await;
        let setup = setup(&address, DEFAULT_ISSUER, None, false);
        ask_limits(
            &client(),
            &setup,
            &sign_in("token", user, None),
            "1.0.92",
            now(),
        )
        .await;
        let mut sent: Vec<Vec<String>> = heads
            .lock()
            .unwrap()
            .iter()
            .map(|head| {
                let mut lines: Vec<String> = head
                    .lines()
                    .filter_map(|line| line.split_once(": "))
                    .map(|(name, value)| format!("{}: {value}", name.to_lowercase()))
                    .filter(|line| !line.starts_with("host:"))
                    .collect();
                lines.sort();
                lines.insert(
                    0,
                    head.split_whitespace()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                lines
            })
            .collect();
        sent.sort();
        sent
    }

    fn grok_headers(page: &str, user: Option<&str>) -> Vec<String> {
        let mut lines = vec![
            "accept: application/json".to_string(),
            "authorization: Bearer token".to_string(),
            "x-grok-client-version: 1.0.92".to_string(),
            "x-xai-token-auth: xai-grok-cli".to_string(),
        ];
        if let Some(user) = user {
            lines.push(format!("x-userid: {user}"));
        }
        lines.sort();
        lines.insert(0, format!("GET /v1/{page}"));
        lines
    }

    /// A reading of the week at `share`, with the plan when `plan` is set.
    fn fresh(share: f64, plan: bool) -> Reading {
        Reading::Fresh(Limits {
            agent: "grok".to_string(),
            plan: plan.then(|| "SuperGrok Plus".to_string()),
            windows: vec![Window {
                name: "weekly".to_string(),
                label: "week".to_string(),
                minutes: Some(10080),
                used: share,
                resets_at: resets(),
                reached: false,
            }],
            taken_at: now(),
            failed: false,
            advice: None,
        })
    }

    lets_expect! {
        // requirement: Grok's billing answer becomes its period's window, plus extra usage when it has a cap
        expect(shape(grok_limits(&answer, &Value::Null, now()))) as groks_reading {
            let answer = billing(json!({ "creditUsagePercent": 10, "currentPeriod": week() }));

            when(answer = billing(json!({ "creditUsagePercent": 10, "currentPeriod": week() }))) as with_a_share_of_the_week to give_the_week {
                equal(Some(vec![named("weekly", 10.0, false)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": "12.5", "currentPeriod": week() }))) as with_the_share_as_text to read_it {
                equal(Some(vec![named("weekly", 12.5, false)]))
            }
            when(answer = billing(json!({ "currentPeriod": week() }))) as with_a_period_but_no_share to give_none_used {
                equal(Some(vec![named("weekly", 0.0, false)]))
            }
            when(answer = billing(json!({ "billingPeriodEnd": "2026-11-27T06:47:45Z" }))) as with_only_the_periods_end to give_none_used_of_a_week {
                equal(Some(vec![named("weekly", 0.0, false)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 100, "currentPeriod": week() }))) as with_the_week_used_up to mark_it_reached {
                equal(Some(vec![named("weekly", 100.0, true)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 140, "currentPeriod": week() }))) as with_more_than_all_used to keep_it_at_all {
                equal(Some(vec![named("weekly", 100.0, true)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 5, "currentPeriod": { "type": "BILLING_PERIOD_TYPE_DAILY" } }))) as with_a_day to give_the_day {
                equal(Some(vec![named("daily", 5.0, false)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 5, "currentPeriod": { "type": "BILLING_PERIOD_TYPE_MONTHLY" } }))) as with_a_month to give_the_month {
                equal(Some(vec![named("monthly", 5.0, false)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 10, "currentPeriod": week(), "onDemandCap": { "val": 2000 }, "onDemandUsed": { "val": 500 } }))) as with_extra_usage to put_it_after_the_week {
                equal(Some(vec![named("weekly", 10.0, false), named("on_demand", 25.0, false)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 10, "currentPeriod": week(), "onDemandCap": { "val": 2000 }, "onDemandUsed": { "val": 2000 } }))) as with_extra_usage_used_up to mark_it_reached {
                equal(Some(vec![named("weekly", 10.0, false), named("on_demand", 100.0, true)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": 10, "currentPeriod": week(), "onDemandCap": { "val": 0 }, "onDemandUsed": { "val": 0 } }))) as with_extra_usage_turned_off to leave_it_out {
                equal(Some(vec![named("weekly", 10.0, false)]))
            }
            when(answer = billing(json!({ "onDemandCap": { "val": "1000" }, "onDemandUsed": { "val": "100" } }))) as with_only_extra_usage to give_it {
                equal(Some(vec![named("on_demand", 10.0, false)]))
            }
            when(answer = billing(json!({ "creditUsagePercent": "much" }))) as with_an_unreadable_share to give_nothing { equal(None) }
            when(answer = billing(json!({}))) as with_an_empty_config to give_nothing { equal(None) }
            when(answer = json!({ "config": "none" })) as with_a_config_that_is_no_object to give_nothing { equal(None) }
            when(answer = json!({})) as without_a_config to give_nothing { equal(None) }
            when(answer = Value::Null) as from_no_json_object to give_nothing { equal(None) }
        }

        expect(facts(grok_limits(&billing(json!({ "creditUsagePercent": 10, "currentPeriod": week(), "onDemandCap": { "val": 2000 }, "onDemandUsed": { "val": 500 } })), &json!({ "subscription_tier_display": "SuperGrokPlus" }), now()).unwrap())) as the_grok_reading {
            to give_everything_else {
                equal(Facts {
                    agent: "grok".to_string(),
                    plan: Some("SuperGrok Plus".to_string()),
                    labels: vec!["week".to_string(), "extra usage".to_string()],
                    minutes: vec![Some(10080), None],
                    resets: vec![resets(), resets()],
                    taken_at: now(),
                })
            }
        }

        // requirement: Grok's plan is its settings' display name, written with a space after "SuperGrok"
        expect(plan_of(settings)) as groks_plan {
            let settings = Value::Null;

            when(settings = Value::Null) as without_settings to be_none { equal(None) }
            when(settings = json!({ "subscription_tier_display": "SuperGrokPlus" })) as of_supergrok_plus to get_a_space { equal(Some("SuperGrok Plus".to_string())) }
            when(settings = json!({ "subscriptionTierDisplay": "SuperGrokHeavy" })) as named_the_other_way to be_read { equal(Some("SuperGrok Heavy".to_string())) }
            when(settings = json!({ "subscription_tier_display": "SuperGrok" })) as of_plain_supergrok to stay { equal(Some("SuperGrok".to_string())) }
            when(settings = json!({ "subscription_tier_display": "SuperGrok Pro" })) as already_spaced to stay { equal(Some("SuperGrok Pro".to_string())) }
            when(settings = json!({ "subscription_tier_display": "  ", "subscription_tier": "Basic" })) as with_an_empty_display_name to take_the_tier { equal(Some("Basic".to_string())) }
            when(settings = json!({ "subscription_tier": "3" })) as given_as_a_number to be_none { equal(None) }
        }

        // requirement: Teryon uses the server and sign-in service Grok's settings name, variables before config.toml
        expect(setup_with(&variables, config)) as groks_setup {
            let variables: Vec<(&str, &str)> = vec![];
            let config: Option<&str> = None;

            when(variables = vec![]) as with_nothing_set to be_groks_own { equal(setup(DEFAULT_SERVER, DEFAULT_ISSUER, None, false)) }
            when(variables = vec![("GROK_CLI_CHAT_PROXY_BASE_URL", "https://grok-proxy.acme.com/v1")]) as with_another_server to use_it {
                equal(setup("https://grok-proxy.acme.com/v1", DEFAULT_ISSUER, None, false))
            }
            when(config = Some("[grok_com_config.oidc]\nissuer = \"https://acme.okta.com\"\nclient_id = \"teryon\"\n")) as with_a_service_in_config to use_it {
                equal(setup(DEFAULT_SERVER, "https://acme.okta.com", Some("teryon"), false))
            }
            when(variables = vec![("GROK_OIDC_ISSUER", "https://sso.example.com"), ("GROK_OIDC_CLIENT_ID", "other")], config = Some("[grok_com_config.oidc]\nissuer = \"https://acme.okta.com\"\nclient_id = \"teryon\"\n")) as with_variables_and_config to prefer_the_variables {
                equal(setup(DEFAULT_SERVER, "https://sso.example.com", Some("other"), false))
            }
            when(variables = vec![("GROK_OIDC_ISSUER", " "), ("GROK_CLI_CHAT_PROXY_BASE_URL", "")]) as with_empty_variables to ignore_them { equal(plain()) }
            when(config = Some("not = [toml")) as with_an_unreadable_config to be_groks_own { equal(plain()) }
            when(config = Some("[grok_com_config]\noidc = 3\n")) as with_a_config_of_another_shape to be_groks_own { equal(plain()) }
            when(variables = vec![("XAI_API_KEY", "xai-key")]) as with_an_api_key to say_so { equal(setup(DEFAULT_SERVER, DEFAULT_ISSUER, None, true)) }
        }

        // requirement: only Grok's sign-in for the service in its settings is used, never another account's or an API key
        expect(chosen(entries, setup)) as the_chosen_sign_in {
            let entries = vec![entry(DEFAULT_ISSUER, "grok-cli", "own", "me")];
            let setup = plain();

            when(entries = vec![entry(DEFAULT_ISSUER, "grok-cli", "own", "me")]) as with_groks_own to take_it { equal(Ok("own".to_string())) }
            when(entries = vec![entry("https://auth.x.ai/", "grok-cli", "own", "me")]) as with_a_slash_after_the_service to take_it { equal(Ok("own".to_string())) }
            when(entries = vec![entry("https://acme.okta.com", "grok-cli", "work", "me")]) as with_another_services_only to find_none { equal(Err(NoSignIn::Missing)) }
            when(entries = vec![entry("https://acme.okta.com", "grok-cli", "work", "me"), entry(DEFAULT_ISSUER, "grok-cli", "own", "me")]) as with_two_services to take_the_one_in_the_settings { equal(Ok("own".to_string())) }
            when(entries = vec![entry("https://acme.okta.com", "grok-cli", "work", "me"), entry(DEFAULT_ISSUER, "grok-cli", "own", "me")], setup = acme(None)) as with_two_services_set_for_the_other to take_the_others { equal(Ok("work".to_string())) }
            when(entries = vec![entry("https://acme.okta.com", "one", "first", "me"), entry("https://acme.okta.com", "two", "second", "me")], setup = acme(None)) as with_two_clients_and_none_set to refuse_to_guess { equal(Err(NoSignIn::Several)) }
            when(entries = vec![entry("https://acme.okta.com", "one", "first", "me"), entry("https://acme.okta.com", "two", "second", "me")], setup = acme(Some("two"))) as with_two_clients_and_one_set to take_it { equal(Ok("second".to_string())) }
            when(entries = vec![entry("https://acme.okta.com", "one", "first", "me")], setup = acme(Some("two"))) as with_another_client_only to find_none { equal(Err(NoSignIn::Missing)) }
            when(entries = vec![(format!("{DEFAULT_ISSUER}::grok-cli"), json!({ "key": "named" }))]) as with_the_service_only_in_the_name to take_it { equal(Ok("named".to_string())) }
            when(entries = vec![(format!("{DEFAULT_ISSUER}::grok-cli"), json!({ "auth_mode": "api_key", "key": "token" }))]) as with_an_api_key_mode to find_none { equal(Err(NoSignIn::Missing)) }
            when(entries = vec![entry(DEFAULT_ISSUER, "a", "sk-secret", "me"), entry(DEFAULT_ISSUER, "b", "xai-secret", "me"), entry(DEFAULT_ISSUER, "c", "  ", "me")]) as with_only_keys_and_empty_tokens to find_none { equal(Err(NoSignIn::Missing)) }
            when(entries = vec![]) as with_none to find_none { equal(Err(NoSignIn::Missing)) }
        }

        expect(grok_sign_in(&text, &plain())) as the_sign_in_read {
            let text = String::new();

            when(text = auth(vec![entry(DEFAULT_ISSUER, "grok-cli", "own", "me")])) as from_groks_entry to give_its_user_and_expiry {
                equal(Ok(sign_in("own", Some("me"), Some("2026-11-27T00:00:00Z"))))
            }
            when(text = auth(vec![(DEFAULT_ISSUER.to_string(), json!({ "key": "own", "principal_id": "someone", "user_id": "" }))])) as with_a_principal_instead to give_the_principal {
                equal(Ok(sign_in("own", Some("someone"), None)))
            }
            when(text = String::new()) as from_an_empty_file to find_none { equal(Err(NoSignIn::Missing)) }
            when(text = "[]".to_string()) as from_a_list to find_none { equal(Err(NoSignIn::Missing)) }
        }

        // requirement: the version is the second word of grok --version
        expect(grok_version(output)) as groks_version {
            when(output = "grok 1.0.92 (b06756fd6256) [alpha]\n") as from_grok to be_read { equal(Some("1.0.92".to_string())) }
            when(output = "grok") as without_one to be_none { equal(None) }
            when(output = "grok version unknown") as that_is_no_number to be_none { equal(None) }
        }

        // requirement: Grok's home is the setting, else GROK_HOME, else ~/.grok
        expect(grok_home(setting.as_deref(), variable.clone(), Some(PathBuf::from("/home/someone")))) as groks_home {
            let setting: Option<PathBuf> = None;
            let variable: Option<String> = None;

            when(variable = None) as by_default to be_in_the_home_folder { equal(Some(PathBuf::from("/home/someone/.grok"))) }
            when(variable = Some("/elsewhere".to_string())) as with_grok_home_set to follow_it { equal(Some(PathBuf::from("/elsewhere"))) }
            when(variable = Some(String::new())) as with_grok_home_empty to ignore_it { equal(Some(PathBuf::from("/home/someone/.grok"))) }
            when(setting = Some(PathBuf::from("/set")), variable = Some("/elsewhere".to_string())) as with_a_setting to follow_the_setting { equal(Some(PathBuf::from("/set"))) }
        }
    }

    lets_expect! { #tokio_test
        // requirement: without a usable sign-in Grok's servers are not asked, and the reading says to sign in
        expect(reading_with(home, file.clone()).await) as groks_reading_without_asking {
            let home = true;
            let file: Option<String> = None;

            when(file = None) as without_a_sign_in_file to advise_signing_in { equal(failed(Some(SIGN_IN_ADVICE))) }
            when(file = Some("not json".to_string())) as with_an_unreadable_file to advise_signing_in { equal(failed(Some(SIGN_IN_ADVICE))) }
            when(file = Some(auth(vec![entry("https://acme.okta.com", "grok-cli", "work", "me")]))) as with_another_services_sign_in_only to advise_signing_in { equal(failed(Some(SIGN_IN_ADVICE))) }
            when(file = Some(auth(vec![(DEFAULT_ISSUER.to_string(), json!({ "key": "token", "expires_at": "2026-11-20T11:00:00Z" }))]))) as with_an_expired_sign_in to advise_signing_in_again { equal(failed(Some(EXPIRED_ADVICE))) }
            when(file = Some(auth(vec![entry(DEFAULT_ISSUER, "one", "first", "me"), entry(DEFAULT_ISSUER, "two", "second", "me")]))) as with_two_sign_ins_for_the_service to fail_without_guessing { equal(failed(None)) }
            when(home = false) as without_a_home_folder to fail_without_advice { equal(failed(None)) }
        }

        // requirement: Grok's servers are asked as Grok asks them, and every failure is soft
        expect(asked(billing.clone(), settings.clone()).await) as asking_groks_server {
            let billing = week_used(10);
            let settings = plus();

            when(billing = week_used(10)) as when_both_pages_answer to give_the_limits_and_plan { equal(fresh(10.0, true)) }
            when(settings = status(500)) as when_the_settings_page_fails to give_the_limits_without_a_plan { equal(fresh(10.0, false)) }
            when(settings = slow(json!({ "subscription_tier_display": "SuperGrokPlus" }))) as when_the_settings_page_is_too_slow to give_the_limits_without_a_plan { equal(fresh(10.0, false)) }
            when(settings = text("not json")) as when_the_settings_page_is_no_json to give_the_limits_without_a_plan { equal(fresh(10.0, false)) }
            when(billing = status(401)) as when_the_sign_in_is_refused to advise_signing_in_again { equal(failed(Some(EXPIRED_ADVICE))) }
            when(billing = status(403)) as when_the_sign_in_is_forbidden to advise_signing_in_again { equal(failed(Some(EXPIRED_ADVICE))) }
            when(billing = status(500)) as when_the_billing_page_fails to fail_without_advice { equal(failed(None)) }
            when(billing = slow(json!({ "config": { "creditUsagePercent": 10 } }))) as when_the_billing_page_is_too_slow to fail_without_advice { equal(failed(None)) }
            when(billing = text("<html>")) as when_the_billing_page_is_no_json to fail_without_advice { equal(failed(None)) }
            when(billing = answer(json!({ "config": {} }))) as when_the_billing_page_names_no_limit to fail_without_advice { equal(failed(None)) }
        }

        // requirement: both pages are asked with Grok's sign-in, its user, and the headers Grok sends
        expect(headers_sent(user).await) as the_headers_sent {
            let user = Some("me");

            when(user = Some("me")) as with_a_user to send_it {
                equal(vec![grok_headers("billing?format=credits", Some("me")), grok_headers("settings", Some("me"))])
            }
            when(user = None) as without_a_user to leave_it_out {
                equal(vec![grok_headers("billing?format=credits", None), grok_headers("settings", None)])
            }
        }
    }
}
