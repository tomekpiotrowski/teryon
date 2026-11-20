//! Which models each agent program offers, and checking a model name before it is passed on.
//!
//! A person picks a model per thread. Every agent can also run on "default",
//! which passes no model at all and leaves the choice to the program's own
//! settings; the lists here are what is offered besides it.
//!
//! Lists go stale as providers release models, so Teryon keeps as little as
//! it can. Claude takes short names that always mean its newest models.
//! Grok lists its own models, which are asked for once when the service
//! starts. Codex has neither, so it offers only what `config.toml` names.
//! A list in `config.toml` replaces the one Teryon would use otherwise.

use std::io::Error as IoError;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;
use ts_rs::TS;

use crate::agents::runner::program_path;
use crate::settings::AgentSettings;

/// The short names Claude Code takes for its newest models.
const CLAUDE_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];

/// How long `grok models` may take before Grok is offered only its default.
const LISTING_TIME: Duration = Duration::from_secs(10);

/// The models offered for each agent, besides its default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ModelLists {
    pub claude: Vec<String>,
    pub codex: Vec<String>,
    pub grok: Vec<String>,
}

impl ModelLists {
    /// The lists as far as they are known without asking any program.
    ///
    /// Grok's stays empty unless `config.toml` names its models: it comes
    /// from [`list_grok_models`] later.
    pub fn from_settings(settings: &AgentSettings) -> Self {
        Self {
            claude: settings
                .claude
                .models
                .clone()
                .unwrap_or_else(|| CLAUDE_MODELS.map(String::from).to_vec()),
            codex: settings.codex.models.clone().unwrap_or_default(),
            grok: settings.grok.models.clone().unwrap_or_default(),
        }
    }
}

/// Why Grok's models could not be listed.
#[derive(Debug, Error)]
pub enum ListingError {
    #[error("could not start `{program}`: {source}")]
    Unstartable {
        program: String,
        #[source]
        source: IoError,
    },
    #[error("`{program} models` did not answer within {} seconds", LISTING_TIME.as_secs())]
    TooSlow { program: String },
    #[error("`{program} models` failed: {complaint}")]
    Failed { program: String, complaint: String },
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
    use crate::settings::{ClaudeSettings, CodexSettings, GrokSettings};

    const GROK_MODELS: &str = "You are logged in with grok.com.\n\nDefault model: grok-4.7\n\nAvailable models:\n  * grok-4.7 (default)\n  - grok-4.7-build-fast\n  - grok-4.6\n";

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn settings(
        claude: Option<&[&str]>,
        codex: Option<&[&str]>,
        grok: Option<&[&str]>,
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
            ..AgentSettings::default()
        }
    }

    lets_expect! {
        expect(ModelLists::from_settings(&settings(claude, codex, grok))) as lists {
            let claude: Option<&[&str]> = None;
            let codex: Option<&[&str]> = None;
            let grok: Option<&[&str]> = None;

            to offer_claudes_short_names_and_nothing_else {
                equal(ModelLists { claude: names(&["opus", "sonnet", "haiku"]), codex: Vec::new(), grok: Vec::new() })
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
        }

        expect(grok_models_in(text)) {
            when(text = GROK_MODELS) as grok_lists_its_models to read_every_name_without_remarks {
                equal(names(&["grok-4.7", "grok-4.7-build-fast", "grok-4.6"]))
            }
            when(text = "") as nothing_is_printed to find_none { equal(Vec::<String>::new()) }
            when(text = "Please log in first.\n- \n*\n") as nothing_is_listed to find_none { equal(Vec::<String>::new()) }
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
