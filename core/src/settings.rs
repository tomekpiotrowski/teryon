//! What a person can change about Teryon on their machine, read from `config.toml`.
//!
//! Every line is optional. Without the file Teryon runs with its defaults,
//! which suit most people; the file exists for the rest, and for development.
//!
//! ```toml
//! [service]
//! port = 7317
//! allowed_origins = []
//!
//! [agents]
//! stop_grace_seconds = 5
//!
//! [agents.claude]
//! program = "claude"
//! use_subscription = true
//! models = ["opus", "sonnet", "haiku"]
//!
//! [agents.codex]
//! program = "codex"
//! use_subscription = true
//! models = ["gpt-6-astra"]
//! # home = "/somewhere/.codex"  # where Codex keeps its files; CODEX_HOME or ~/.codex when not set
//!
//! [agents.grok]
//! program = "grok"
//! home = "~/.grok"
//! writable = ["~/.cache", "~/.cargo", "~/.rustup", "~/.bun", "~/.npm", "~/go", "~/.gradle", "~/.m2"]
//! models = ["grok-4.7", "grok-4.7-build-fast"]
//!
//! [open]
//! editor = "code {path}"
//! terminal = "alacritty --working-directory {path}"
//!
//! [github]
//! program = "gh"
//! ```

use std::fs::read_to_string;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// The port the service tries first.
pub const USUAL_PORT: u16 = 7317;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("Could not read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("{path} is not valid: {source}")]
    Invalid {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// Everything in `config.toml`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub service: ServiceSettings,
    pub agents: AgentSettings,
    pub open: OpenSettings,
    pub github: GitHubSettings,
}

impl Settings {
    /// Reads `config.toml` from `folder`. A folder without one gives the defaults.
    pub fn read(folder: &Path) -> Result<Self, SettingsError> {
        let path = folder.join("config.toml");
        match read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|source| SettingsError::Invalid { path, source })
            }
            Err(problem) if problem.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(SettingsError::Unreadable { path, source }),
        }
    }
}

/// How the local service listens.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceSettings {
    /// The port tried first. When it is taken, the next free one is used.
    pub port: u16,
    /// Web pages, besides the desktop app, that may connect to the service.
    ///
    /// Empty by default: any web page open in a browser can try to reach a
    /// program on this machine, and this service runs agents with full
    /// permissions. Development lists the address of `bun run start` here.
    pub allowed_origins: Vec<String>,
}

impl Default for ServiceSettings {
    fn default() -> Self {
        Self {
            port: USUAL_PORT,
            allowed_origins: Vec::new(),
        }
    }
}

/// How agents are run on this machine.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    /// How long a stopped agent gets to end on its own before it is killed.
    pub stop_grace_seconds: u64,
    pub claude: ClaudeSettings,
    pub codex: CodexSettings,
    pub grok: GrokSettings,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            stop_grace_seconds: 5,
            claude: ClaudeSettings::default(),
            codex: CodexSettings::default(),
            grok: GrokSettings::default(),
        }
    }
}

/// The programs a thread's copy can be opened in, besides the file manager.
///
/// Each is a command split on spaces, with `{path}` standing for the copy's
/// location. Nothing set means the menu does not offer it: editors and
/// terminals differ from person to person, and guessing wrong is worse than
/// not offering.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct OpenSettings {
    pub editor: Option<String>,
    pub terminal: Option<String>,
}

/// How GitHub is reached: through its own `gh` program, signed in by the person.
///
/// Teryon stores no GitHub credentials, as it stores none for agents.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GitHubSettings {
    pub program: String,
}

impl Default for GitHubSettings {
    fn default() -> Self {
        Self {
            program: "gh".to_string(),
        }
    }
}

/// How the Claude Code program is started.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ClaudeSettings {
    pub program: String,
    /// Run on the subscription the program is signed in with.
    ///
    /// Claude Code prefers an API key from the environment over its own
    /// sign-in, which would silently bill a key instead of the subscription.
    /// When true, the key variables are not passed on; when false the
    /// environment is passed unchanged and the program decides.
    pub use_subscription: bool,
    /// The models offered besides the program's default. Nothing means Teryon's own list, see [`crate::agents::models`].
    pub models: Option<Vec<String>>,
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        Self {
            program: "claude".to_string(),
            use_subscription: true,
            models: None,
        }
    }
}

/// How the Codex program, OpenAI's agent, is started.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CodexSettings {
    pub program: String,
    /// Run on the ChatGPT subscription the program is signed in with.
    ///
    /// Codex uses an API key from the environment when it finds one, which
    /// would bill the key instead of the subscription. When true, the key
    /// variables are not passed on; when false the environment is passed
    /// unchanged and the program decides.
    pub use_subscription: bool,
    /// The models offered besides the program's default. Nothing means Teryon's own list, see [`crate::agents::models`].
    pub models: Option<Vec<String>>,
    /// Where Codex keeps its files, where Teryon reads its usage limits.
    ///
    /// Codex prints nothing about its limits, but writes them to its session
    /// files. Nothing means where Codex itself looks: `CODEX_HOME`, else
    /// `.codex` in the home folder.
    pub home: Option<PathBuf>,
}

impl Default for CodexSettings {
    fn default() -> Self {
        Self {
            program: "codex".to_string(),
            use_subscription: true,
            models: None,
            home: None,
        }
    }
}

/// How the Grok program is started.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GrokSettings {
    pub program: String,
    /// Grok's own folder, where Teryon keeps Grok's sandbox profiles.
    ///
    /// Nothing means where Grok looks: `GROK_HOME`, else `~/.grok`.
    pub home: Option<PathBuf>,
    /// Folders outside the project an author may write, `~` standing for the home folder.
    ///
    /// Grok runs in a sandbox, as nobody is at its prompt to approve
    /// anything. Build tools keep caches in the home folder and fill them
    /// when they fetch a dependency, so the usual ones are listed.
    pub writable: Vec<String>,
    /// The models offered besides the program's default. Nothing means Teryon's own list, see [`crate::agents::models`].
    pub models: Option<Vec<String>>,
}

impl Default for GrokSettings {
    fn default() -> Self {
        Self {
            program: "grok".to_string(),
            home: None,
            writable: [
                "~/.cache",
                "~/.cargo",
                "~/.rustup",
                "~/.bun",
                "~/.npm",
                "~/go",
                "~/.gradle",
                "~/.m2",
            ]
            .map(String::from)
            .to_vec(),
            models: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::write;

    use lets_expect::lets_expect;
    use tempfile::tempdir;

    use super::*;

    fn read(text: Option<&str>) -> Result<Settings, String> {
        let folder = tempdir().unwrap();
        if let Some(text) = text {
            write(folder.path().join("config.toml"), text).unwrap();
        }
        Settings::read(folder.path()).map_err(|problem| problem.to_string())
    }

    lets_expect! {
        expect(read(text)) {
            when(text = None) as there_is_no_file to give_the_defaults { equal(Ok(Settings::default())) }
            when(text = Some("")) as the_file_is_empty to give_the_defaults { equal(Ok(Settings::default())) }
            when(text = Some("[service]\nallowed_origins = [\"http://localhost:4200\"]\n")) as one_line_is_set {
                to keep_the_rest_as_default {
                    equal(Ok(Settings {
                        service: ServiceSettings { port: USUAL_PORT, allowed_origins: vec!["http://localhost:4200".to_string()] },
                        agents: AgentSettings::default(),
                        open: OpenSettings::default(),
                        github: GitHubSettings::default(),
                    }))
                }
            }
            when(text = Some("[agents.claude]\nprogram = \"/opt/claude\"\n")) as the_program_is_moved {
                to keep_using_the_subscription {
                    have(as_ref().unwrap().agents.claude.clone()) equal(ClaudeSettings { program: "/opt/claude".to_string(), use_subscription: true, models: None })
                }
            }
            when(text = Some("[agents.codex]\nuse_subscription = false\n")) as the_codex_key_is_allowed {
                to keep_the_usual_program {
                    have(as_ref().unwrap().agents.codex.clone()) equal(CodexSettings { program: "codex".to_string(), use_subscription: false, models: None, home: None })
                }
            }
            when(text = Some("[agents.grok]\nmodels = [\"grok-4.6\"]\n")) as grok_models_are_named {
                to keep_them {
                    have(as_ref().unwrap().agents.grok.models.clone()) equal(Some(vec!["grok-4.6".to_string()]))
                }
            }
            when(text = Some("[agents.claude]\nmodels = []\n")) as no_claude_models_are_named {
                to keep_the_empty_list {
                    have(as_ref().unwrap().agents.claude.models.clone()) equal(Some(Vec::new()))
                }
            }
            when(text = Some("[open]\neditor = \"code {path}\"\n")) as an_editor_is_named {
                to keep_the_terminal_unset {
                    have(as_ref().unwrap().open.clone()) equal(OpenSettings { editor: Some("code {path}".to_string()), terminal: None })
                }
            }
            when(text = Some("[service]\nprot = 1\n")) as a_name_is_misspelt to be_refused { be_err }
            when(text = Some("[service")) as the_file_is_not_toml to be_refused { be_err }
        }
    }
}
