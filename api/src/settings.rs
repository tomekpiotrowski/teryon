//! Teryon's own configuration, read from the `[agents]` section.
//!
//! Erno owns the rest of the configuration file and hands this part over
//! untouched, so everything about running agents can be changed without code:
//! where projects may live and which programs are started.

use std::path::PathBuf;

use serde::Deserialize;

/// Everything Teryon adds to erno's configuration.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub agents: AgentSettings,
}

/// How agents are run on this machine.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Folders a project directory must be inside of.
    ///
    /// Empty means no project can be created: an agent is allowed to change
    /// files, so the folders it may be pointed at are opt-in.
    pub allowed_folders: Vec<PathBuf>,
    /// How long a stopped agent gets to end on its own before it is killed.
    pub stop_grace_seconds: u64,
    pub claude: ClaudeSettings,
    pub grok: GrokSettings,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            allowed_folders: Vec::new(),
            stop_grace_seconds: 5,
            claude: ClaudeSettings::default(),
            grok: GrokSettings::default(),
        }
    }
}

/// How the Claude Code program is started.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ClaudeSettings {
    pub program: String,
    /// Run on the subscription the program is signed in with.
    ///
    /// Claude Code prefers an API key from the environment over its own
    /// sign-in, which would silently bill a key instead of the subscription.
    /// When true, the key variables are not passed on; when false the
    /// environment is passed unchanged and the program decides.
    pub use_subscription: bool,
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        Self {
            program: "claude".to_string(),
            use_subscription: true,
        }
    }
}

/// How the Grok program is started.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct GrokSettings {
    pub program: String,
}

impl Default for GrokSettings {
    fn default() -> Self {
        Self {
            program: "grok".to_string(),
        }
    }
}
