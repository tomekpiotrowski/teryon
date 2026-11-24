//! Versioned models and effort levels offered by the installed agent programs.
//!
//! A configured list overrides discovery. Claude's local catalog is read
//! without starting a conversation. Other programs expose listing commands.

use std::env::var_os;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, from_value, json, Value};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::time::timeout;
use tracing::warn;
use ts_rs::TS;

use crate::agents::codex::KEY_VARIABLES;
use crate::agents::runner::program_path;
use crate::agents::Agent;
use crate::settings::{AgentSettings, ClaudeSettings, CodexSettings};

/// The short names Claude Code takes for its newest models.
const CLAUDE_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];

/// How long a program may take to list models before only its default is offered.
const LISTING_TIME: Duration = Duration::from_secs(10);

/// How long Antigravity may take to list Gemini models.
///
/// `agy models` starts its language server and fetches the catalog over the
/// network, which regularly takes longer than ten seconds on a cold start.
const GEMINI_LISTING_TIME: Duration = Duration::from_secs(30);

/// A provider's level of reasoning, in the order offered to the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

impl Effort {
    /// Every known effort, in increasing order.
    pub const ALL: [Self; 6] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
        Self::Ultra,
    ];

    /// Matches a name printed by a provider, including mixed-case labels.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|effort| effort.name().eq_ignore_ascii_case(name))
    }

    /// The spelling used in configuration and program arguments.
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    /// The wording shown in thread notices.
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Xhigh => "Extra high",
            Self::Max => "Max",
            Self::Ultra => "Ultra",
        }
    }
}

/// A versioned model and the efforts it supports.
///
/// Replaces plain model strings so every provider exposes a human-readable
/// version name, canonical effort levels, and default markers in one shape.
///
/// For grouped Antigravity Gemini variants, `id` is the shared base identifier
/// (for example `gemini-4-neon`) and the chosen effort suffix is restored when
/// invoking the program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub efforts: Vec<Effort>,
    pub default_effort: Option<Effort>,
    pub is_default: bool,
}

/// The catalog of each agent program.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ModelLists {
    pub claude: Vec<ModelOption>,
    pub codex: Vec<ModelOption>,
    pub grok: Vec<ModelOption>,
    pub gemini: Vec<ModelOption>,
}

impl ModelLists {
    /// The lists already known before discovery has finished.
    pub fn from_settings(settings: &AgentSettings) -> Self {
        let claude = settings
            .claude
            .models
            .clone()
            .unwrap_or_else(|| CLAUDE_MODELS.map(String::from).to_vec());
        Self {
            claude: named_models(&claude, Agent::Claude),
            codex: named_models(
                settings.codex.models.as_deref().unwrap_or_default(),
                Agent::Codex,
            ),
            grok: named_models(
                settings.grok.models.as_deref().unwrap_or_default(),
                Agent::Grok,
            ),
            gemini: group_gemini(
                settings
                    .gemini
                    .models
                    .iter()
                    .flatten()
                    .map(|id| (id.as_str(), "", false)),
            ),
        }
    }
}

/// Ensures one model is marked as the provider's preferred choice.
fn mark_default(models: &mut [ModelOption], preferred: impl Fn(&str) -> bool) {
    let chosen = models
        .iter()
        .position(|model| preferred(&model.id))
        .or_else(|| models.iter().position(|model| model.is_default))
        .unwrap_or(0);
    for (index, model) in models.iter_mut().enumerate() {
        model.is_default = index == chosen;
    }
}

/// Formats a versioned identifier when the provider supplies no display label.
fn formatted_model(id: &str) -> String {
    let id = id.strip_prefix("claude-").unwrap_or(id);
    let mut label = String::new();
    let mut numeric_version = false;
    for word in id.split('-') {
        let number = word.len() <= 2 && word.bytes().all(|byte| byte.is_ascii_digit());
        if !label.is_empty() {
            label.push(
                if label == "GPT" && word.starts_with(|c: char| c.is_ascii_digit()) {
                    '-'
                } else if numeric_version && number {
                    '.'
                } else {
                    ' '
                },
            );
        }
        if word.eq_ignore_ascii_case("gpt") {
            label.push_str("GPT");
        } else {
            let mut chars = word.chars();
            label.extend(chars.next().into_iter().flat_map(char::to_uppercase));
            label.extend(chars);
        }
        numeric_version = number;
    }
    label
}

/// Attaches standard efforts when a catalog entry supplies only a name.
fn named_models(ids: &[String], agent: Agent) -> Vec<ModelOption> {
    let (levels, default_effort) = match agent {
        Agent::Claude => (&Effort::ALL[..5], Some(Effort::Medium)),
        Agent::Grok => (&Effort::ALL[..4], Some(Effort::Medium)),
        Agent::Codex => (&Effort::ALL[..4], None),
        Agent::Gemini => (&Effort::ALL[..0], None),
    };
    let mut models = ids
        .iter()
        .filter_map(|id| checked_model(Some(id)).ok().flatten())
        .map(|id| ModelOption {
            label: formatted_model(&id),
            id,
            efforts: levels.to_vec(),
            default_effort,
            is_default: false,
        })
        .collect::<Vec<_>>();
    mark_default(&mut models, |_| false);
    models
}

/// Reads a provider's effort, warning about names a newer program may introduce.
fn listed_effort(provider: &str, name: &str) -> Option<Effort> {
    let effort = Effort::parse(name);
    if effort.is_none() {
        warn!(provider, name, "unrecognised model effort");
    }
    effort
}

/// Sorts supported efforts independently of a provider's listing order.
fn ordered_efforts(efforts: impl IntoIterator<Item = Effort>) -> Vec<Effort> {
    let mut efforts: Vec<_> = efforts.into_iter().collect();
    efforts.sort_unstable();
    efforts.dedup();
    efforts
}

/// Reads Claude's cached catalog, falling back to the built-in model aliases.
///
/// Claude Code has no non-interactive command to list models without starting
/// a conversation, so Teryon reads the local catalog cache instead.
///
/// Reads the newest JSON file in `<claude_home>/cache/model-catalog/`, keeps
/// entries in the `"main"` section, and marks the model configured in
/// `settings.json` as default. An explicit `models` list in `config.toml`
/// overrides discovery.
pub async fn list_claude_models(settings: &ClaudeSettings) -> Vec<ModelOption> {
    if let Some(ids) = &settings.models {
        return named_models(ids, Agent::Claude);
    }
    let home = settings
        .home
        .clone()
        .or_else(|| var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
        .or_else(|| BaseDirs::new().map(|base| base.home_dir().join(".claude")));
    if let Some(home) = home {
        let folder = home.join("cache/model-catalog");
        match read_claude_catalog(&folder).await {
            Ok(catalog) => {
                let configured = tokio::fs::read_to_string(home.join("settings.json"))
                    .await
                    .ok()
                    .and_then(|text| from_str::<Value>(&text).ok())
                    .and_then(|value| value["model"].as_str().map(str::to_string));
                let models = claude_models_in(&catalog, configured.as_deref());
                if !models.is_empty() {
                    return models;
                }
                warn!(path = %folder.display(), "Claude's model catalog has no main models");
            }
            Err(problem) => {
                warn!(path = %folder.display(), %problem, "could not read Claude's model catalog")
            }
        }
    } else {
        warn!("could not find Claude's configuration directory");
    }
    named_models(&CLAUDE_MODELS.map(String::from), Agent::Claude)
}

/// Reads the most recently updated catalog file.
async fn read_claude_catalog(folder: &Path) -> Result<Value, IoError> {
    let mut files = tokio::fs::read_dir(folder).await?;
    let mut newest = None;
    while let Some(file) = files.next_entry().await? {
        let path = file.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let modified = file.metadata().await?.modified()?;
            if newest
                .as_ref()
                .is_none_or(|(time, previous)| (&modified, &path) > (time, previous))
            {
                newest = Some((modified, path));
            }
        }
    }
    let (_, path) = newest.ok_or_else(|| IoError::new(ErrorKind::NotFound, "no catalog file"))?;
    let text = tokio::fs::read_to_string(path).await?;
    from_str(&text).map_err(|problem| IoError::new(ErrorKind::InvalidData, problem))
}

/// Keeps current Claude models and resolves the configured model or alias.
fn claude_models_in(catalog: &Value, configured: Option<&str>) -> Vec<ModelOption> {
    let mut models = catalog["catalog"]["config"]["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["section"] == "main")
        .filter_map(|entry| {
            let id = checked_model(entry["id"].as_str()).ok().flatten()?;
            let runtime = &entry["runtime"];
            let efforts = ordered_efforts(
                runtime["effort_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(|name| listed_effort("Claude", name)),
            );
            let default_effort = runtime["default_effort"]
                .as_str()
                .and_then(|name| listed_effort("Claude", name))
                .filter(|effort| efforts.contains(effort));
            Some(ModelOption {
                label: entry["name"]
                    .as_str()
                    .map_or_else(|| formatted_model(&id), str::to_string),
                id,
                efforts,
                default_effort,
                is_default: false,
            })
        })
        .collect::<Vec<_>>();
    mark_default(&mut models, |id| {
        configured.is_some_and(|name| {
            id == name
                || id
                    .strip_prefix("claude-")
                    .and_then(|rest| rest.split('-').next())
                    == Some(name)
        })
    });
    models
}

/// Why an agent's models could not be listed.
#[derive(Debug, Error)]
pub enum ListingError {
    #[error("could not start `{program}`: {source}")]
    Unstartable {
        program: String,
        #[source]
        source: IoError,
    },
    #[error("`{program}` did not list its models within {seconds} seconds")]
    TooSlow { program: String, seconds: u64 },
    #[error("`{program}` could not list its models: {complaint}")]
    Failed { program: String, complaint: String },
}

/// Asks Codex for its visible models without starting a conversation or a turn.
///
/// The app server uses the same program and subscription policy as Codex turns.
/// Its newline-delimited requests need an initialization handshake and may
/// paginate. The whole exchange is bounded, and the child is stopped afterward.
pub async fn list_codex_models(settings: &CodexSettings) -> Result<Vec<ModelOption>, ListingError> {
    let mut command = Command::new(program_path(&settings.program));
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if settings.use_subscription {
        for variable in KEY_VARIABLES {
            command.env_remove(variable);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|source| ListingError::Unstartable {
            program: settings.program.clone(),
            source,
        })?;
    let mut input = child.stdin.take().expect("piped stdin");
    let mut output = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let result = timeout(LISTING_TIME, codex_exchange(&mut input, &mut output)).await;
    // Kill and reap even after a protocol error or timeout. kill_on_drop also
    // handles cancellation by the caller while a response is being read.
    let _ = child.kill().await;
    result
        .map_err(|_| ListingError::TooSlow {
            program: settings.program.clone(),
            seconds: LISTING_TIME.as_secs(),
        })?
        .map_err(|problem| ListingError::Failed {
            program: settings.program.clone(),
            complaint: problem.to_string(),
        })
}

async fn codex_exchange(
    input: &mut ChildStdin,
    output: &mut Lines<BufReader<ChildStdout>>,
) -> Result<Vec<ModelOption>, IoError> {
    send_codex(
        input,
        json!({"id": 0, "method": "initialize", "params": {
            "clientInfo": {"name": "teryon", "version": env!("CARGO_PKG_VERSION")}
        }}),
    )
    .await?;
    codex_response(output, 0).await?;
    send_codex(input, json!({"method": "initialized", "params": {}})).await?;
    let mut cursor: Option<String> = None;
    let mut names: Vec<ModelOption> = Vec::new();
    let mut id = 1;
    loop {
        send_codex(
            input,
            json!({"id": id, "method": "model/list", "params": {
                "includeHidden": false, "cursor": cursor
            }}),
        )
        .await?;
        let page: CodexModelsPage = from_value(codex_response(output, id).await?)
            .map_err(|problem| IoError::new(ErrorKind::InvalidData, problem))?;
        for model in page.data {
            if !model.hidden {
                let name = checked_model(Some(&model.model))
                    .map_err(|problem| IoError::new(ErrorKind::InvalidData, problem))?
                    .expect("an explicit model name");
                if !names.iter().any(|option| option.id == name) {
                    let efforts = ordered_efforts(
                        model
                            .supported_reasoning_efforts
                            .iter()
                            .filter_map(|option| listed_effort("Codex", &option.reasoning_effort)),
                    );
                    let default_effort = model
                        .default_reasoning_effort
                        .as_deref()
                        .and_then(|name| listed_effort("Codex", name))
                        .filter(|effort| efforts.contains(effort));
                    names.push(ModelOption {
                        label: model
                            .display_name
                            .filter(|label| !label.is_empty())
                            .unwrap_or_else(|| formatted_model(&name)),
                        id: name,
                        efforts,
                        default_effort,
                        is_default: model.is_default,
                    });
                }
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            mark_default(&mut names, |_| false);
            return Ok(names);
        }
        id += 1;
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexModelsPage {
    data: Vec<CodexModel>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexModel {
    model: String,
    display_name: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    is_default: bool,
    #[serde(default)]
    supported_reasoning_efforts: Vec<CodexEffort>,
    default_reasoning_effort: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexEffort {
    reasoning_effort: String,
}

async fn send_codex(input: &mut ChildStdin, message: Value) -> Result<(), IoError> {
    input.write_all(format!("{message}\n").as_bytes()).await?;
    input.flush().await
}

/// Waits for one answer, ignoring unrelated notifications.
async fn codex_response(
    output: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
) -> Result<Value, IoError> {
    while let Some(line) = output.next_line().await? {
        let message: Value =
            from_str(&line).map_err(|problem| IoError::new(ErrorKind::InvalidData, problem))?;
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(IoError::other(error.to_string()));
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "Codex returned no result"));
    }
    Err(IoError::new(
        ErrorKind::UnexpectedEof,
        "Codex closed before answering",
    ))
}

/// Asks the Grok program which models it offers.
pub async fn list_grok_models(program: &str) -> Result<Vec<ModelOption>, ListingError> {
    let mut command = Command::new(program_path(program));
    command
        .arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = timeout(LISTING_TIME, command.output())
        .await
        .map_err(|_| ListingError::TooSlow {
            program: program.to_string(),
            seconds: LISTING_TIME.as_secs(),
        })?
        .map_err(|source| ListingError::Unstartable {
            program: program.to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(ListingError::Failed {
            program: program.to_string(),
            complaint: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(grok_models_in(&String::from_utf8_lossy(&output.stdout)))
}

/// Asks Antigravity which Gemini models it offers.
pub async fn list_gemini_models(program: &str) -> Result<Vec<ModelOption>, ListingError> {
    let mut command = Command::new(program_path(program));
    command
        .arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = timeout(GEMINI_LISTING_TIME, command.output())
        .await
        .map_err(|_| ListingError::TooSlow {
            program: program.to_string(),
            seconds: GEMINI_LISTING_TIME.as_secs(),
        })?
        .map_err(|source| ListingError::Unstartable {
            program: program.to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(ListingError::Failed {
            program: program.to_string(),
            complaint: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(gemini_models_in(&String::from_utf8_lossy(&output.stdout)))
}

/// Whether an effort name is one of Antigravity's unambiguous Gemini suffixes.
///
/// Gemini model families themselves use tier names such as `ultra`, so without
/// a bracketed label from `agy models` or a discovered catalog entry, only
/// these four suffixes are split from a Gemini identifier.
fn is_gemini_effort_suffix(effort: Effort) -> bool {
    matches!(
        effort,
        Effort::Low | Effort::Medium | Effort::High | Effort::Xhigh
    )
}

/// Splits a concrete Antigravity identifier into its base model and effort.
///
/// Uses the discovered catalog when available so a standalone model whose name
/// ends in an effort word (such as `gemini-4-ultra`) is never cut, while a
/// grouped variant (such as `gemini-4-neon-high`) is matched back to its base.
/// Without a matching catalog entry, only Antigravity's unambiguous effort
/// suffixes (`low`, `medium`, `high`, `xhigh`) are split.
pub fn split_gemini_model<'a>(id: &'a str, catalog: &[ModelOption]) -> (&'a str, Option<Effort>) {
    if catalog.iter().any(|option| option.id == id) {
        return (id, None);
    }
    if let Some((base, suffix)) = id.rsplit_once('-') {
        if let Some(effort) = Effort::parse(suffix) {
            if let Some(option) = catalog.iter().find(|option| option.id == base) {
                return if option.efforts.is_empty() {
                    (id, None)
                } else {
                    (base, Some(effort))
                };
            }
            if is_gemini_effort_suffix(effort) {
                return (base, Some(effort));
            }
        }
    }
    (id, None)
}

/// Turns a Gemini model choice into the concrete identifier Antigravity accepts.
///
/// Grouping Antigravity variants improves selection, but threads must store the
/// concrete model name so turns started after a restart—before `agy models`
/// finishes—still pass a valid identifier to Antigravity.
pub fn resolve_gemini_choice(
    catalog: &[ModelOption],
    model: Option<String>,
    effort: Option<Effort>,
) -> (Option<String>, Option<Effort>) {
    let Some(model) = model else {
        return (None, effort);
    };
    if let Some(option) = catalog.iter().find(|option| option.id == model) {
        if option.efforts.is_empty() {
            return (Some(model), effort);
        }
        return match effort.or(option.default_effort) {
            Some(chosen) => (
                Some(format!("{}-{}", option.id, chosen.name())),
                Some(chosen),
            ),
            None => (Some(model), None),
        };
    }
    let (base, suffix) = split_gemini_model(&model, catalog);
    match (suffix, effort) {
        (Some(_), Some(chosen)) => (Some(format!("{base}-{}", chosen.name())), Some(chosen)),
        _ => (Some(model), effort),
    }
}

/// Whether a Claude short alias such as `opus` refers to a versioned model identifier.
pub fn claude_alias_matches(alias: &str, id: &str) -> bool {
    !alias.contains('-')
        && id
            .strip_prefix("claude-")
            .and_then(|rest| rest.split('-').next())
            == Some(alias)
}

/// Reads Antigravity's Gemini models and their display labels.
pub fn gemini_models_in(text: &str) -> Vec<ModelOption> {
    group_gemini(text.lines().filter_map(|line| {
        let trimmed = line.trim_start();
        let words = trimmed
            .strip_prefix(['*', '-', '•', '>'])
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .unwrap_or(trimmed)
            .trim_start();
        let (id, label) = words.split_once(char::is_whitespace).unwrap_or((words, ""));
        if id.to_ascii_lowercase().starts_with("gemini-") {
            Some((id, label.trim(), trimmed.starts_with('*')))
        } else {
            if label
                .split_whitespace()
                .any(|word| word.to_ascii_lowercase().starts_with("gemini-"))
            {
                warn!("skipped a Gemini model line whose layout was not recognised: {line}");
            }
            None
        }
    }))
}

/// Splits a Gemini entry from `agy models` only when its bracketed label names the same effort.
///
/// Standalone models whose names end in an effort word (such as `gemini-4-ultra`
/// labelled `Gemini 4 Ultra`) carry no effort tag in brackets and stay whole.
fn split_labelled_gemini<'a>(id: &'a str, label: &'a str) -> (&'a str, &'a str, Option<Effort>) {
    let Some((name, tag)) = label
        .strip_suffix(')')
        .and_then(|text| text.rsplit_once(" ("))
    else {
        return (id, label, None);
    };
    let Some((base, suffix)) = id.rsplit_once('-') else {
        return (id, label, None);
    };
    if !suffix.eq_ignore_ascii_case(tag) {
        return (id, label, None);
    }
    match listed_effort("Gemini", tag) {
        Some(effort) => (base, name, Some(effort)),
        None => (id, label, None),
    }
}

/// Combines concrete Antigravity variants under the version name shown in the chooser.
///
/// Entries from `agy models` carry a display label and are grouped only when the
/// label ends with a matching effort tag in brackets. Explicit `config.toml`
/// lists have no labels, so only Antigravity's unambiguous effort suffixes are
/// grouped there (plus any `-max`/`-ultra` variant sharing an already grouped base).
fn group_gemini<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str, bool)>,
) -> Vec<ModelOption> {
    let valid: Vec<(String, &'a str, bool)> = entries
        .into_iter()
        .filter_map(|(id, label, marked)| {
            let id = checked_model(Some(id)).ok().flatten()?;
            Some((id, label, marked))
        })
        .collect();
    let configured_bases: Vec<String> = valid
        .iter()
        .filter(|(_, label, _)| label.is_empty())
        .filter_map(|(id, _, _)| {
            let (base, suffix) = id.rsplit_once('-')?;
            let effort = Effort::parse(suffix)?;
            is_gemini_effort_suffix(effort).then(|| base.to_string())
        })
        .collect();
    let mut models: Vec<ModelOption> = Vec::new();
    for (id, label, marked) in &valid {
        let (base, display, effort) = if !label.is_empty() {
            split_labelled_gemini(id, label)
        } else if let Some((base, suffix)) = id.rsplit_once('-') {
            match Effort::parse(suffix) {
                Some(effort)
                    if is_gemini_effort_suffix(effort)
                        || configured_bases.iter().any(|known| known == base) =>
                {
                    (base, "", Some(effort))
                }
                _ => (id.as_str(), "", None),
            }
        } else {
            (id.as_str(), "", None)
        };
        if let Some(model) = models.iter_mut().find(|model| model.id == base) {
            if let Some(effort) = effort {
                if !model.efforts.contains(&effort) {
                    model.efforts.push(effort);
                    model.efforts.sort_unstable();
                }
                if *marked || model.default_effort.is_none() {
                    model.default_effort = Some(effort);
                }
            }
            model.is_default |= *marked;
        } else {
            models.push(ModelOption {
                id: base.to_string(),
                label: if display.is_empty() {
                    formatted_model(base)
                } else {
                    display.to_string()
                },
                efforts: effort.into_iter().collect(),
                default_effort: effort,
                is_default: *marked,
            });
        }
    }
    mark_default(&mut models, |_| false);
    models
}

/// Reads Grok's versioned names and its marked or announced default.
pub fn grok_models_in(text: &str) -> Vec<ModelOption> {
    let mut announced = None;
    let mut marked = None;
    let mut names = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Default model:") {
            announced = rest.split_whitespace().next();
        }
        if let Some(rest) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")) {
            if let Some(id) = rest.split_whitespace().next() {
                if line.starts_with('*') {
                    marked = Some(id);
                }
                names.push(id.to_string());
            }
        }
    }
    let mut models = named_models(&names, Agent::Grok);
    mark_default(&mut models, |id| Some(id) == marked.or(announced));
    models
}

/// Why a model name was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelError {
    #[error("A model needs a name.")]
    Empty,
    #[error("A model's name has no spaces in it, and does not start with a dash.")]
    Malformed,
}

/// Checks a model name sent by a client before it reaches a program's command line.
///
/// Whether the program knows the model is left to the program, which says
/// so in the thread. A name that would be read as an option, or as more than
/// one word, is refused here.
pub fn checked_model(name: Option<&str>) -> Result<Option<String>, ModelError> {
    let Some(name) = name else {
        return Ok(None);
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(ModelError::Empty);
    }
    if name.starts_with('-') || name.chars().any(char::is_whitespace) {
        return Err(ModelError::Malformed);
    }
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;
    use crate::settings::{CodexSettings, GeminiSettings, GrokSettings};

    const GROK_MODELS: &str = "You are logged in with grok.com.\n\nDefault model: grok-4.7\n\nAvailable models:\n  * grok-4.7 (default)\n  - grok-4.7-build-fast\n  - grok-4.6\n";
    const GEMINI_MODELS: &str = "gemini-4-neon-high             Gemini 4 Neon (High)\ngemini-3.8-flash-medium        Gemini 3.8 Flash (Medium)\ngemini-4-neon-high             Gemini 4 Neon (High)\nclaude-opus-5-5-high           Claude Opus 5.5 (High)\ngpt-oss-120b-medium            GPT-OSS 120B (Medium)\n";

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn settings(
        claude: Option<&[&str]>,
        codex: Option<&[&str]>,
        grok: Option<&[&str]>,
        gemini: Option<&[&str]>,
    ) -> AgentSettings {
        AgentSettings {
            claude: ClaudeSettings {
                models: claude.map(names),
                ..ClaudeSettings::default()
            },
            codex: CodexSettings {
                models: codex.map(names),
                ..CodexSettings::default()
            },
            grok: GrokSettings {
                models: grok.map(names),
                ..GrokSettings::default()
            },
            gemini: GeminiSettings {
                models: gemini.map(names),
                ..GeminiSettings::default()
            },
            ..AgentSettings::default()
        }
    }

    fn ids(models: Vec<ModelOption>) -> Vec<String> {
        models.into_iter().map(|model| model.id).collect()
    }

    lets_expect! {
        expect({ let models = ModelLists::from_settings(&settings(claude, codex, grok, gemini)); (ids(models.claude), ids(models.codex), ids(models.grok), ids(models.gemini)) }) as lists {
            let claude: Option<&[&str]> = None;
            let codex: Option<&[&str]> = None;
            let grok: Option<&[&str]> = None;
            let gemini: Option<&[&str]> = None;

            to offer_claudes_short_names_and_nothing_else {
                have(0.clone()) equal(names(&["opus", "sonnet", "haiku"]))
            }
            when(claude = Some(&["claude-opus-5-5"][..])) as claude_models_are_named to replace_the_short_names {
                have(0.clone()) equal(names(&["claude-opus-5-5"]))
            }
            when(claude = Some(&[][..])) as an_empty_list_is_named to offer_nothing_but_the_default {
                have(0.clone()) equal(Vec::<String>::new())
            }
            when(codex = Some(&["gpt-6-astra"][..])) as codex_models_are_named to offer_them {
                have(1.clone()) equal(names(&["gpt-6-astra"]))
            }
            when(grok = Some(&["grok-4.6"][..])) as grok_models_are_named to offer_them {
                have(2.clone()) equal(names(&["grok-4.6"]))
            }
            when(gemini = Some(&["gemini-4-neon-high"][..])) as gemini_models_are_named to offer_them {
                have(3.clone()) equal(names(&["gemini-4-neon"]))
            }
        }

        expect(ids(grok_models_in(text))) {
            when(text = GROK_MODELS) as grok_lists_its_models to read_every_name_without_remarks {
                equal(names(&["grok-4.7", "grok-4.7-build-fast", "grok-4.6"]))
            }
            when(text = "") as nothing_is_printed to find_none { equal(Vec::<String>::new()) }
            when(text = "Please log in first.\n- \n*\n") as nothing_is_listed to find_none { equal(Vec::<String>::new()) }
        }

        expect(ids(gemini_models_in(text))) {
            when(text = GEMINI_MODELS) as antigravity_lists_models to keep_only_unique_gemini_models {
                equal(names(&["gemini-4-neon", "gemini-3.8-flash"]))
            }
            when(text = "MODEL                          LABEL\n  * gemini-4-neon-high         Gemini 4 Neon (High)\n  - gemini-3.8-flash-medium    Gemini 3.8 Flash (Medium)\n  • gemini-3.8-flash-low       Gemini 3.8 Flash (Low)\n  > gemini-3.1-pro-high        Gemini 3.1 Pro (High)\n") as list_has_header_and_default_markers to keep_marked_models {
                equal(names(&["gemini-4-neon", "gemini-3.8-flash", "gemini-3.1-pro"]))
            }
            when(text = "") as nothing_is_printed to find_none { equal(Vec::<String>::new()) }
            when(text = "claude-opus-5-5-high   Claude Opus 5.5\n\n") as only_other_providers_are_listed to find_none { equal(Vec::<String>::new()) }
        }

        expect(Effort::parse(name)) {
            when(name = "XHigh") as mixed_case to parse { equal(Some(Effort::Xhigh)) }
            when(name = "ultra") as highest_effort to parse { equal(Some(Effort::Ultra)) }
            when(name = "unknown") as a_new_name to skip { equal(None) }
        }

        story antigravity_groups_only_concrete_effort_variants {
            let models = gemini_models_in("gemini-4-neon-high    Gemini 4 Neon (High)\n* gemini-4-neon-xhigh    Gemini 4 Neon (XHigh)\ngemini-4-neon-low    Gemini 4 Neon (Low)\ngemini-pro-orchestrator-neon    Orchestrator (Neon)\ngemini-4-ultra    Gemini 4 Ultra\ngemini-4-high    Gemini 4 High");
            expect(models[0].id.as_str()) to equal("gemini-4-neon")
            expect(models[0].label.as_str()) to equal("Gemini 4 Neon")
            expect(models[0].efforts.clone()) to equal(vec![Effort::Low, Effort::High, Effort::Xhigh])
            expect(models[0].default_effort) to equal(Some(Effort::Xhigh))
            expect(models[0].is_default) to be_true
            expect(models[1].label.as_str()) to equal("Orchestrator (Neon)")
            expect(models[1].efforts.is_empty()) to be_true
            expect(models[2].id.as_str()) to equal("gemini-4-ultra")
            expect(models[2].label.as_str()) to equal("Gemini 4 Ultra")
            expect(models[2].efforts.is_empty()) to be_true
            expect(models[3].id.as_str()) to equal("gemini-4-high")
            expect(models[3].label.as_str()) to equal("Gemini 4 High")
            expect(models[3].efforts.is_empty()) to be_true
        }

        story configured_gemini_ultra_stays_whole_and_catalog_splitting_respects_standalone_models {
            let configured = ModelLists::from_settings(&settings(None, None, None, Some(&["gemini-4-ultra"]))).gemini;
            expect(configured[0].id.as_str()) to equal("gemini-4-ultra")
            expect(configured[0].label.as_str()) to equal("Gemini 4 Ultra")
            expect(configured[0].efforts.is_empty()) to be_true

            let catalog = gemini_models_in("gemini-4-neon-high    Gemini 4 Neon (High)\ngemini-4-neon-ultra    Gemini 4 Neon (Ultra)\ngemini-4-ultra    Gemini 4 Ultra\ngemini-4-high    Gemini 4 High");
            expect(split_gemini_model("gemini-4-ultra", &catalog)) to equal(("gemini-4-ultra", None))
            expect(split_gemini_model("gemini-4-ultra", &[])) to equal(("gemini-4-ultra", None))
            expect(split_gemini_model("gemini-4-high", &catalog)) to equal(("gemini-4-high", None))
            expect(split_gemini_model("gemini-4-neon-high", &catalog)) to equal(("gemini-4-neon", Some(Effort::High)))
            expect(split_gemini_model("gemini-4-neon-high", &[])) to equal(("gemini-4-neon", Some(Effort::High)))
            expect(split_gemini_model("gemini-4-neon-ultra", &catalog)) to equal(("gemini-4-neon", Some(Effort::Ultra)))
            expect(resolve_gemini_choice(&catalog, Some("gemini-4-neon".to_string()), None)) to equal((Some("gemini-4-neon-high".to_string()), Some(Effort::High)))
            expect(resolve_gemini_choice(&catalog, Some("gemini-4-neon".to_string()), Some(Effort::Ultra))) to equal((Some("gemini-4-neon-ultra".to_string()), Some(Effort::Ultra)))
            expect(resolve_gemini_choice(&catalog, Some("gemini-4-ultra".to_string()), Some(Effort::High))) to equal((Some("gemini-4-ultra".to_string()), Some(Effort::High)))
        }

        story grok_uses_the_announced_default_without_a_star {
            let models = grok_models_in("Default model: grok-4.6\n- grok-4.7-build-fast\n- grok-4.6");
            expect(models[0].label.as_str()) to equal("Grok 4.7 Build Fast")
            expect(models[0].is_default) to be_false
            expect(models[1].is_default) to be_true
            expect(models[1].efforts.clone()) to equal(Effort::ALL[..4].to_vec())
            expect(models[1].default_effort) to equal(Some(Effort::Medium))
        }

        expect(checked_model(name)) {
            when(name = None) as the_default_is_chosen to pass_nothing { equal(Ok(None)) }
            when(name = Some("opus")) as a_name_is_given to keep_it { equal(Ok(Some("opus".to_string()))) }
            when(name = Some("  grok-4.6 ")) as a_name_has_spaces_around_it to trim_them { equal(Ok(Some("grok-4.6".to_string()))) }
            when(name = Some("")) as the_name_is_empty to refuse_it { equal(Err(ModelError::Empty)) }
            when(name = Some("   ")) as the_name_is_blank to refuse_it { equal(Err(ModelError::Empty)) }
            when(name = Some("--help")) as the_name_looks_like_an_option to refuse_it { equal(Err(ModelError::Malformed)) }
            when(name = Some("opus sonnet")) as the_name_is_two_words to refuse_it { equal(Err(ModelError::Malformed)) }
        }
    }
}
