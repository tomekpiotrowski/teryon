import { describe, expect, it } from 'vitest';
import {
  appendTask,
  projectById,
  resolveAttention,
  type Project,
  type ThreadEntry,
} from './workspace.model';

const projects: readonly Project[] = [
  { id: 'one', name: 'One', runs: '1 run today', live: true, items: [] },
  { id: 'two', name: 'Two', runs: 'no runs yet', live: false, items: [] },
];

describe('workspace model', () => {
  it('finds the requested project and falls back to the first project', () => {
    expect(projectById(projects, 'two')).toBe(projects[1]);
    expect(projectById(projects, 'missing')).toBe(projects[0]);
  });

  it('rejects an empty project collection', () => {
    expect(() => projectById([], 'missing')).toThrow('The workspace needs at least one project.');
  });

  it('does not create a task for whitespace-only input', () => {
    const entries: readonly ThreadEntry[] = [{ kind: 'message', author: 'agent', text: 'Existing' }];

    expect(appendTask(entries, '  \n ', new Date('2026-09-06T10:30:00'))).toBe(entries);
  });

  it('appends a trimmed user task and live agent activity', () => {
    const entries: readonly ThreadEntry[] = [];

    const result = appendTask(entries, '  Audit retrying webhooks.  ', new Date('2026-09-06T10:30:00'));

    expect(result).toEqual([
      { kind: 'message', author: 'user', text: 'Audit retrying webhooks.', time: '10:30 AM' },
      {
        kind: 'activity',
        lines: ['Spawned 2 agents · planner, worker', 'Reading context'],
      },
      { kind: 'live', text: 'planner · drafting a plan · 3s' },
    ]);
  });

  it('resolves only the selected attention request without mutating the thread', () => {
    const entries: readonly ThreadEntry[] = [
      { kind: 'message', author: 'agent', text: 'Before' },
      {
        kind: 'attention',
        glyph: '!',
        text: 'Approve the destructive action?',
        primary: 'approve',
        secondary: 'reject',
      },
    ];

    const result = resolveAttention(entries, 1, 'Approved by you — agents resumed');

    expect(result).not.toBe(entries);
    expect(result[0]).toBe(entries[0]);
    expect(result[1]).toEqual({ kind: 'activity', lines: ['Approved by you — agents resumed'] });
    expect(entries[1].kind).toBe('attention');
  });
});
