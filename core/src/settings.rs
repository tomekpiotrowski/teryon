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
//! strong = "opus"      # the model each strength runs; Claude's are built in
//! medium = "sonnet"
//! fast = "haiku"
//!
//! [agents.codex]
//! program = "codex"
//! use_subscription = true
//! models = ["gpt-6-astra"]
//! strong = "gpt-6-astra"   # a strength left out runs the program's default model
//! # home = "/somewhere/.codex"  # where Codex keeps its files; CODEX_HOME or ~/.codex when not set
//!
//! [agents.grok]
//! program = "grok"
//! home = "~/.grok"
//! writable = ["~/.cache", "~/.cargo", "~/.rustup", "~/.bun", "~/.npm", "~/go", "~/.gradle", "~/.m2"]
//! models = ["grok-4.7", "grok-4.7-build-fast"]
//! fast = "grok-4.7-build-fast"
//!
//! [agents.gemini]
//! program = "agy"
//! models = ["gemini-4-neon-high", "gemini-3.8-flash-medium"]
//! strong = "gemini-4-neon-high"
//! fast = "gemini-3.8-flash-low"
//!
//! [workflow]
//! default = "default"    # the workflow new threads start on
//! automatic_rounds = 3   # agents started in a row without a message from the person; 0 starts none
//!
//! # Replaces Teryon's own workflows, `default` and `small`, when present.
//! [workflows.default]
//! description = "Anything that needs thinking through: features, bugs, changes to design."
//! stages = ["talking", "planning", "implementing", "review", "ready", "done"]
//!
//! [workflows.small]
//! description = "Small, clear changes: documentation, wording, settings."
//! stages = ["talking", "implementing", "ready", "done"]
//!
//! # Who works in each stage: a provider, chosen as "any", "prefer <agent>"
//! # or "only <agent>", and a strength: "strong", "medium" or "fast".
//! [stages.planning]
//! provider = "prefer claude"
//! strength = "strong"
//!
//! [stages.implementing]
//! provider = "only codex"
//! strength = "medium"
//!
//! [stages.review]
//! provider = "any"
//! among = ["claude", "grok"]   # which providers "any" and "prefer" may fall back to; all installed ones when left out
//! strength = "strong"
//!
//! [stages.ready]
//! label = "Ready to merge"     # what the stage is called on screen
//!
//! [open]
//! editor = "code {path}"
//! terminal = "alacritty --working-directory {path}"
//!
//! [github]
//! program = "gh"
//! ```

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter, Result as FormatResult};
use std::fs::read_to_string;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};
use ts_rs::TS;

use crate::agents::models::Effort;
use crate::agents::Agent;
use crate::workflow::{built_in_workflows, Stage};

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
    /// The file reads well but breaks a rule about workflows or stages; the text says which.
    #[error("{path} is not valid: {problem}")]
    Rule { path: PathBuf, problem: String },
}

/// Everything in `config.toml`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub service: ServiceSettings,
    pub agents: AgentSettings,
    pub open: OpenSettings,
    pub github: GitHubSettings,
    pub workflow: WorkflowSettings,
    /// Nothing means Teryon's own workflows, see [`crate::workflow::built_in_workflows`].
    pub workflows: Option<BTreeMap<String, WorkflowDefinition>>,
    pub stages: StageSettings,
}

impl Settings {
    /// Reads `config.toml` from `folder`. A folder without one gives the defaults.
    pub fn read(folder: &Path) -> Result<Self, SettingsError> {
        let path = folder.join("config.toml");
        let settings: Self = match read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|source| SettingsError::Invalid {
                path: path.clone(),
                source,
            })?,
            Err(problem) if problem.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(SettingsError::Unreadable { path, source }),
        };
        settings
            .check()
            .map_err(|problem| SettingsError::Rule { path, problem })?;
        Ok(settings)
    }

    /// Refuses workflows and stage settings Teryon could not follow. The text says what is wrong, for the person.
    ///
    /// Checked when the service starts rather than when a thread reaches the
    /// stage, so a mistake shows at once instead of in the middle of the night.
    pub fn check(&self) -> Result<(), String> {
        let workflows = self.workflows.clone().unwrap_or_else(built_in_workflows);
        if workflows.is_empty() {
            return Err("[workflows] names no workflow.".to_string());
        }
        for (name, workflow) in &workflows {
            check_workflow(name, &workflow.stages)?;
        }
        if !workflows.contains_key(&self.workflow.default) {
            return Err(format!(
                "[workflow] default names `{}`, which is not one of the workflows.",
                self.workflow.default
            ));
        }
        for stage in Stage::ALL {
            let Some(setting) = self.stages.of(stage) else {
                continue;
            };
            let chooses =
                setting.provider.is_some() || setting.among.is_some() || setting.strength.is_some();
            if !stage.worked() && chooses {
                return Err(format!(
                    "[stages.{}] can only set a label: nobody works in that stage, it waits for you.",
                    stage.name()
                ));
            }
            if let Some(among) = &setting.among {
                if matches!(setting.provider, Some(ProviderChoice::Only(_))) {
                    return Err(format!(
                        "[stages.{}] sets `among` beside \"only\", which never picks another provider.",
                        stage.name()
                    ));
                }
                if among.is_empty() {
                    return Err(format!(
                        "[stages.{}] among lists no provider.",
                        stage.name()
                    ));
                }
            }
        }
        let only = |stage| match self.stages.of(stage).and_then(|setting| setting.provider) {
            Some(ProviderChoice::Only(agent)) => Some(agent),
            _ => None,
        };
        if let (Some(author), Some(reviewer)) = (only(Stage::Implementing), only(Stage::Review)) {
            let both = workflows.iter().find(|(_, workflow)| {
                workflow.stages.contains(&Stage::Implementing)
                    && workflow.stages.contains(&Stage::Review)
            });
            if let (true, Some((name, _))) = (author == reviewer, both) {
                return Err(format!(
                    "implementing and review both allow only {}, but an agent cannot review its own change, and the `{name}` workflow has both stages.",
                    author.label()
                ));
            }
        }
        Ok(())
    }
}

/// Refuses a workflow Teryon could not take a thread through.
///
/// The stages keep the fixed order, so a workflow can skip stages but not
/// reorder them. Ready and Done are always there: a change has to end
/// somewhere. A thread starts in talking, planning or implementing.
fn check_workflow(name: &str, stages: &[Stage]) -> Result<(), String> {
    if stages.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!(
            "the `{name}` workflow lists its stages out of order, or one twice. The order is talking, planning, implementing, review, ready, done."
        ));
    }
    if !stages.contains(&Stage::Ready) || !stages.contains(&Stage::Done) {
        return Err(format!(
            "the `{name}` workflow needs `ready` and `done`: a change has to end somewhere."
        ));
    }
    if !stages.first().is_some_and(|stage| *stage < Stage::Review) {
        return Err(format!(
            "the `{name}` workflow has to start with talking, planning or implementing."
        ));
    }
    Ok(())
}

/// Which workflow new threads start on, and how far agents may go without the person.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WorkflowSettings {
    pub default: String,
    /// How many agents Teryon starts by itself, in a row, before it waits for a message from the person.
    ///
    /// An author and a reviewer who never agree would otherwise use up a
    /// subscription while nobody watches. Zero turns automatic starts off.
    pub automatic_rounds: u32,
}

impl Default for WorkflowSettings {
    fn default() -> Self {
        Self {
            default: "default".to_string(),
            automatic_rounds: 3,
        }
    }
}

/// One workflow: the stages a thread on it goes through.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    /// What the workflow suits, in one line, for the agent choosing one.
    #[serde(default)]
    pub description: String,
    pub stages: Vec<Stage>,
}

/// What `config.toml` says about each stage.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StageSettings {
    pub talking: Option<StageSetting>,
    pub planning: Option<StageSetting>,
    pub implementing: Option<StageSetting>,
    pub review: Option<StageSetting>,
    pub ready: Option<StageSetting>,
    pub done: Option<StageSetting>,
}

impl StageSettings {
    /// Whether any stage chooses who works in it.
    ///
    /// Until one does, Teryon starts no agent by itself and works as before
    /// stages existed: the person passes on findings and asks for reviews.
    pub fn any_chooses(&self) -> bool {
        Stage::ALL
            .into_iter()
            .any(|stage| self.of(stage).is_some_and(StageSetting::chooses))
    }

    pub fn of(&self, stage: Stage) -> Option<&StageSetting> {
        match stage {
            Stage::Talking => self.talking.as_ref(),
            Stage::Planning => self.planning.as_ref(),
            Stage::Implementing => self.implementing.as_ref(),
            Stage::Review => self.review.as_ref(),
            Stage::Ready => self.ready.as_ref(),
            Stage::Done => self.done.as_ref(),
        }
    }
}

/// Who works in a stage, and what it is called.
///
/// A stage without `provider` and `strength` keeps the thread's agent and
/// model as they are. With only one of them, the other is "any" or "medium".
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StageSetting {
    pub provider: Option<ProviderChoice>,
    /// The providers "any" and "prefer" may pick or fall back to. Nothing means every one installed.
    pub among: Option<Vec<AgentName>>,
    pub strength: Option<Strength>,
    pub label: Option<String>,
}

impl StageSetting {
    /// Whether the stage chooses who works in it, rather than keeping the thread's agent.
    pub fn chooses(&self) -> bool {
        self.provider.is_some() || self.strength.is_some()
    }
}

/// An agent named in `config.toml`, refused there when Teryon does not know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct AgentName(pub Agent);

impl TryFrom<String> for AgentName {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Agent::parse(&name).map(Self).ok_or_else(|| {
            format!("`{name}` is not an agent Teryon knows: claude, codex, grok or gemini")
        })
    }
}

/// How a stage chooses its provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum ProviderChoice {
    /// Any provider with usage left, picked at random.
    Any,
    /// This one while it has usage left, another otherwise.
    Prefer(Agent),
    /// This one only; when its usage runs out, the thread waits.
    Only(Agent),
}

impl TryFrom<String> for ProviderChoice {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let agent = |name: &str| {
            Agent::parse(name).ok_or_else(|| {
                format!("`{name}` is not an agent Teryon knows: claude, codex, grok or gemini")
            })
        };
        match words.as_slice() {
            ["any"] => Ok(Self::Any),
            ["prefer", name] => agent(name).map(Self::Prefer),
            ["only", name] => agent(name).map(Self::Only),
            _ => Err(format!(
                "provider is \"any\", \"prefer <agent>\" or \"only <agent>\", not \"{text}\""
            )),
        }
    }
}

impl Display for ProviderChoice {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult {
        match self {
            Self::Any => write!(formatter, "Any provider"),
            Self::Prefer(agent) => write!(formatter, "Prefer {}", agent.label()),
            Self::Only(agent) => write!(formatter, "Only {}", agent.label()),
        }
    }
}

/// Which of a provider's three chosen models a stage runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Strong,
    Medium,
    Fast,
}

impl Strength {
    pub fn name(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Medium => "medium",
            Self::Fast => "fast",
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
    pub gemini: GeminiSettings,
}

impl AgentSettings {
    /// The model and effort an agent runs for a strength.
    ///
    /// An unset value leaves that part of the choice to the program.
    pub fn choice_for(&self, agent: Agent, strength: Strength) -> (Option<String>, Option<Effort>) {
        let choices = match agent {
            Agent::Claude => [
                (&self.claude.strong, self.claude.strong_effort),
                (&self.claude.medium, self.claude.medium_effort),
                (&self.claude.fast, self.claude.fast_effort),
            ],
            Agent::Codex => [
                (&self.codex.strong, self.codex.strong_effort),
                (&self.codex.medium, self.codex.medium_effort),
                (&self.codex.fast, self.codex.fast_effort),
            ],
            Agent::Grok => [
                (&self.grok.strong, self.grok.strong_effort),
                (&self.grok.medium, self.grok.medium_effort),
                (&self.grok.fast, self.grok.fast_effort),
            ],
            Agent::Gemini => [
                (&self.gemini.strong, self.gemini.strong_effort),
                (&self.gemini.medium, self.gemini.medium_effort),
                (&self.gemini.fast, self.gemini.fast_effort),
            ],
        };
        let (model, effort) = match strength {
            Strength::Strong => choices[0],
            Strength::Medium => choices[1],
            Strength::Fast => choices[2],
        };
        (model.clone(), effort)
    }

    /// The program that runs an agent.
    pub fn program(&self, agent: Agent) -> &str {
        match agent {
            Agent::Claude => &self.claude.program,
            Agent::Codex => &self.codex.program,
            Agent::Grok => &self.grok.program,
            Agent::Gemini => &self.gemini.program,
        }
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            stop_grace_seconds: 5,
            claude: ClaudeSettings::default(),
            codex: CodexSettings::default(),
            grok: GrokSettings::default(),
            gemini: GeminiSettings::default(),
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
    /// Claude Code configuration directory, also used for its cached catalog.
    pub home: Option<PathBuf>,
    /// Run on the subscription the program is signed in with.
    ///
    /// Claude Code prefers an API key from the environment over its own
    /// sign-in, which would silently bill a key instead of the subscription.
    /// When true, the key variables are not passed on; when false the
    /// environment is passed unchanged and the program decides.
    pub use_subscription: bool,
    /// The models offered besides the program's default. Nothing means Teryon's own list, see [`crate::agents::models`].
    pub models: Option<Vec<String>>,
    /// The model for each strength. Claude's short names are built in: they always mean its newest version, so they never go stale.
    pub strong: Option<String>,
    pub medium: Option<String>,
    pub fast: Option<String>,
    pub strong_effort: Option<Effort>,
    pub medium_effort: Option<Effort>,
    pub fast_effort: Option<Effort>,
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        Self {
            program: "claude".to_string(),
            home: None,
            use_subscription: true,
            models: None,
            strong: Some("opus".to_string()),
            medium: Some("sonnet".to_string()),
            fast: Some("haiku".to_string()),
            strong_effort: None,
            medium_effort: None,
            fast_effort: None,
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
    /// The models offered besides the program's default.
    ///
    /// Nothing means asking Codex's app server for the available models when
    /// Teryon starts. An explicit list, including an empty one, overrides discovery.
    pub models: Option<Vec<String>>,
    /// Where Codex keeps its files, where Teryon reads its usage limits.
    ///
    /// Codex prints nothing about its limits, but writes them to its session
    /// files. Nothing means where Codex itself looks: `CODEX_HOME`, else
    /// `.codex` in the home folder.
    pub home: Option<PathBuf>,
    /// The model for the strong, medium and fast strengths. Nothing runs the program's default model.
    pub strong: Option<String>,
    pub medium: Option<String>,
    pub fast: Option<String>,
    pub strong_effort: Option<Effort>,
    pub medium_effort: Option<Effort>,
    pub fast_effort: Option<Effort>,
}

impl Default for CodexSettings {
    fn default() -> Self {
        Self {
            program: "codex".to_string(),
            use_subscription: true,
            models: None,
            home: None,
            strong: None,
            medium: None,
            fast: None,
            strong_effort: None,
            medium_effort: None,
            fast_effort: None,
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
    /// The model for the strong, medium and fast strengths. Nothing runs the program's default model.
    pub strong: Option<String>,
    pub medium: Option<String>,
    pub fast: Option<String>,
    pub strong_effort: Option<Effort>,
    pub medium_effort: Option<Effort>,
    pub fast_effort: Option<Effort>,
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
            strong: None,
            medium: None,
            fast: None,
            strong_effort: None,
            medium_effort: None,
            fast_effort: None,
        }
    }
}

/// How Antigravity, which runs Gemini models, is started.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GeminiSettings {
    pub program: String,
    /// The models offered besides the program's default. Nothing means asking `agy models` for its Gemini models.
    pub models: Option<Vec<String>>,
    /// The model for the strong, medium and fast strengths. Nothing runs the program's default model.
    pub strong: Option<String>,
    pub medium: Option<String>,
    pub fast: Option<String>,
    pub strong_effort: Option<Effort>,
    pub medium_effort: Option<Effort>,
    pub fast_effort: Option<Effort>,
}

impl Default for GeminiSettings {
    fn default() -> Self {
        Self {
            program: "agy".to_string(),
            models: None,
            strong: None,
            medium: None,
            fast: None,
            strong_effort: None,
            medium_effort: None,
            fast_effort: None,
        }
    }
}

/// The choices the settings screen reads and saves.
///
/// The screen shows what Teryon is using, defaults included. A blank model
/// name means the default again: Claude's built-in name, or the program's
/// own for Codex, Grok and Gemini. A blank label means the stage's usual word.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Configuration {
    pub default_workflow: String,
    #[ts(type = "number")]
    pub automatic_rounds: u32,
    /// Whether `config.toml` names its own workflows. False means the built-in pair.
    pub own_workflows: bool,
    pub workflows: Vec<ConfiguredWorkflow>,
    /// One entry per stage, in the fixed order.
    pub stages: Vec<ConfiguredStage>,
    pub claude: StrengthModels,
    pub codex: StrengthModels,
    pub grok: StrengthModels,
    pub gemini: StrengthModels,
    /// Empty when no editor is set up.
    pub editor: String,
    /// Empty when no terminal is set up.
    pub terminal: String,
}

/// One workflow as the settings screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConfiguredWorkflow {
    pub name: String,
    pub description: String,
    pub stages: Vec<Stage>,
}

/// What the settings screen says about one stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConfiguredStage {
    pub stage: Stage,
    /// Empty when the stage keeps its usual name.
    pub label: String,
    /// Nothing when the stage keeps the thread's agent, unless `strength` is set, which means any provider.
    pub provider: Option<StageProvider>,
    /// Nothing when the stage keeps the thread's agent.
    pub strength: Option<Strength>,
}

/// How a stage chooses its provider, as the settings screen sends it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageProvider {
    /// Any provider with usage left. An empty `among` means every one installed.
    Any { among: Vec<String> },
    /// This one while it has usage left. An empty `among` means every one installed.
    Prefer { agent: String, among: Vec<String> },
    /// This one only.
    Only { agent: String },
}

/// The model an agent runs for each strength. Empty means the default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StrengthModels {
    pub strong: StrengthChoice,
    pub medium: StrengthChoice,
    pub fast: StrengthChoice,
}

/// The model and optional effort displayed for one strength.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StrengthChoice {
    pub model: String,
    pub effort: Option<Effort>,
}

impl Configuration {
    /// The settings screen's view of `settings`.
    pub fn from_settings(settings: &Settings) -> Self {
        let own_workflows = settings.workflows.is_some();
        let definitions = settings
            .workflows
            .clone()
            .unwrap_or_else(built_in_workflows);
        Self {
            default_workflow: settings.workflow.default.clone(),
            automatic_rounds: settings.workflow.automatic_rounds,
            own_workflows,
            workflows: definitions
                .into_iter()
                .map(|(name, definition)| ConfiguredWorkflow {
                    name,
                    description: definition.description,
                    stages: definition.stages,
                })
                .collect(),
            stages: Stage::ALL
                .into_iter()
                .map(|stage| configured_stage(stage, settings.stages.of(stage)))
                .collect(),
            claude: shown_models(&settings.agents, Agent::Claude),
            codex: shown_models(&settings.agents, Agent::Codex),
            grok: shown_models(&settings.agents, Agent::Grok),
            gemini: shown_models(&settings.agents, Agent::Gemini),
            editor: settings.open.editor.clone().unwrap_or_default(),
            terminal: settings.open.terminal.clone().unwrap_or_default(),
        }
    }
}

fn configured_stage(stage: Stage, setting: Option<&StageSetting>) -> ConfiguredStage {
    let Some(setting) = setting else {
        return ConfiguredStage {
            stage,
            label: String::new(),
            provider: None,
            strength: None,
        };
    };
    ConfiguredStage {
        stage,
        label: setting.label.clone().unwrap_or_default(),
        provider: setting.provider.map(|provider| match provider {
            ProviderChoice::Any => StageProvider::Any {
                among: among_names(setting.among.as_deref()),
            },
            ProviderChoice::Prefer(agent) => StageProvider::Prefer {
                agent: agent.name().to_string(),
                among: among_names(setting.among.as_deref()),
            },
            ProviderChoice::Only(agent) => StageProvider::Only {
                agent: agent.name().to_string(),
            },
        }),
        strength: setting.strength,
    }
}

fn among_names(among: Option<&[AgentName]>) -> Vec<String> {
    among
        .map(|agents| {
            agents
                .iter()
                .map(|agent| agent.0.name().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Presents unset models using the built-in choice for the strength, if any.
fn shown_models(agents: &AgentSettings, agent: Agent) -> StrengthModels {
    let defaults = AgentSettings::default();
    let shown = |strength| {
        let (model, effort) = agents.choice_for(agent, strength);
        StrengthChoice {
            model: model
                .or_else(|| defaults.choice_for(agent, strength).0)
                .unwrap_or_default(),
            effort,
        }
    };
    StrengthModels {
        strong: shown(Strength::Strong),
        medium: shown(Strength::Medium),
        fast: shown(Strength::Fast),
    }
}

/// The text `config.toml` would have after the settings screen saves `draft`.
///
/// The screen writes only the choices it edits, and a broken draft must not
/// replace a file the service is already using.
///
/// `current` is the file's text, or empty when there is no file. On success
/// the new text keeps every other section and the comments on keys it only
/// updates. On failure the sentence says why, and the caller leaves the file
/// as it was.
pub fn revise(current: &str, draft: &Configuration) -> Result<String, String> {
    let present = parse_settings(current)?;
    let next = overlay(&present, draft)?;
    next.check()?;
    let mut document = if current.trim().is_empty() {
        DocumentMut::new()
    } else {
        current
            .parse::<DocumentMut>()
            .map_err(|problem| problem.to_string())?
    };
    patch(&mut document, &present, &next);
    Ok(document.to_string())
}

fn parse_settings(text: &str) -> Result<Settings, String> {
    if text.trim().is_empty() {
        return Ok(Settings::default());
    }
    let settings: Settings = toml::from_str(text).map_err(|problem| problem.to_string())?;
    settings.check()?;
    Ok(settings)
}

/// Lays the screen's draft over the settings the file already has.
fn overlay(current: &Settings, draft: &Configuration) -> Result<Settings, String> {
    let mut settings = current.clone();
    settings.workflow.default = draft.default_workflow.trim().to_string();
    settings.workflow.automatic_rounds = draft.automatic_rounds;
    settings.workflows = workflows_of(draft)?;
    settings.stages = stages_of(draft)?;
    let claude = ClaudeSettings::default();
    settings.agents.claude.strong =
        strength_value(&draft.claude.strong.model, claude.strong.as_deref());
    settings.agents.claude.strong_effort = draft.claude.strong.effort;
    settings.agents.claude.medium =
        strength_value(&draft.claude.medium.model, claude.medium.as_deref());
    settings.agents.claude.medium_effort = draft.claude.medium.effort;
    settings.agents.claude.fast = strength_value(&draft.claude.fast.model, claude.fast.as_deref());
    settings.agents.claude.fast_effort = draft.claude.fast.effort;
    settings.agents.codex.strong = strength_value(&draft.codex.strong.model, None);
    settings.agents.codex.strong_effort = draft.codex.strong.effort;
    settings.agents.codex.medium = strength_value(&draft.codex.medium.model, None);
    settings.agents.codex.medium_effort = draft.codex.medium.effort;
    settings.agents.codex.fast = strength_value(&draft.codex.fast.model, None);
    settings.agents.codex.fast_effort = draft.codex.fast.effort;
    settings.agents.grok.strong = strength_value(&draft.grok.strong.model, None);
    settings.agents.grok.strong_effort = draft.grok.strong.effort;
    settings.agents.grok.medium = strength_value(&draft.grok.medium.model, None);
    settings.agents.grok.medium_effort = draft.grok.medium.effort;
    settings.agents.grok.fast = strength_value(&draft.grok.fast.model, None);
    settings.agents.grok.fast_effort = draft.grok.fast.effort;
    settings.agents.gemini.strong = strength_value(&draft.gemini.strong.model, None);
    settings.agents.gemini.strong_effort = draft.gemini.strong.effort;
    settings.agents.gemini.medium = strength_value(&draft.gemini.medium.model, None);
    settings.agents.gemini.medium_effort = draft.gemini.medium.effort;
    settings.agents.gemini.fast = strength_value(&draft.gemini.fast.model, None);
    settings.agents.gemini.fast_effort = draft.gemini.fast.effort;
    settings.open.editor = blank_as_none(&draft.editor);
    settings.open.terminal = blank_as_none(&draft.terminal);
    Ok(settings)
}

fn workflows_of(
    draft: &Configuration,
) -> Result<Option<BTreeMap<String, WorkflowDefinition>>, String> {
    if draft.workflows.is_empty() {
        return Err("[workflows] names no workflow.".to_string());
    }
    let mut workflows = BTreeMap::new();
    for workflow in &draft.workflows {
        let name = workflow.name.trim();
        if name.is_empty() {
            return Err("A workflow needs a name.".to_string());
        }
        if workflows.contains_key(name) {
            return Err(format!("Two workflows are named `{name}`."));
        }
        workflows.insert(
            name.to_string(),
            WorkflowDefinition {
                description: workflow.description.trim().to_string(),
                stages: workflow.stages.clone(),
            },
        );
    }
    if workflows == built_in_workflows() {
        return Ok(None);
    }
    Ok(Some(workflows))
}

fn stages_of(draft: &Configuration) -> Result<StageSettings, String> {
    let mut stages = StageSettings::default();
    let mut seen = Vec::new();
    for stage in &draft.stages {
        if seen.contains(&stage.stage) {
            return Err(format!("[stages.{}] is listed twice.", stage.stage.name()));
        }
        seen.push(stage.stage);
        *stages.slot_mut(stage.stage) = stage_setting(stage)?;
    }
    Ok(stages)
}

fn stage_setting(stage: &ConfiguredStage) -> Result<Option<StageSetting>, String> {
    let label = blank_as_none(&stage.label);
    let (provider, among) = match &stage.provider {
        None => (None, None),
        Some(StageProvider::Any { among }) => (Some(ProviderChoice::Any), named_agents(among)?),
        Some(StageProvider::Prefer { agent, among }) => (
            Some(ProviderChoice::Prefer(named_agent(agent)?)),
            named_agents(among)?,
        ),
        Some(StageProvider::Only { agent }) => {
            (Some(ProviderChoice::Only(named_agent(agent)?)), None)
        }
    };
    if label.is_none() && provider.is_none() && among.is_none() && stage.strength.is_none() {
        return Ok(None);
    }
    Ok(Some(StageSetting {
        provider,
        among,
        strength: stage.strength,
        label,
    }))
}

fn named_agents(names: &[String]) -> Result<Option<Vec<AgentName>>, String> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut agents = Vec::new();
    for name in names {
        agents.push(AgentName(named_agent(name)?));
    }
    Ok(Some(agents))
}

fn named_agent(name: &str) -> Result<Agent, String> {
    Agent::parse(name.trim()).ok_or_else(|| {
        format!("`{name}` is not an agent Teryon knows: claude, codex, grok or gemini")
    })
}

/// An empty field means the default. A name equal to the default is stored as that default, and left out of the file.
fn strength_value(text: &str, default: Option<&str>) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return default.map(str::to_string);
    }
    Some(trimmed.to_string())
}

fn blank_as_none(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn patch(document: &mut DocumentMut, before: &Settings, next: &Settings) {
    patch_workflow_settings(document, &next.workflow);
    patch_workflows(document, before, next);
    patch_stages(document, &next.stages);
    let defaults = AgentSettings::default();
    for agent in Agent::ALL {
        patch_strengths(document, agent, &next.agents, &defaults);
    }
    patch_open(document, &next.open);
}

fn patch_workflow_settings(document: &mut DocumentMut, workflow: &WorkflowSettings) {
    let usual = WorkflowSettings::default();
    let default = (workflow.default != usual.default).then_some(workflow.default.as_str());
    let rounds =
        (workflow.automatic_rounds != usual.automatic_rounds).then_some(workflow.automatic_rounds);
    patch_table(document, &["workflow"], |table| {
        set_or_clear_string(table, "default", default);
        match rounds {
            Some(rounds) => set_value(table, "automatic_rounds", Value::from(i64::from(rounds))),
            None => {
                table.remove("automatic_rounds");
            }
        }
    });
}

fn patch_workflows(document: &mut DocumentMut, before: &Settings, next: &Settings) {
    let before = before.workflows.clone().unwrap_or_else(built_in_workflows);
    let after = next.workflows.clone().unwrap_or_else(built_in_workflows);
    if before == after {
        return;
    }
    if next.workflows.is_none() {
        document.as_table_mut().remove("workflows");
        return;
    }
    let mut table = Table::new();
    for (name, definition) in after {
        let mut one = Table::new();
        if !definition.description.is_empty() {
            one["description"] = value_item(&definition.description);
        }
        let mut stages = Array::new();
        for stage in &definition.stages {
            stages.push(stage.name());
        }
        one["stages"] = value_item(stages);
        table.insert(&name, Item::Table(one));
    }
    document
        .as_table_mut()
        .insert("workflows", Item::Table(table));
}

fn patch_stages(document: &mut DocumentMut, stages: &StageSettings) {
    for stage in Stage::ALL {
        let name = stage.name();
        match stages.of(stage) {
            None => {
                let empty = document
                    .as_table_mut()
                    .get_mut("stages")
                    .and_then(Item::as_table_like_mut)
                    .is_some_and(|parent| {
                        parent.remove(name);
                        parent.is_empty()
                    });
                if empty {
                    document.as_table_mut().remove("stages");
                }
            }
            Some(setting) => {
                patch_table(document, &["stages", name], |table| {
                    match setting.provider {
                        Some(provider) => set_or_clear_string(
                            table,
                            "provider",
                            Some(provider_text(provider).as_str()),
                        ),
                        None => {
                            table.remove("provider");
                        }
                    }
                    match &setting.among {
                        Some(among) => {
                            let mut names = Array::new();
                            for agent in among {
                                names.push(agent.0.name());
                            }
                            set_value(table, "among", Value::Array(names));
                        }
                        None => {
                            table.remove("among");
                        }
                    }
                    match setting.strength {
                        Some(strength) => {
                            set_or_clear_string(table, "strength", Some(strength.name()))
                        }
                        None => {
                            table.remove("strength");
                        }
                    }
                    set_or_clear_string(table, "label", setting.label.as_deref());
                });
            }
        }
    }
}

fn provider_text(provider: ProviderChoice) -> String {
    match provider {
        ProviderChoice::Any => "any".to_string(),
        ProviderChoice::Prefer(agent) => format!("prefer {}", agent.name()),
        ProviderChoice::Only(agent) => format!("only {}", agent.name()),
    }
}

/// Updates only the model and effort keys edited by the settings screen.
fn patch_strengths(
    document: &mut DocumentMut,
    agent: Agent,
    values: &AgentSettings,
    defaults: &AgentSettings,
) {
    patch_table(document, &["agents", agent.name()], |table| {
        for (strength, effort_key) in [
            (Strength::Strong, "strong_effort"),
            (Strength::Medium, "medium_effort"),
            (Strength::Fast, "fast_effort"),
        ] {
            let (model, effort) = values.choice_for(agent, strength);
            let default = defaults.choice_for(agent, strength).0;
            set_or_clear_string(
                table,
                strength.name(),
                written_strength(&model, default.as_deref()),
            );
            set_or_clear_string(table, effort_key, effort.map(Effort::name));
        }
    });
}

fn written_strength<'a>(value: &'a Option<String>, default: Option<&str>) -> Option<&'a str> {
    match value {
        Some(name) if Some(name.as_str()) != default => Some(name),
        _ => None,
    }
}

fn patch_open(document: &mut DocumentMut, open: &OpenSettings) {
    patch_table(document, &["open"], |table| {
        set_or_clear_string(table, "editor", open.editor.as_deref());
        set_or_clear_string(table, "terminal", open.terminal.as_deref());
    });
}

/// Updates one table, creating it when a key is set and removing it when it ends up empty.
///
/// `path` is one or two names, such as `workflow` or `agents` then `claude`.
/// An inline table is edited where it stands. Replacing it with a standard
/// table would drop the keys this screen does not own, and the next start
/// would then be missing a program path or a folder.
fn patch_table(document: &mut DocumentMut, path: &[&str], edit: impl FnOnce(&mut dyn TableLike)) {
    ensure_table(document, path);
    if let Some(table) = table_at(document, path) {
        edit(table);
    }
    remove_if_empty(document, path);
}

fn ensure_table(document: &mut DocumentMut, path: &[&str]) {
    match path {
        [name] => {
            let item = document
                .as_table_mut()
                .entry(name)
                .or_insert(Item::Table(Table::new()));
            keep_table(item, false);
        }
        [parent, child] => {
            let parent_item = document
                .as_table_mut()
                .entry(parent)
                .or_insert(Item::Table(Table::new()));
            keep_table(parent_item, false);
            ensure_child(parent_item, child);
        }
        _ => {}
    }
}

/// Makes sure `child` is a table inside `parent`, which is already a table.
///
/// A missing child takes the same shape as its parent, so an inline parent
/// gains an inline child. Inserting a standard table into an inline one is
/// not a value that table can hold.
fn ensure_child(parent: &mut Item, child: &str) {
    let inline = parent.is_inline_table();
    let Some(table) = parent.as_table_like_mut() else {
        return;
    };
    if let Some(existing) = table.get_mut(child) {
        keep_table(existing, inline);
        return;
    }
    table.insert(child, empty_table(inline));
}

/// Leaves a table as it is, inline or standard. Anything else becomes an empty one.
///
/// `inline` chooses the empty table's shape. An inline table already holding
/// keys, such as `claude = { program = "/opt/claude", use_subscription = false }`,
/// is left alone: those keys belong to the file, not to this screen.
fn keep_table(item: &mut Item, inline: bool) {
    if !item.is_table_like() {
        *item = empty_table(inline);
    }
}

fn empty_table(inline: bool) -> Item {
    if inline {
        Item::Value(Value::InlineTable(InlineTable::new()))
    } else {
        Item::Table(Table::new())
    }
}

fn table_at<'a>(document: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut dyn TableLike> {
    match path {
        [name] => document.as_table_mut().get_mut(name)?.as_table_like_mut(),
        [parent, child] => child_table(document.as_table_mut().get_mut(parent)?, child),
        _ => None,
    }
}

fn child_table<'a>(parent: &'a mut Item, child: &str) -> Option<&'a mut dyn TableLike> {
    match parent {
        Item::Table(table) => table.get_mut(child)?.as_table_like_mut(),
        Item::Value(Value::InlineTable(table)) => table
            .get_mut(child)?
            .as_inline_table_mut()
            .map(|child| child as &mut dyn TableLike),
        _ => None,
    }
}

fn remove_if_empty(document: &mut DocumentMut, path: &[&str]) {
    match path {
        [name] => {
            if table_is_empty(document, path) {
                document.as_table_mut().remove(name);
            }
        }
        [parent, child] => {
            if table_is_empty(document, path) {
                if let Some(parent_table) = document
                    .as_table_mut()
                    .get_mut(parent)
                    .and_then(Item::as_table_like_mut)
                {
                    parent_table.remove(child);
                }
            }
            if table_is_empty(document, &[parent]) {
                document.as_table_mut().remove(parent);
            }
        }
        _ => {}
    }
}

fn table_is_empty(document: &DocumentMut, path: &[&str]) -> bool {
    match path {
        [name] => document
            .as_table()
            .get(name)
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.is_empty()),
        [parent, child] => document
            .as_table()
            .get(parent)
            .and_then(Item::as_table_like)
            .and_then(|table| table.get(child))
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.is_empty()),
        _ => false,
    }
}

fn set_or_clear_string(table: &mut dyn TableLike, key: &str, text: Option<&str>) {
    match text {
        Some(text) => set_value(table, key, Value::from(text)),
        None => {
            table.remove(key);
        }
    }
}

/// Sets a key, keeping a comment that already sits beside it.
fn set_value(table: &mut dyn TableLike, key: &str, next: Value) {
    if let Some(existing) = table.get_mut(key).and_then(Item::as_value_mut) {
        let decor = existing.decor().clone();
        *existing = next;
        *existing.decor_mut() = decor;
        return;
    }
    table.insert(key, Item::Value(next));
}

fn value_item(item: impl Into<Value>) -> Item {
    Item::Value(item.into())
}

impl StageSettings {
    fn slot_mut(&mut self, stage: Stage) -> &mut Option<StageSetting> {
        match stage {
            Stage::Talking => &mut self.talking,
            Stage::Planning => &mut self.planning,
            Stage::Implementing => &mut self.implementing,
            Stage::Review => &mut self.review,
            Stage::Ready => &mut self.ready,
            Stage::Done => &mut self.done,
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
        story settings_round_trip_efforts_in_an_inline_table {
            let source = "agents = { claude = { strong_effort = \"medium\", home = \"/test/claude\" }, grok = { fast_effort = \"low\" } } # preserve this comment";
            let settings = parse_settings(source).unwrap();
            let mut draft = Configuration::from_settings(&settings);
            expect(draft.claude.strong.effort) to equal(Some(Effort::Medium))
            draft.claude.strong.effort = Some(Effort::Xhigh);
            draft.claude.medium.effort = Some(Effort::High);
            draft.grok.fast.effort = None;
            let saved = revise(source, &draft).unwrap();
            expect(saved.contains("# preserve this comment")) to be_true
            expect(saved.contains("home = \"/test/claude\"")) to be_true
            expect(saved.contains("fast_effort")) to be_false
            let restored = parse_settings(&saved).unwrap();
            expect(restored.agents.choice_for(Agent::Claude, Strength::Strong)) to equal((Some("opus".to_string()), Some(Effort::Xhigh)))
            expect(restored.agents.choice_for(Agent::Claude, Strength::Medium)) to equal((Some("sonnet".to_string()), Some(Effort::High)))
            expect(parse_settings("[agents.codex]\nfast_effort = \"unknown\"").is_err()) to be_true
        }

        expect(read(text)) {
            when(text = None) as there_is_no_file to give_the_defaults { equal(Ok(Settings::default())) }
            when(text = Some("")) as the_file_is_empty to give_the_defaults { equal(Ok(Settings::default())) }
            when(text = Some("[service]\nallowed_origins = [\"http://localhost:4200\"]\n")) as one_line_is_set {
                to keep_the_rest_as_default {
                    equal(Ok(Settings {
                        service: ServiceSettings { port: USUAL_PORT, allowed_origins: vec!["http://localhost:4200".to_string()] },
                        agents: AgentSettings::default(),
                        ..Settings::default()
                    }))
                }
            }
            when(text = Some("[agents.claude]\nprogram = \"/opt/claude\"\n")) as the_program_is_moved {
                to keep_using_the_subscription {
                    have(as_ref().unwrap().agents.claude.clone()) equal(ClaudeSettings { program: "/opt/claude".to_string(), ..ClaudeSettings::default() })
                }
            }
            when(text = Some("[agents.codex]\nuse_subscription = false\n")) as the_codex_key_is_allowed {
                to keep_the_usual_program {
                    have(as_ref().unwrap().agents.codex.clone()) equal(CodexSettings { use_subscription: false, ..CodexSettings::default() })
                }
            }
            when(text = Some("[agents.grok]\nmodels = [\"grok-4.6\"]\n")) as grok_models_are_named {
                to keep_them {
                    have(as_ref().unwrap().agents.grok.models.clone()) equal(Some(vec!["grok-4.6".to_string()]))
                }
            }
            when(text = Some("[agents.gemini]\nmodels = [\"gemini-4-neon-high\"]\nstrong = \"gemini-4-neon-high\"\n")) as gemini_settings_are_named {
                to keep_them {
                    have(as_ref().unwrap().agents.gemini.clone()) equal(GeminiSettings { models: Some(vec!["gemini-4-neon-high".to_string()]), strong: Some("gemini-4-neon-high".to_string()), ..GeminiSettings::default() })
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
            when(text = Some("[agents.codex]\nstrong = \"gpt-6-astra\"\n")) as one_codex_strength_is_set {
                to keep_it_and_leave_the_others_unset {
                    have(as_ref().unwrap().agents.codex.clone()) equal(CodexSettings { strong: Some("gpt-6-astra".to_string()), ..CodexSettings::default() })
                }
            }
            when(text = Some("[agents.claude]\nfast = \"claude-haiku-5-5\"\n")) as one_claude_strength_is_replaced {
                to keep_the_other_built_in_ones {
                    have(as_ref().unwrap().agents.claude.clone()) equal(ClaudeSettings { fast: Some("claude-haiku-5-5".to_string()), ..ClaudeSettings::default() })
                }
            }
            when(text = Some("[workflow]\nautomatic_rounds = 0\n")) as automatic_starts_are_off {
                to keep_the_default_workflow {
                    have(as_ref().unwrap().workflow.clone()) equal(WorkflowSettings { default: "default".to_string(), automatic_rounds: 0 })
                }
            }
            when(text = Some("[workflows.docs]\nstages = [\"talking\", \"implementing\", \"ready\", \"done\"]\n\n[workflow]\ndefault = \"docs\"\n")) as workflows_replace_the_built_in_ones {
                to keep_only_those {
                    have(as_ref().unwrap().workflows.clone().unwrap().into_keys().collect::<Vec<_>>()) equal(vec!["docs".to_string()])
                }
            }
            when(text = Some("[workflows.docs]\nstages = [\"talking\", \"ready\", \"done\"]\n")) as workflows_without_the_default {
                to be_refused { have(clone().unwrap_err().ends_with("config.toml is not valid: [workflow] default names `default`, which is not one of the workflows.")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = [\"implementing\", \"talking\", \"ready\", \"done\"]\n")) as stages_out_of_order {
                to be_refused { have(clone().unwrap_err().contains("out of order, or one twice")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = [\"talking\", \"talking\", \"ready\", \"done\"]\n")) as a_stage_twice {
                to be_refused { have(clone().unwrap_err().contains("out of order, or one twice")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = [\"talking\", \"implementing\", \"done\"]\n")) as a_workflow_without_ready {
                to be_refused { have(clone().unwrap_err().contains("needs `ready` and `done`")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = [\"talking\", \"implementing\", \"ready\"]\n")) as a_workflow_without_done {
                to be_refused { have(clone().unwrap_err().contains("needs `ready` and `done`")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = [\"review\", \"ready\", \"done\"]\n")) as a_workflow_starting_with_review {
                to be_refused { have(clone().unwrap_err().contains("has to start with talking, planning or implementing")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = []\n")) as an_empty_workflow {
                to be_refused { be_err }
            }
            when(text = Some("[workflows]\n")) as no_workflows {
                to be_refused { have(clone().unwrap_err().contains("names no workflow")) be_true }
            }
            when(text = Some("[workflows.default]\nstages = [\"talking\", \"testing\", \"ready\", \"done\"]\n")) as an_unknown_stage {
                to be_refused { be_err }
            }
            when(text = Some("[stages.testing]\nprovider = \"any\"\n")) as settings_for_an_unknown_stage {
                to be_refused { be_err }
            }
            when(text = Some("[stages.review]\nprovider = \"any\"\nstrength = \"strong\"\namong = [\"claude\", \"grok\"]\n")) as any_provider_among_some {
                to keep_it { have(as_ref().unwrap().stages.review.clone().unwrap().among.unwrap().len()) equal(2) }
            }
            when(text = Some("[stages.planning]\nprovider = \"prefer claude\"\n")) as a_preferred_provider {
                to keep_it { have(as_ref().unwrap().stages.planning.clone().unwrap().provider) equal(Some(ProviderChoice::Prefer(Agent::Claude))) }
            }
            when(text = Some("[stages.implementing]\nprovider = \"only codex\"\n")) as only_one_provider {
                to keep_it { have(as_ref().unwrap().stages.implementing.clone().unwrap().provider) equal(Some(ProviderChoice::Only(Agent::Codex))) }
            }
            when(text = Some("[stages.implementing]\nprovider = \"only mistral\"\n")) as an_unknown_agent_in_provider to be_refused { be_err }
            when(text = Some("[stages.implementing]\nprovider = \"only gemini\"\n")) as only_gemini_provider {
                to keep_it { have(as_ref().unwrap().stages.implementing.clone().unwrap().provider) equal(Some(ProviderChoice::Only(Agent::Gemini))) }
            }
            when(text = Some("[stages.implementing]\nprovider = \"always claude\"\n")) as an_unknown_way_to_choose to be_refused { be_err }
            when(text = Some("[stages.implementing]\nprovider = \"prefer\"\n")) as prefer_without_an_agent to be_refused { be_err }
            when(text = Some("[stages.implementing]\nprovider = \"any claude\"\n")) as any_with_an_agent to be_refused { be_err }
            when(text = Some("[stages.implementing]\nstrength = \"huge\"\n")) as an_unknown_strength to be_refused { be_err }
            when(text = Some("[stages.review]\namong = [\"mistral\"]\n")) as an_unknown_agent_in_among to be_refused { be_err }
            when(text = Some("[stages.review]\namong = []\n")) as an_empty_among {
                to be_refused { have(clone().unwrap_err().contains("among lists no provider")) be_true }
            }
            when(text = Some("[stages.review]\nprovider = \"only grok\"\namong = [\"claude\"]\n")) as among_beside_only {
                to be_refused { have(clone().unwrap_err().contains("beside \"only\"")) be_true }
            }
            when(text = Some("[stages.ready]\nprovider = \"any\"\n")) as a_provider_for_ready {
                to be_refused { have(clone().unwrap_err().contains("[stages.ready] can only set a label")) be_true }
            }
            when(text = Some("[stages.done]\nstrength = \"fast\"\n")) as a_strength_for_done {
                to be_refused { have(clone().unwrap_err().contains("[stages.done] can only set a label")) be_true }
            }
            when(text = Some("[stages.ready]\nlabel = \"Ready to merge\"\n")) as a_label_for_ready {
                to keep_it { have(as_ref().unwrap().stages.ready.clone().unwrap().label) equal(Some("Ready to merge".to_string())) }
            }
            when(text = Some("[stages.implementing]\nprovider = \"only codex\"\n\n[stages.review]\nprovider = \"only codex\"\n")) as one_provider_writing_and_reviewing {
                to be_refused { have(clone().unwrap_err().contains("an agent cannot review its own change, and the `default` workflow has both stages")) be_true }
            }
            when(text = Some("[stages.implementing]\nprovider = \"only codex\"\n\n[stages.review]\nprovider = \"only codex\"\n\n[workflows.default]\nstages = [\"talking\", \"implementing\", \"ready\", \"done\"]\n")) as one_provider_on_workflows_without_review {
                to be_accepted { be_ok }
            }
            when(text = Some("[stages.implementing]\nprovider = \"only codex\"\n\n[stages.review]\nprovider = \"only grok\"\n")) as different_providers_writing_and_reviewing {
                to be_accepted { be_ok }
            }
            when(text = Some("[service")) as the_file_is_not_toml to be_refused { be_err }
        }
    }

    fn draft_of(text: &str) -> Configuration {
        Configuration::from_settings(&parse_settings(text).unwrap())
    }

    fn kept(text: &str, part: &str) -> bool {
        revise(text, &draft_of(text)).unwrap().contains(part)
    }

    /// Agent sections written inline, the shape a hand-edited file often has.
    ///
    /// The screen does not own the program, the subscription switch, the model
    /// lists or the folders. A save still has to leave them in the file.
    fn inline_agents() -> &'static str {
        "\
[agents]
stop_grace_seconds = 5
claude = { program = \"/opt/claude\", use_subscription = false }
codex = { home = \"/opt/codex\", models = [\"gpt-kept\"] }
grok = { writable = [\"/work\"], models = [\"grok-kept\"] }
"
    }

    /// The same agent settings, with the whole `[agents]` table inline as well.
    fn inline_agents_root() -> &'static str {
        "\
agents = { stop_grace_seconds = 5, claude = { program = \"/opt/claude\", use_subscription = false }, codex = { home = \"/opt/codex\", models = [\"gpt-kept\"] }, grok = { writable = [\"/work\"], models = [\"grok-kept\"] } }
"
    }

    /// A workflows table this file owns, with a comment inside it.
    ///
    /// Saving without changing the workflows has to leave that table as it
    /// stands, comment included. Rewriting it would drop the comment.
    fn own_workflows() -> &'static str {
        "\
[workflow]
default = \"notes\"

[workflows.notes]
# kept with the workflow
description = \"Notes\"
stages = [\"talking\", \"implementing\", \"ready\", \"done\"]
"
    }

    lets_expect! {
        expect(revise("", &Configuration::from_settings(&Settings::default()))) as saving_the_defaults_over_nothing {
            to write_nothing { have(unwrap().trim().is_empty()) be_true }
        }

        expect(kept(text, part)) {
            when(text = "[service]\n# the port\nport = 1\n", part = "# the port") as a_comment_elsewhere {
                to stay { be_true }
            }
            when(text = "[service]\nport = 1\n", part = "port = 1") as an_unrelated_key {
                to stay { be_true }
            }
            when(text = "[service]\nport = 1\n", part = "[workflows]") as the_built_in_workflows {
                to stay_out_of_the_file { be_false }
            }
            when(text = own_workflows(), part = "# kept with the workflow") as an_unchanged_workflows_table {
                to keep_the_comment { be_true }
            }
            when(text = own_workflows(), part = "description = \"Notes\"") as an_unchanged_workflows_description {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "use_subscription = false") as an_inline_subscription_switch {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "/opt/claude") as an_inline_program {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "/opt/codex") as an_inline_codex_home {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "gpt-kept") as an_inline_codex_model_list {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "/work") as an_inline_grok_folder {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "grok-kept") as an_inline_grok_model_list {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "stop_grace_seconds = 5") as a_stop_grace_beside_inline_agents {
                to stay { be_true }
            }
            when(text = inline_agents(), part = "claude = {") as an_inline_claude_table {
                to stay_inline { be_true }
            }
            when(text = inline_agents_root(), part = "use_subscription = false") as an_inline_agents_table {
                to keep_the_subscription_switch { be_true }
            }
        }

        expect(revise(text, &changed).unwrap()) {
            when(text = "[open]\neditor = \"code {path}\" # visual studio\n", changed = { let mut draft = draft_of("[open]\neditor = \"code {path}\"\n"); draft.editor = "zed {path}".to_string(); draft }) as a_comment_beside_a_changed_key {
                to keep_the_comment { have(contains("# visual studio")) be_true }
                to keep_the_new_command { have(contains("zed {path}")) be_true }
            }
            when(text = "[agents.claude]\nprogram = \"/opt/claude\"\nfast = \"custom-haiku\"\n", changed = { let mut draft = draft_of("[agents.claude]\nprogram = \"/opt/claude\"\nfast = \"custom-haiku\"\n"); draft.claude.fast.model = "haiku".to_string(); draft }) as a_strength_put_back_to_the_default {
                to keep_the_program { have(contains("/opt/claude")) be_true }
                to drop_the_strength { have(contains("custom-haiku")) be_false }
            }
            when(text = "[service]\nport = 9\n\n[stages.planning]\nprovider = \"only claude\"\n", changed = { let mut draft = draft_of("[service]\nport = 9\n\n[stages.planning]\nprovider = \"only claude\"\n"); draft.stages.iter_mut().find(|stage| stage.stage == Stage::Planning).unwrap().provider = None; draft.stages.iter_mut().find(|stage| stage.stage == Stage::Planning).unwrap().strength = None; draft }) as a_stage_cleared {
                to keep_the_port { have(contains("port = 9")) be_true }
                to drop_the_stage { have(contains("only claude")) be_false }
            }
            when(text = "[workflows.docs]\ndescription = \"Docs\"\nstages = [\"talking\", \"ready\", \"done\"]\n\n[workflow]\ndefault = \"docs\"\n", changed = Configuration::from_settings(&Settings::default())) as workflows_put_back_to_the_built_in_pair {
                to drop_the_table { have(contains("[workflows")) be_false }
                to drop_the_old_default { have(contains("docs")) be_false }
            }
            when(text = own_workflows(), changed = { let mut draft = draft_of(own_workflows()); draft.editor = "zed {path}".to_string(); draft }) as the_editor_changes_and_the_workflows_do_not {
                to keep_the_comment { have(contains("# kept with the workflow")) be_true }
                to keep_the_description { have(contains("description = \"Notes\"")) be_true }
                to write_the_editor { have(contains("zed {path}")) be_true }
            }
            when(text = inline_agents(), changed = { let mut draft = draft_of(inline_agents()); draft.claude.fast.model = "custom-haiku".to_string(); draft }) as a_strength_written_into_an_inline_table {
                to keep_the_program { have(contains("/opt/claude")) be_true }
                to keep_the_subscription_switch { have(contains("use_subscription = false")) be_true }
                to keep_the_codex_home { have(contains("/opt/codex")) be_true }
                to keep_the_codex_models { have(contains("gpt-kept")) be_true }
                to keep_the_grok_folder { have(contains("/work")) be_true }
                to keep_the_grok_models { have(contains("grok-kept")) be_true }
                to keep_the_stop_grace { have(contains("stop_grace_seconds = 5")) be_true }
                to stay_inline { have(contains("claude = {")) be_true }
                to write_the_strength { have(contains("custom-haiku")) be_true }
            }
            when(text = inline_agents_root(), changed = { let mut draft = draft_of(inline_agents_root()); draft.claude.fast.model = "custom-haiku".to_string(); draft }) as a_strength_written_into_a_fully_inline_agents_table {
                to keep_the_program { have(contains("/opt/claude")) be_true }
                to keep_the_subscription_switch { have(contains("use_subscription = false")) be_true }
                to keep_the_codex_home { have(contains("/opt/codex")) be_true }
                to keep_the_grok_folder { have(contains("/work")) be_true }
                to write_the_strength { have(contains("custom-haiku")) be_true }
            }
        }

        expect(revise("", &draft)) {
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[0].stages = vec![Stage::Implementing, Stage::Talking, Stage::Ready, Stage::Done]; draft }) as stages_out_of_order {
                to be_refused { have(clone().unwrap_err().contains("out of order, or one twice")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[0].stages = vec![Stage::Talking, Stage::Talking, Stage::Ready, Stage::Done]; draft }) as a_stage_twice {
                to be_refused { have(clone().unwrap_err().contains("out of order, or one twice")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[0].stages = vec![Stage::Talking, Stage::Implementing, Stage::Done]; draft }) as a_workflow_without_ready {
                to be_refused { have(clone().unwrap_err().contains("needs `ready` and `done`")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[0].stages = vec![Stage::Talking, Stage::Implementing, Stage::Ready]; draft }) as a_workflow_without_done {
                to be_refused { have(clone().unwrap_err().contains("needs `ready` and `done`")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[0].stages = vec![Stage::Review, Stage::Ready, Stage::Done]; draft }) as a_workflow_starting_with_review {
                to be_refused { have(clone().unwrap_err().contains("has to start with talking, planning or implementing")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows.clear(); draft }) as no_workflows {
                to be_refused { have(clone().unwrap_err().contains("names no workflow")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.default_workflow = "missing".to_string(); draft }) as a_default_that_is_not_a_workflow {
                to be_refused { have(clone().unwrap_err().contains("which is not one of the workflows")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[0].name = "  ".to_string(); draft }) as an_empty_workflow_name {
                to be_refused { have(clone().unwrap_err().contains("A workflow needs a name")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); draft.workflows[1].name = "default".to_string(); draft }) as two_workflows_with_one_name {
                to be_refused { have(clone().unwrap_err().contains("Two workflows are named `default`")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); let ready = draft.stages.iter_mut().find(|stage| stage.stage == Stage::Ready).unwrap(); ready.provider = Some(StageProvider::Any { among: Vec::new() }); draft }) as a_provider_for_ready {
                to be_refused { have(clone().unwrap_err().contains("[stages.ready] can only set a label")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); for stage in &mut draft.stages { if stage.stage == Stage::Implementing || stage.stage == Stage::Review { stage.provider = Some(StageProvider::Only { agent: "claude".to_string() }); } } draft }) as one_provider_writing_and_reviewing {
                to be_refused { have(clone().unwrap_err().contains("cannot review its own change")) be_true }
            }
            when(draft = { let mut draft = Configuration::from_settings(&Settings::default()); let review = draft.stages.iter_mut().find(|stage| stage.stage == Stage::Review).unwrap(); review.provider = Some(StageProvider::Any { among: vec!["mistral".to_string()] }); draft }) as an_unknown_agent {
                to be_refused { have(clone().unwrap_err().contains("is not an agent Teryon knows")) be_true }
            }
        }
    }
}
