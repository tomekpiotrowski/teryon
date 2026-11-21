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

use serde::Deserialize;
use thiserror::Error;

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
        Agent::parse(&name)
            .map(Self)
            .ok_or_else(|| format!("`{name}` is not an agent Teryon knows: claude, codex or grok"))
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
                format!("`{name}` is not an agent Teryon knows: claude, codex or grok")
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
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
}

impl AgentSettings {
    /// The model an agent runs for a strength. Nothing means the program's own default.
    pub fn model_for(&self, agent: Agent, strength: Strength) -> Option<String> {
        let (strong, medium, fast) = match agent {
            Agent::Claude => (&self.claude.strong, &self.claude.medium, &self.claude.fast),
            Agent::Codex => (&self.codex.strong, &self.codex.medium, &self.codex.fast),
            Agent::Grok => (&self.grok.strong, &self.grok.medium, &self.grok.fast),
        };
        match strength {
            Strength::Strong => strong.clone(),
            Strength::Medium => medium.clone(),
            Strength::Fast => fast.clone(),
        }
    }

    /// The program that runs an agent.
    pub fn program(&self, agent: Agent) -> &str {
        match agent {
            Agent::Claude => &self.claude.program,
            Agent::Codex => &self.codex.program,
            Agent::Grok => &self.grok.program,
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
    /// The model for each strength. Claude's short names are built in: they always mean its newest version, so they never go stale.
    pub strong: Option<String>,
    pub medium: Option<String>,
    pub fast: Option<String>,
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        Self {
            program: "claude".to_string(),
            use_subscription: true,
            models: None,
            strong: Some("opus".to_string()),
            medium: Some("sonnet".to_string()),
            fast: Some("haiku".to_string()),
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
            when(text = Some("[stages.implementing]\nprovider = \"only gemini\"\n")) as an_unknown_agent_in_provider to be_refused { be_err }
            when(text = Some("[stages.implementing]\nprovider = \"always claude\"\n")) as an_unknown_way_to_choose to be_refused { be_err }
            when(text = Some("[stages.implementing]\nprovider = \"prefer\"\n")) as prefer_without_an_agent to be_refused { be_err }
            when(text = Some("[stages.implementing]\nprovider = \"any claude\"\n")) as any_with_an_agent to be_refused { be_err }
            when(text = Some("[stages.implementing]\nstrength = \"huge\"\n")) as an_unknown_strength to be_refused { be_err }
            when(text = Some("[stages.review]\namong = [\"gemini\"]\n")) as an_unknown_agent_in_among to be_refused { be_err }
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
}
