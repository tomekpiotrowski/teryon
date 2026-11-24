import { modelOptionForTest as model } from '../workspace/model.testing';
import type { Event } from './protocol';
import { applyEvent, NO_LISTS, withEntries, type WorkspaceLists } from './workspace-events';
import { NO_WORKFLOWS, type Project, type Starting, type Thread, type ThreadEntry, type Workflows } from '../workspace/workspace.model';

function project(id: string): Project {
  return { id, name: id, directory: `/work/${id}`, created_at: '2026-11-18T10:00:00Z', state: 'ready' };
}

function thread(id: string, projectId: string, title = ''): Thread {
  return {
    id,
    project_id: projectId,
    title,
    agent: 'claude',
    status: 'idle',
    unread: false,
    commands: [],
    change: null,
    finished: null,
    reviewer: null,
    created_at: '2026-11-18T10:00:00Z',
  };
}

function entry(id: string, position: number): ThreadEntry {
  return { id, thread_id: 't', position, kind: 'notice', body: { text: id }, created_at: '2026-11-18T10:00:00Z' };
}

const lists: WorkspaceLists = {
  projects: [project('a'), project('b')],
  threads: [thread('t1', 'a'), thread('t2', 'b')],
  opens: { editor: false, terminal: false },
  models: { claude: ['opus'].map((id) => model(id)), codex: [], grok: [], gemini: [] },
  workflows: NO_WORKFLOWS,
  starting: null,
  installed: [],
};

const STARTING: Starting = { type: 'provider', agent: 'claude', model: 'sonnet', effort: null };

const SMALL_ONLY: Workflows = { ...NO_WORKFLOWS, default: 'small', workflows: [{ name: 'small', description: '', stages: ['talking', 'ready', 'done'] }] };

function apply(event: unknown): WorkspaceLists {
  return applyEvent(lists, event as Event);
}

describe('applyEvent', () => {
  it('replaces everything with a snapshot, including what copies can be opened in', () => {
    const models = { claude: ['sonnet'].map((id) => model(id)), codex: ['gpt-6-astra'].map((id) => model(id)), grok: [], gemini: [] };
    const fresh = apply({ type: 'snapshot', projects: [project('c')], threads: [], opens: { editor: true, terminal: false }, models, workflows: SMALL_ONLY, starting: STARTING, installed: ['claude', 'grok'] });
    expect(fresh).toEqual({ projects: [project('c')], threads: [], opens: { editor: true, terminal: false }, models, workflows: SMALL_ONLY, starting: STARTING, installed: ['claude', 'grok'] });
  });

  it('replaces only the models when they change, as when Grok has listed its own', () => {
    const models = { claude: ['opus'].map((id) => model(id)), codex: [], grok: ['grok-4.6'].map((id) => model(id)), gemini: ['gemini-4-neon-high'].map((id) => model(id)) };
    expect(apply({ type: 'models_changed', models })).toEqual({ ...lists, models });
  });

  it('replaces the workflows and what copies can be opened in when the settings are saved', () => {
    const opens = { editor: true, terminal: false };
    expect(apply({ type: 'settings_changed', opens, workflows: SMALL_ONLY })).toEqual({ ...lists, opens, workflows: SMALL_ONLY });
  });

  it('replaces only who new threads start with when that changes', () => {
    expect(apply({ type: 'starting_changed', starting: STARTING })).toEqual({ ...lists, starting: STARTING });
    expect(apply({ type: 'starting_changed', starting: null })).toEqual(lists);
  });

  it('adds a project at the end', () => {
    expect(apply({ type: 'project_added', project: project('c') }).projects.map((kept) => kept.id)).toEqual(['a', 'b', 'c']);
  });

  it('replaces a project added again in its place', () => {
    const renamed = { ...project('a'), name: 'Renamed' };
    expect(apply({ type: 'project_added', project: renamed }).projects).toEqual([renamed, project('b')]);
  });

  it('removes a project together with its threads', () => {
    expect(apply({ type: 'project_removed', project_id: 'a' })).toEqual({ ...lists, projects: [project('b')], threads: [thread('t2', 'b')] });
  });

  it('ignores the removal of a project it does not know', () => {
    expect(apply({ type: 'project_removed', project_id: 'z' })).toEqual(lists);
  });

  it('replaces a changed thread in its place', () => {
    const titled = thread('t1', 'a', 'Titled');
    expect(apply({ type: 'thread_changed', thread: titled }).threads).toEqual([titled, thread('t2', 'b')]);
  });

  it('adds a thread it does not know yet', () => {
    expect(apply({ type: 'thread_changed', thread: thread('t3', 'a') }).threads.length).toBe(3);
  });

  it('leaves the lists alone for a new entry', () => {
    expect(apply({ type: 'entry_added', entry: entry('e', 1) })).toBe(lists);
  });

  it('starts from nothing', () => {
    expect(NO_LISTS).toEqual({ projects: [], threads: [], opens: { editor: false, terminal: false }, models: { claude: [], codex: [], grok: [], gemini: [] }, workflows: NO_WORKFLOWS, starting: null, installed: [] });
  });
});

describe('withEntries', () => {
  it('adds entries in order of position', () => {
    expect(withEntries([entry('a', 1), entry('c', 3)], [entry('b', 2)]).map((kept) => kept.id)).toEqual(['a', 'b', 'c']);
  });

  it('keeps an entry that arrives twice once', () => {
    expect(withEntries([entry('a', 1), entry('b', 2)], [entry('b', 2)]).map((kept) => kept.id)).toEqual(['a', 'b']);
  });

  it('works from nothing', () => {
    expect(withEntries([], [entry('a', 1)]).map((kept) => kept.id)).toEqual(['a']);
  });
});
