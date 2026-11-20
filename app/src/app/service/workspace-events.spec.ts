import type { Event } from './protocol';
import { applyEvent, NO_LISTS, withEntries, type WorkspaceLists } from './workspace-events';
import type { Project, Thread, ThreadEntry } from '../workspace/workspace.model';

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
  models: { claude: ['opus'], codex: [], grok: [] },
};

function apply(event: unknown): WorkspaceLists {
  return applyEvent(lists, event as Event);
}

describe('applyEvent', () => {
  it('replaces everything with a snapshot, including what copies can be opened in', () => {
    const models = { claude: ['sonnet'], codex: ['gpt-6-astra'], grok: [] };
    const fresh = apply({ type: 'snapshot', projects: [project('c')], threads: [], opens: { editor: true, terminal: false }, models });
    expect(fresh).toEqual({ projects: [project('c')], threads: [], opens: { editor: true, terminal: false }, models });
  });

  it('replaces only the models when they change, as when Grok has listed its own', () => {
    const models = { claude: ['opus'], codex: [], grok: ['grok-4.6'] };
    expect(apply({ type: 'models_changed', models })).toEqual({ ...lists, models });
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
    expect(NO_LISTS).toEqual({ projects: [], threads: [], opens: { editor: false, terminal: false }, models: { claude: [], codex: [], grok: [] } });
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
