import {
  agentLabel,
  durationText,
  entryLine,
  groupThreads,
  NOTHING_SELECTED,
  otherAgent,
  refusalText,
  resolveSelection,
  summaryText,
  threadLabel,
  type EntryKind,
  type Project,
  type Thread,
  type ThreadEntry,
} from './workspace.model';

function project(id: string, created_at: string): Project {
  return { id, name: `Project ${id}`, directory: `/work/${id}`, created_at };
}

function thread(id: string, project_id: string, created_at: string, status: Thread['status'] = 'idle'): Thread {
  return { id, project_id, title: '', agent: 'claude', status, created_at };
}

function entry(kind: EntryKind, body: Record<string, unknown>): ThreadEntry {
  return { id: 'e', thread_id: 't', position: 1, kind, body, created_at: '2026-11-14T10:00:00' };
}

describe('groupThreads', () => {
  const older = project('a', '2026-11-01T10:00:00');
  const newer = project('b', '2026-11-02T10:00:00');

  it('lists projects oldest first', () => {
    expect(groupThreads([newer, older], []).map((group) => group.project.id)).toEqual(['a', 'b']);
  });

  it('gives each project its own threads, newest first', () => {
    const threads = [
      thread('t1', 'a', '2026-11-03T10:00:00'),
      thread('t2', 'b', '2026-11-04T10:00:00'),
      thread('t3', 'a', '2026-11-05T10:00:00'),
    ];
    const groups = groupThreads([older, newer], threads);
    expect(groups[0].threads.map((item) => item.id)).toEqual(['t3', 't1']);
    expect(groups[1].threads.map((item) => item.id)).toEqual(['t2']);
  });

  it('marks a project as running when one of its threads is', () => {
    const threads = [thread('t1', 'a', '2026-11-03T10:00:00', 'running'), thread('t2', 'b', '2026-11-04T10:00:00')];
    expect(groupThreads([older, newer], threads).map((group) => group.running)).toEqual([true, false]);
  });

  it('leaves out threads of projects it does not know', () => {
    expect(groupThreads([older], [thread('t1', 'gone', '2026-11-03T10:00:00')])[0].threads).toEqual([]);
  });

  it('returns nothing without projects', () => {
    expect(groupThreads([], [thread('t1', 'a', '2026-11-03T10:00:00')])).toEqual([]);
  });
});

describe('resolveSelection', () => {
  const groups = groupThreads(
    [project('a', '2026-11-01T10:00:00'), project('b', '2026-11-02T10:00:00')],
    [thread('t1', 'b', '2026-11-03T10:00:00')],
  );

  it('shows the first project when nothing is selected', () => {
    expect(resolveSelection(groups, NOTHING_SELECTED)).toEqual({ project: groups[0].project, thread: null });
  });

  it('shows the selected project', () => {
    expect(resolveSelection(groups, { projectId: 'b', threadId: null }).project?.id).toBe('b');
  });

  it('shows the selected thread together with its project', () => {
    const resolved = resolveSelection(groups, { projectId: 'a', threadId: 't1' });
    expect(resolved.thread?.id).toBe('t1');
    expect(resolved.project?.id).toBe('b');
  });

  it('falls back to the selected project when the thread is gone', () => {
    expect(resolveSelection(groups, { projectId: 'b', threadId: 'gone' })).toEqual({
      project: groups[1].project,
      thread: null,
    });
  });

  it('falls back to the first project when the selected one is gone', () => {
    expect(resolveSelection(groups, { projectId: 'gone', threadId: null }).project?.id).toBe('a');
  });

  it('shows nothing when there are no projects', () => {
    expect(resolveSelection([], { projectId: 'a', threadId: 't1' })).toEqual({ project: null, thread: null });
  });
});

describe('labels', () => {
  it('names an untitled thread', () => {
    expect(threadLabel(thread('t', 'a', '2026-11-03T10:00:00'))).toBe('New thread');
  });

  it('uses a thread title when there is one', () => {
    expect(threadLabel({ ...thread('t', 'a', '2026-11-03T10:00:00'), title: 'Fix login' })).toBe('Fix login');
  });

  it('names the agents', () => {
    expect([agentLabel('claude'), agentLabel('grok')]).toEqual(['Claude', 'Grok']);
  });

  it('switches between the agents', () => {
    expect([otherAgent('claude'), otherAgent('grok')]).toEqual(['grok', 'claude']);
  });
});

describe('entryLine', () => {
  it('shows a user message', () => {
    expect(entryLine(entry('user_message', { text: 'Fix it' }))).toEqual({ kind: 'user', text: 'Fix it' });
  });

  it('shows an agent message', () => {
    expect(entryLine(entry('agent_message', { text: 'Done' }))).toEqual({ kind: 'agent', text: 'Done' });
  });

  it('shows a tool with what it was used on', () => {
    expect(entryLine(entry('activity', { tool: 'Bash', summary: 'Run the tests' }))).toEqual({
      kind: 'activity',
      text: 'Bash  Run the tests',
    });
  });

  it('shows a tool without a summary by its name alone', () => {
    expect(entryLine(entry('activity', { tool: 'Bash', summary: '' })).text).toBe('Bash');
  });

  it('shows a notice', () => {
    expect(entryLine(entry('notice', { text: 'Stopped by you.' }))).toEqual({ kind: 'notice', text: 'Stopped by you.' });
  });

  it('shows an error', () => {
    expect(entryLine(entry('error', { text: 'Not signed in' }))).toEqual({ kind: 'error', text: 'Not signed in' });
  });

  it('shows the end of a turn', () => {
    expect(entryLine(entry('turn_summary', { duration_ms: 2000, cost_usd: 0.5 }))).toEqual({
      kind: 'summary',
      text: 'Done in 2s · $0.50',
    });
  });

  it('shows empty text when a message has none', () => {
    expect(entryLine(entry('agent_message', {})).text).toBe('');
  });
});

describe('summaryText', () => {
  it('shows duration and cost', () => {
    expect(summaryText({ duration_ms: 72_000, cost_usd: 0.4938 })).toBe('Done in 1m 12s · $0.49');
  });

  it('leaves out a cost that was not reported', () => {
    expect(summaryText({ duration_ms: 3000, cost_usd: null })).toBe('Done in 3s');
  });

  it('leaves out a duration that was not reported', () => {
    expect(summaryText({ cost_usd: 1 })).toBe('Done · $1.00');
  });

  it('says only that the turn ended when nothing was reported', () => {
    expect(summaryText({})).toBe('Done');
  });
});

describe('durationText', () => {
  it.each([
    [0, '0s'],
    [499, '0s'],
    [500, '1s'],
    [59_000, '59s'],
    [60_000, '1m 0s'],
    [3_599_000, '59m 59s'],
    [3_600_000, '1h 0m'],
    [5_400_000, '1h 30m'],
    [-5, '0s'],
  ])('shows %i milliseconds as %s', (milliseconds, text) => {
    expect(durationText(milliseconds)).toBe(text);
  });
});

describe('refusalText', () => {
  it('uses the reason the server gave', () => {
    expect(refusalText({ error: { error: 'A project needs a name.' } }, 'Fallback')).toBe('A project needs a name.');
  });

  it.each([[null], [undefined], ['text'], [{}], [{ error: 'text' }], [{ error: {} }], [{ error: { error: '  ' } }], [{ error: { error: 5 } }]])(
    'falls back for %j',
    (failure) => {
      expect(refusalText(failure, 'Fallback')).toBe('Fallback');
    },
  );
});
