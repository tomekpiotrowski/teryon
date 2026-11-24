//! Which models each agent program offers, and checking a model name before it is passed on.
//!
//! A person picks a model per thread. Every agent can also run on "default",
//! which passes no model at all and leaves the choice to the program's own
//! settings; the lists here are what is offered besides it.
//!
//! Lists go stale as providers release models, so Teryon keeps as little as
//! it can. Claude takes short names that always mean its newest models.
//! Grok, Codex and Antigravity (for Gemini) list their own models, which are
//! asked for once when the service starts. Codex is asked through its app
//! server's `model/list` method.
//! A list in `config.toml` replaces the one Teryon would use otherwise.

use std::io::{Error as IoError, ErrorKind};
use std::process::Stdio;
use std::time::Duration;

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
use crate::settings::{AgentSettings, CodexSettings};

/// The short names Claude Code takes for its newest models.
const CLAUDE_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];

/// How long a program may take to list models before only its default is offered.
const LISTING_TIME: Duration = Duration::from_secs(10);

/// How long Antigravity may take to list Gemini models.
///
/// `agy models` starts its language server and fetches the catalog over the
/// network, which regularly takes longer than ten seconds on a cold start.
const GEMINI_LISTING_TIME: Duration = Duration::from_secs(30);

/// The models offered for each agent, besides its default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ModelLists {
    pub claude: Vec<String>,
    pub codex: Vec<String>,
    pub grok: Vec<String>,
    pub gemini: Vec<String>,
}

impl ModelLists {
    /// The lists as far as they are known without asking any program.
    ///
    /// Codex, Grok and Gemini stay empty unless `config.toml` names their
    /// models: their programs are asked later.
    pub fn from_settings(settings: &AgentSettings) -> Self {
        Self {
            claude: settings
                .claude
                .models
                .clone()
                .unwrap_or_else(|| CLAUDE_MODELS.map(String::from).to_vec()),
            codex: settings.codex.models.clone().unwrap_or_default(),
            grok: settings.grok.models.clone().unwrap_or_default(),
            gemini: settings.gemini.models.clone().unwrap_or_default(),
        }
    }
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
pub async fn list_codex_models(settings: &CodexSettings) -> Result<Vec<String>, ListingError> {
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
) -> Result<Vec<String>, IoError> {
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
    let mut names = Vec::new();
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
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
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
struct CodexModel {
    model: String,
    #[serde(default)]
    hidden: bool,
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
pub async fn list_grok_models(program: &str) -> Result<Vec<String>, ListingError> {
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
pub async fn list_gemini_models(program: &str) -> Result<Vec<String>, ListingError> {
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

/// Reads the Gemini model names out of what `agy models` prints.
///
/// Antigravity lists models from several providers, one per line, with the
/// model identifier first (optionally preceded by a list or default marker
/// such as `* ` or `- `) followed by spaces and its display label:
///
/// ```text
/// * gemini-4-neon-high           Gemini 4 Neon (High)
/// gemini-3.8-flash-medium        Gemini 3.8 Flash (Medium)
/// claude-opus-5-5-high           Claude Opus 5.5 (High)
/// ```
///
/// Only Gemini models are kept, since Teryon runs other providers through
/// their own programs. Duplicates are dropped while keeping the printed order.
/// A line whose first word is not a Gemini slug, yet still contains a
/// `gemini-*` word later on, logs a warning so layout changes do not silently
/// drop models.
pub fn gemini_models_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let without_marker = trimmed
            .strip_prefix('*')
            .or_else(|| trimmed.strip_prefix('-'))
            .or_else(|| trimmed.strip_prefix('•'))
            .or_else(|| trimmed.strip_prefix('>'))
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .unwrap_or(trimmed);
        let mut words = without_marker.split_whitespace();
        let Some(token) = words.next() else {
            continue;
        };
        if !token.to_ascii_lowercase().starts_with("gemini-") {
            if words.any(|word| word.to_ascii_lowercase().starts_with("gemini-")) {
                warn!("skipped a Gemini model line whose layout was not recognised: {line}");
            }
            continue;
        }
        if let Ok(Some(name)) = checked_model(Some(token)) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Reads the model names out of what `grok models` prints.
///
/// It prints a few lines about the sign-in, then one model per line after
/// a `*` (the default) or a `-`, with remarks after the name:
///
/// ```text
/// Available models:
///   * grok-4.7 (default)
///   - grok-4.7-build-fast
/// ```
pub fn grok_models_in(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line
                .strip_prefix("* ")
                .or_else(|| line.strip_prefix("- "))?;
            rest.split_whitespace().next().map(str::to_string)
        })
        .collect()
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
    use crate::settings::{ClaudeSettings, CodexSettings, GeminiSettings, GrokSettings};

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

    lets_expect! {
        expect(ModelLists::from_settings(&settings(claude, codex, grok, gemini))) as lists {
            let claude: Option<&[&str]> = None;
            let codex: Option<&[&str]> = None;
            let grok: Option<&[&str]> = None;
            let gemini: Option<&[&str]> = None;

            to offer_claudes_short_names_and_nothing_else {
                equal(ModelLists { claude: names(&["opus", "sonnet", "haiku"]), codex: Vec::new(), grok: Vec::new(), gemini: Vec::new() })
            }
            when(claude = Some(&["claude-opus-5-5"][..])) as claude_models_are_named to replace_the_short_names {
                have(claude.clone()) equal(names(&["claude-opus-5-5"]))
            }
            when(claude = Some(&[][..])) as an_empty_list_is_named to offer_nothing_but_the_default {
                have(claude.clone()) equal(Vec::<String>::new())
            }
            when(codex = Some(&["gpt-6-astra"][..])) as codex_models_are_named to offer_them {
                have(codex.clone()) equal(names(&["gpt-6-astra"]))
            }
            when(grok = Some(&["grok-4.6"][..])) as grok_models_are_named to offer_them {
                have(grok.clone()) equal(names(&["grok-4.6"]))
            }
            when(gemini = Some(&["gemini-4-neon-high"][..])) as gemini_models_are_named to offer_them {
                have(gemini.clone()) equal(names(&["gemini-4-neon-high"]))
            }
        }

        expect(grok_models_in(text)) {
            when(text = GROK_MODELS) as grok_lists_its_models to read_every_name_without_remarks {
                equal(names(&["grok-4.7", "grok-4.7-build-fast", "grok-4.6"]))
            }
            when(text = "") as nothing_is_printed to find_none { equal(Vec::<String>::new()) }
            when(text = "Please log in first.\n- \n*\n") as nothing_is_listed to find_none { equal(Vec::<String>::new()) }
        }

        expect(gemini_models_in(text)) {
            when(text = GEMINI_MODELS) as antigravity_lists_models to keep_only_unique_gemini_models {
                equal(names(&["gemini-4-neon-high", "gemini-3.8-flash-medium"]))
            }
            when(text = "MODEL                          LABEL\n  * gemini-4-neon-high         Gemini 4 Neon (High)\n  - gemini-3.8-flash-medium    Gemini 3.8 Flash (Medium)\n  • gemini-3.8-flash-low       Gemini 3.8 Flash (Low)\n  > gemini-3.1-pro-high        Gemini 3.1 Pro (High)\n") as list_has_header_and_default_markers to keep_marked_models {
                equal(names(&["gemini-4-neon-high", "gemini-3.8-flash-medium", "gemini-3.8-flash-low", "gemini-3.1-pro-high"]))
            }
            when(text = "") as nothing_is_printed to find_none { equal(Vec::<String>::new()) }
            when(text = "claude-opus-5-5-high   Claude Opus 5.5\n\n") as only_other_providers_are_listed to find_none { equal(Vec::<String>::new()) }
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
