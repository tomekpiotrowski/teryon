import type { Configuration, Stage } from '../service/protocol';
import {
  fromConfiguration,
  refusal,
  renameWorkflow,
  setEffort,
  setModel,
  setMode,
  settingsView,
  toConfiguration,
  toggleWorkflowStage,
  unchanged,
  withWorkflow,
  withoutWorkflow,
} from './settings.model';
import { modelOptionForTest as model } from './model.testing';
import { NO_MODELS, STAGES } from './workspace.model';

const SAVED: Configuration = {
  default_workflow: 'default',
  automatic_rounds: 3,
  own_workflows: false,
  workflows: [
    {
      name: 'default',
      description: 'Anything that needs thinking through: features, bugs, changes to design.',
      stages: [...STAGES],
    },
    {
      name: 'small',
      description: 'Small, clear changes: documentation, wording, settings.',
      stages: ['talking', 'implementing', 'ready', 'done'],
    },
  ],
  stages: STAGES.map((stage) => ({ stage, label: '', provider: null, strength: null })),
  claude: { strong: { model: 'opus', effort: null }, medium: { model: 'sonnet', effort: null }, fast: { model: 'haiku', effort: null } },
  codex: { strong: { model: '', effort: null }, medium: { model: '', effort: null }, fast: { model: '', effort: null } },
  grok: { strong: { model: '', effort: null }, medium: { model: '', effort: null }, fast: { model: '', effort: null } },
  gemini: { strong: { model: '', effort: null }, medium: { model: '', effort: null }, fast: { model: '', effort: null } },
  editor: '',
  terminal: '',
};

function draftOf(configuration: Configuration = SAVED) {
  return fromConfiguration(configuration);
}

function withStages(name: string, stages: readonly Stage[]) {
  const draft = draftOf();
  return {
    ...draft,
    workflows: draft.workflows.map((workflow) => (workflow.name === name ? { ...workflow, stages } : workflow)),
  };
}

describe('the settings draft', () => {
  it('round-trips effort separately from the model and offers the catalog levels', () => {
    const opus = model('claude-opus-5-5', 'Opus 5.5', ['low', 'medium', 'xhigh'], 'medium');
    const models = { ...NO_MODELS, claude: [opus] };
    const draft = setEffort(draftOf(), 'claude', 'strong', 'xhigh');
    const strong = (state: typeof draft) => settingsView(state, SAVED, null, false, null, models).models[0].fields[0];
    expect(strong(draft).suggestions[0].label).toBe('Opus 5.5');
    expect(strong(draft).efforts.map((effort) => effort.label)).toEqual(['Low', 'Medium', 'Extra high']);
    const custom = setModel(draft, 'claude', 'strong', 'custom-claude');
    expect(strong(custom).efforts).toHaveLength(6);
    expect(toConfiguration(custom).claude.strong).toEqual({ model: 'custom-claude', effort: 'xhigh' });
    const unlisted = setEffort(draftOf(), 'claude', 'strong', 'ultra');
    expect(strong(unlisted).efforts.map((effort) => effort.value)).toEqual(['low', 'medium', 'xhigh', 'ultra']);
    expect(toConfiguration(setEffort(custom, 'claude', 'strong', null)).claude.strong.effort).toBeNull();
  });

  it('saves an untouched screen as the choices it was given', () => {
    const draft = draftOf();
    expect(toConfiguration(draft)).toEqual(SAVED);
    expect(unchanged(draft, SAVED)).toBe(true);
  });

  it('keeps a missing provider when a strength was set and nobody changed it', () => {
    const saved: Configuration = {
      ...SAVED,
      stages: SAVED.stages.map((stage) => (stage.stage === 'talking' ? { ...stage, strength: 'fast' } : stage)),
    };
    expect(toConfiguration(draftOf(saved)).stages[0]).toEqual({ stage: 'talking', label: '', provider: null, strength: 'fast' });
  });

  it('accepts a stage that names only one provider and no fallback list', () => {
    expect(refusal(setMode(draftOf(), 'implementing', 'only'))).toBeNull();
  });

  it('refuses stages out of order or listed twice', () => {
    expect(refusal(withStages('default', ['implementing', 'talking', 'ready', 'done']))).toBe(
      'the `default` workflow lists its stages out of order, or one twice. The order is talking, planning, implementing, review, ready, done.',
    );
  });

  it('refuses a workflow without Ready and Done', () => {
    expect(refusal(withStages('small', ['talking', 'implementing', 'ready']))).toBe(
      'the `small` workflow needs `ready` and `done`: a change has to end somewhere.',
    );
  });

  it('refuses a workflow that starts at review', () => {
    expect(refusal(withStages('default', ['review', 'ready', 'done']))).toBe(
      'the `default` workflow has to start with talking, planning or implementing.',
    );
  });

  it('refuses a provider or a strength on Ready or Done', () => {
    expect(refusal(setMode(draftOf(), 'ready', 'any'))).toBe(
      '[stages.ready] can only set a label: nobody works in that stage, it waits for you.',
    );
  });

  it('refuses a workflow with no name', () => {
    expect(refusal(withWorkflow(draftOf(), 'new-1'))).toBe('A workflow needs a name.');
  });

  it('refuses two workflows with the same name', () => {
    const named = renameWorkflow(withWorkflow(draftOf(), 'new-1'), 'new-1', 'default');
    expect(refusal(named)).toBe('Two workflows are named `default`.');
  });

  it('refuses a default that names a workflow not on the screen', () => {
    expect(refusal({ ...draftOf(), defaultKey: 'missing' })).toBe(
      '[workflow] default names ``, which is not one of the workflows.',
    );
  });

  it('keeps a new workflow’s name editable, and locks it once that draft came from a save', () => {
    const added = withWorkflow(draftOf(), 'new-1');
    expect(added.workflows.at(-1)?.locked).toBe(false);
    const saved = toConfiguration(renameWorkflow(added, 'new-1', 'hotfix'));
    expect(fromConfiguration({ ...saved, own_workflows: true }).workflows.at(-1)?.locked).toBe(true);
  });

  it('leaves Ready and Done on, and tells the person when a workflow is removed', () => {
    const toggled = toggleWorkflowStage(draftOf(), 'default', 'ready');
    expect(toggled.workflows[0].stages).toContain('ready');
    const removed = withoutWorkflow(draftOf(), 'small');
    expect(removed.workflows.map((workflow) => workflow.name)).toEqual(['default']);
    expect(removed.removalNote).toBe('Threads still on this workflow follow the default afterwards.');
    expect(withoutWorkflow(removed, 'default')).toBe(removed);
  });

  it('points the default at a remaining workflow when the chosen one is removed', () => {
    const removed = withoutWorkflow(draftOf(), 'default');
    expect(removed.defaultKey).toBe('small');
    expect(toConfiguration(removed).default_workflow).toBe('small');
  });
});
