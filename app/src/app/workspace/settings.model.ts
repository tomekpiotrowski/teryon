/**
 * The settings screen's draft, and the rules it checks before saving.
 *
 * The draft remembers which workflows have never been saved, and which stage
 * fields the person has actually changed. A field left alone is saved as the
 * file had it, so opening the screen and saving writes nothing new.
 *
 * The sentences are the ones the service uses, so a mistake reads the same
 * whether the screen or the file caught it.
 */

import type { Configuration, ConfiguredStage, Stage, StageProvider, Strength, StrengthModels } from '../service/protocol';
import { AGENTS, type AgentName, agentLabel, type ModelLists, STAGES, usualStageLabel } from './workspace.model';

/** How a stage picks who works. Keep leaves the thread's agent as it is. */
export type ProviderMode = 'keep' | 'any' | 'prefer' | 'only';

/** One workflow while it is being edited. The name stays editable until it has been saved. */
export interface WorkflowDraft {
  readonly key: string;
  readonly name: string;
  readonly description: string;
  readonly stages: readonly Stage[];
  readonly locked: boolean;
}

/** One stage while it is being edited. */
export interface StageDraft {
  readonly stage: Stage;
  readonly label: string;
  readonly mode: ProviderMode;
  readonly agent: string;
  readonly among: readonly string[];
  readonly strength: Strength | null;
  /** What the file said, kept until the person changes that part. */
  readonly rawProvider: StageProvider | null;
  readonly rawStrength: Strength | null;
  readonly modeTouched: boolean;
  readonly strengthTouched: boolean;
  readonly amongTouched: boolean;
}

/** Everything the screen is editing. */
export interface SettingsDraft {
  readonly defaultKey: string;
  readonly roundsText: string;
  readonly ownWorkflows: boolean;
  readonly workflows: readonly WorkflowDraft[];
  readonly stages: readonly StageDraft[];
  readonly claude: StrengthModels;
  readonly codex: StrengthModels;
  readonly grok: StrengthModels;
  readonly editor: string;
  readonly terminal: string;
  /** Set when a workflow was removed, until the next save replaces the draft. */
  readonly removalNote: string | null;
}

const CLAUDE_PLACEHOLDERS: Readonly<Record<Strength, string>> = {
  strong: 'opus',
  medium: 'sonnet',
  fast: 'haiku',
};

const STRENGTHS: readonly Strength[] = ['strong', 'medium', 'fast'];

const STRENGTH_LABELS: Readonly<Record<Strength, string>> = {
  strong: 'Strong',
  medium: 'Medium',
  fast: 'Fast',
};

/** The largest number of agents the file can store. */
const MOST_ROUNDS = 4_294_967_295;

/** A workflow as the screen lists it. */
export interface WorkflowRow {
  readonly key: string;
  readonly name: string;
  readonly description: string;
  readonly locked: boolean;
  readonly stageChoices: readonly { readonly stage: Stage; readonly label: string; readonly checked: boolean; readonly fixed: boolean }[];
}

/** A stage as the screen lists it. */
export interface StageRow {
  readonly stage: Stage;
  readonly heading: string;
  readonly label: string;
  readonly placeholder: string;
  readonly worked: boolean;
  readonly mode: ProviderMode;
  readonly agent: string;
  readonly strength: Strength;
  readonly showAgent: boolean;
  readonly showAmong: boolean;
  readonly showStrength: boolean;
  readonly among: readonly { readonly agent: AgentName; readonly label: string; readonly checked: boolean }[];
}

/** One agent's three model fields. */
export interface ModelGroup {
  readonly agent: AgentName;
  readonly title: string;
  readonly fields: readonly { readonly strength: Strength; readonly label: string; readonly value: string; readonly placeholder: string; readonly suggestions: readonly string[]; readonly listId: string }[];
}

/** What the settings screen draws. */
export interface SettingsView {
  readonly loading: boolean;
  readonly loadProblem: string | null;
  readonly problem: string | null;
  readonly saving: boolean;
  readonly dirty: boolean;
  readonly removalNote: string | null;
  readonly rounds: string;
  readonly defaultKey: string;
  readonly workflowOptions: readonly { readonly key: string; readonly label: string }[];
  readonly canRemove: boolean;
  readonly workflows: readonly WorkflowRow[];
  readonly stages: readonly StageRow[];
  readonly models: readonly ModelGroup[];
  readonly editor: string;
  readonly terminal: string;
}

/** The draft the screen starts from. Every workflow already has a saved name. */
export function fromConfiguration(configuration: Configuration): SettingsDraft {
  const workflows = configuration.workflows.map((workflow) => ({
    key: workflow.name,
    name: workflow.name,
    description: workflow.description,
    stages: ordered(workflow.stages),
    locked: true,
  }));
  const chosen = workflows.find((workflow) => workflow.name === configuration.default_workflow);
  return {
    defaultKey: chosen?.key ?? workflows[0]?.key ?? '',
    roundsText: String(configuration.automatic_rounds),
    ownWorkflows: configuration.own_workflows,
    workflows,
    stages: STAGES.map((stage) => stageFrom(stage, configuration.stages.find((item) => item.stage === stage))),
    claude: { ...configuration.claude },
    codex: { ...configuration.codex },
    grok: { ...configuration.grok },
    editor: configuration.editor,
    terminal: configuration.terminal,
    removalNote: null,
  };
}

/** The draft as the service expects it. Fields the person left alone stay as the file had them. */
export function toConfiguration(draft: SettingsDraft): Configuration {
  return {
    default_workflow: nameOf(draft, draft.defaultKey),
    automatic_rounds: wholeNumber(draft.roundsText) ?? 0,
    own_workflows: draft.ownWorkflows,
    workflows: draft.workflows.map((workflow) => ({
      name: workflow.name.trim(),
      description: workflow.description.trim(),
      stages: [...workflow.stages],
    })),
    stages: STAGES.map((stage) => wireStage(draft.stages.find((item) => item.stage === stage) ?? stageFrom(stage, undefined))),
    claude: { ...draft.claude },
    codex: { ...draft.codex },
    grok: { ...draft.grok },
    editor: draft.editor,
    terminal: draft.terminal,
  };
}

/**
 * Why the draft cannot be saved, in the service's words.
 *
 * Nothing when it can. Checked here so a mistake never leaves the machine,
 * and again by the service in case this copy of the screen is old.
 */
export function refusal(draft: SettingsDraft): string | null {
  if (draft.workflows.length === 0) return '[workflows] names no workflow.';
  const seen = new Set<string>();
  for (const workflow of draft.workflows) {
    const name = workflow.name.trim();
    if (!name) return 'A workflow needs a name.';
    if (seen.has(name)) return `Two workflows are named \`${name}\`.`;
    seen.add(name);
    const stages = refusalOfWorkflow(name, workflow.stages);
    if (stages) return stages;
  }
  const chosen = nameOf(draft, draft.defaultKey);
  if (!seen.has(chosen)) return `[workflow] default names \`${chosen}\`, which is not one of the workflows.`;
  for (const stage of draft.stages) {
    const problem = refusalOfStage(wireStage(stage));
    if (problem) return problem;
  }
  const both = refusalOfOnlyBoth(draft);
  if (both) return both;
  if (wholeNumber(draft.roundsText) === null) return 'The number of agents Teryon starts on its own has to be a whole number.';
  return null;
}

/** Whether saving would write the same choices the screen was given. */
export function unchanged(draft: SettingsDraft, saved: Configuration): boolean {
  if (refusal(draft)) return false;
  return JSON.stringify(toConfiguration(draft)) === JSON.stringify(toConfiguration(fromConfiguration(saved)));
}

/** Adds a workflow the person has not saved yet, so its name can still be typed. */
export function withWorkflow(draft: SettingsDraft, key: string): SettingsDraft {
  return {
    ...draft,
    workflows: [...draft.workflows, { key, name: '', description: '', stages: [...STAGES], locked: false }],
  };
}

/** Removes a workflow. The last one stays: a thread has to follow something. */
export function withoutWorkflow(draft: SettingsDraft, key: string): SettingsDraft {
  if (draft.workflows.length <= 1) return draft;
  const workflows = draft.workflows.filter((workflow) => workflow.key !== key);
  return {
    ...draft,
    workflows,
    defaultKey: draft.defaultKey === key ? workflows[0].key : draft.defaultKey,
    removalNote: 'Threads still on this workflow follow the default afterwards.',
  };
}

/** Renames a workflow that has not been saved. A saved name is what threads remember. */
export function renameWorkflow(draft: SettingsDraft, key: string, name: string): SettingsDraft {
  return mapWorkflow(draft, key, (workflow) => (workflow.locked ? workflow : { ...workflow, name }));
}

/** Sets the one line that says what a workflow suits. */
export function describeWorkflow(draft: SettingsDraft, key: string, description: string): SettingsDraft {
  return mapWorkflow(draft, key, (workflow) => ({ ...workflow, description }));
}

/** Includes a stage or leaves it out. Ready and Done stay: a change has to end. */
export function toggleWorkflowStage(draft: SettingsDraft, key: string, stage: Stage): SettingsDraft {
  if (stage === 'ready' || stage === 'done') return draft;
  return mapWorkflow(draft, key, (workflow) => {
    const stages = workflow.stages.includes(stage) ? workflow.stages.filter((item) => item !== stage) : [...workflow.stages, stage];
    return { ...workflow, stages: ordered(stages) };
  });
}

/** Chooses which workflow new threads start on, including one not saved yet. */
export function chooseDefault(draft: SettingsDraft, key: string): SettingsDraft {
  return { ...draft, defaultKey: key };
}

/** Sets how many agents may start on their own, as typed. */
export function setRounds(draft: SettingsDraft, roundsText: string): SettingsDraft {
  return { ...draft, roundsText };
}

/** Sets the word a stage is called, or clears it so the usual word returns. */
export function setLabel(draft: SettingsDraft, stage: Stage, label: string): SettingsDraft {
  return mapStage(draft, stage, (item) => ({ ...item, label }));
}

/** Chooses who works in a stage. */
export function setMode(draft: SettingsDraft, stage: Stage, mode: ProviderMode): SettingsDraft {
  return mapStage(draft, stage, (item) => ({ ...item, mode, modeTouched: true, agent: item.agent || 'claude' }));
}

/** Chooses the agent a preferring or only stage names. */
export function setAgent(draft: SettingsDraft, stage: Stage, agent: string): SettingsDraft {
  return mapStage(draft, stage, (item) => ({ ...item, agent, modeTouched: true }));
}

/** Ticks or unticks a provider that Any or Prefer may fall back to. None ticked means every one installed. */
export function toggleAmong(draft: SettingsDraft, stage: Stage, agent: string): SettingsDraft {
  return mapStage(draft, stage, (item) => ({
    ...item,
    among: item.among.includes(agent) ? item.among.filter((name) => name !== agent) : [...item.among, agent],
    amongTouched: true,
  }));
}

/** Chooses the strength a stage runs. */
export function setStrength(draft: SettingsDraft, stage: Stage, strength: Strength): SettingsDraft {
  return mapStage(draft, stage, (item) => ({ ...item, strength, strengthTouched: true }));
}

/** Sets the model an agent runs for a strength. Empty means the default again. */
export function setModel(draft: SettingsDraft, agent: AgentName, strength: Strength, value: string): SettingsDraft {
  return { ...draft, [agent]: { ...draft[agent], [strength]: value } };
}

/** Sets the command that opens a copy in an editor. Empty means the menu does not offer it. */
export function setEditor(draft: SettingsDraft, editor: string): SettingsDraft {
  return { ...draft, editor };
}

/** Sets the command that opens a copy in a terminal. Empty means the menu does not offer it. */
export function setTerminal(draft: SettingsDraft, terminal: string): SettingsDraft {
  return { ...draft, terminal };
}

/** The screen's view of a draft, or of the wait before the draft arrives. */
export function settingsView(
  draft: SettingsDraft | null,
  saved: Configuration | null,
  problem: string | null,
  saving: boolean,
  loadProblem: string | null,
  models: ModelLists,
): SettingsView {
  if (!draft) {
    return {
      loading: !loadProblem,
      loadProblem,
      problem,
      saving: false,
      dirty: false,
      removalNote: null,
      rounds: '',
      defaultKey: '',
      workflowOptions: [],
      canRemove: false,
      workflows: [],
      stages: [],
      models: [],
      editor: '',
      terminal: '',
    };
  }
  return {
    loading: false,
    loadProblem,
    problem,
    saving,
    dirty: saved !== null && !unchanged(draft, saved),
    removalNote: draft.removalNote,
    rounds: draft.roundsText,
    defaultKey: draft.defaultKey,
    workflowOptions: draft.workflows.map((workflow) => ({ key: workflow.key, label: workflow.name.trim() || 'New workflow' })),
    canRemove: draft.workflows.length > 1,
    workflows: draft.workflows.map((workflow) => ({
      key: workflow.key,
      name: workflow.name,
      description: workflow.description,
      locked: workflow.locked,
      stageChoices: STAGES.map((stage) => ({
        stage,
        label: usualStageLabel(stage),
        checked: workflow.stages.includes(stage),
        fixed: stage === 'ready' || stage === 'done',
      })),
    })),
    stages: draft.stages.map((stage) => stageRow(stage)),
    models: AGENTS.map((agent) => modelGroup(agent, draft[agent], models[agent])),
    editor: draft.editor,
    terminal: draft.terminal,
  };
}

function stageFrom(stage: Stage, configured: ConfiguredStage | undefined): StageDraft {
  const provider = configured?.provider ?? null;
  const strength = configured?.strength ?? null;
  return {
    stage,
    label: configured?.label ?? '',
    mode: provider ? provider.kind : strength ? 'any' : 'keep',
    agent: provider && provider.kind !== 'any' ? provider.agent : 'claude',
    among: provider && provider.kind !== 'only' ? provider.among : [],
    strength,
    rawProvider: provider,
    rawStrength: strength,
    modeTouched: false,
    strengthTouched: false,
    amongTouched: false,
  };
}

function wireStage(stage: StageDraft): ConfiguredStage {
  if (stage.mode === 'keep') return { stage: stage.stage, label: stage.label, provider: null, strength: null };
  const provider = stage.modeTouched || stage.amongTouched ? explicitProvider(stage) : stage.rawProvider;
  const strength = stage.strengthTouched ? (stage.strength ?? 'medium') : stage.rawStrength;
  return { stage: stage.stage, label: stage.label, provider, strength };
}

function explicitProvider(stage: StageDraft): StageProvider {
  if (stage.mode === 'only') return { kind: 'only', agent: stage.agent.trim() };
  if (stage.mode === 'prefer') return { kind: 'prefer', agent: stage.agent.trim(), among: [...stage.among] };
  return { kind: 'any', among: [...stage.among] };
}

function refusalOfWorkflow(name: string, stages: readonly Stage[]): string | null {
  for (let index = 1; index < stages.length; index += 1) {
    if (stageIndex(stages[index - 1]) >= stageIndex(stages[index])) {
      return `the \`${name}\` workflow lists its stages out of order, or one twice. The order is talking, planning, implementing, review, ready, done.`;
    }
  }
  if (!stages.includes('ready') || !stages.includes('done')) {
    return `the \`${name}\` workflow needs \`ready\` and \`done\`: a change has to end somewhere.`;
  }
  const first = stages[0];
  if (first === undefined || stageIndex(first) >= stageIndex('review')) {
    return `the \`${name}\` workflow has to start with talking, planning or implementing.`;
  }
  return null;
}

function refusalOfStage(stage: ConfiguredStage): string | null {
  const chooses = stage.provider !== null || stage.strength !== null;
  if ((stage.stage === 'ready' || stage.stage === 'done') && chooses) {
    return `[stages.${stage.stage}] can only set a label: nobody works in that stage, it waits for you.`;
  }
  const names = providerNames(stage.provider);
  for (const name of names) {
    if (!knownAgent(name)) return `\`${name}\` is not an agent Teryon knows: claude, codex or grok`;
  }
  return null;
}

function refusalOfOnlyBoth(draft: SettingsDraft): string | null {
  const author = onlyAgent(draft, 'implementing');
  const reviewer = onlyAgent(draft, 'review');
  if (!author || author !== reviewer) return null;
  const both = [...draft.workflows]
    .sort((left, right) => left.name.trim().localeCompare(right.name.trim()))
    .find((workflow) => workflow.stages.includes('implementing') && workflow.stages.includes('review'));
  if (!both) return null;
  return `implementing and review both allow only ${agentLabel(author)}, but an agent cannot review its own change, and the \`${both.name.trim()}\` workflow has both stages.`;
}

function onlyAgent(draft: SettingsDraft, stage: Stage): AgentName | null {
  const found = draft.stages.find((item) => item.stage === stage);
  if (!found || found.mode !== 'only') return null;
  return knownAgent(found.agent);
}

function providerNames(provider: StageProvider | null): readonly string[] {
  if (!provider) return [];
  if (provider.kind === 'only') return [provider.agent];
  return provider.kind === 'prefer' ? [provider.agent, ...provider.among] : provider.among;
}

function knownAgent(name: string): AgentName | null {
  const trimmed = name.trim();
  return AGENTS.find((agent) => agent === trimmed) ?? null;
}

function wholeNumber(text: string): number | null {
  if (!/^\d+$/.test(text.trim())) return null;
  const value = Number(text);
  if (!Number.isSafeInteger(value) || value > MOST_ROUNDS) return null;
  return value;
}

function nameOf(draft: SettingsDraft, key: string): string {
  return draft.workflows.find((workflow) => workflow.key === key)?.name.trim() ?? '';
}

function ordered(stages: readonly Stage[]): Stage[] {
  return STAGES.filter((stage) => stages.includes(stage));
}

function stageIndex(stage: Stage): number {
  return STAGES.indexOf(stage);
}

function mapWorkflow(draft: SettingsDraft, key: string, change: (workflow: WorkflowDraft) => WorkflowDraft): SettingsDraft {
  return { ...draft, workflows: draft.workflows.map((workflow) => (workflow.key === key ? change(workflow) : workflow)) };
}

function mapStage(draft: SettingsDraft, stage: Stage, change: (item: StageDraft) => StageDraft): SettingsDraft {
  return { ...draft, stages: draft.stages.map((item) => (item.stage === stage ? change(item) : item)) };
}

function stageRow(stage: StageDraft): StageRow {
  const worked = stage.stage !== 'ready' && stage.stage !== 'done';
  return {
    stage: stage.stage,
    heading: usualStageLabel(stage.stage),
    label: stage.label,
    placeholder: usualStageLabel(stage.stage),
    worked,
    mode: stage.mode,
    agent: stage.agent,
    strength: stage.strength ?? 'medium',
    showAgent: worked && (stage.mode === 'prefer' || stage.mode === 'only'),
    showAmong: worked && (stage.mode === 'any' || stage.mode === 'prefer'),
    showStrength: worked && stage.mode !== 'keep',
    among: AGENTS.map((agent) => ({ agent, label: agentLabel(agent), checked: stage.among.includes(agent) })),
  };
}

function modelGroup(agent: AgentName, models: StrengthModels, suggestions: readonly string[]): ModelGroup {
  return {
    agent,
    title: agentLabel(agent),
    fields: STRENGTHS.map((strength) => ({
      strength,
      label: STRENGTH_LABELS[strength],
      value: models[strength],
      placeholder: agent === 'claude' ? CLAUDE_PLACEHOLDERS[strength] : 'Program default',
      suggestions,
      listId: `settings-model-${agent}-${strength}`,
    })),
  };
}
