import {
  agentLabel,
  barTitle,
  connectionText,
  composerAction,
  type ComposerPress,
  durationText,
  entryLine,
  groupThreads,
  isBareLetter,
  type KeyPress,
  NO_PLACE,
  NOTHING_SELECTED,
  otherAgent,
  placeIn,
  refusalText,
  resolveSelection,
  runningText,
  screenAfter,
  seenNow,
  summaryText,
  threadLabel,
  waitingText,
  waitingThreads,
  type EntryKind,
  type Project,
  type Thread,
  type ThreadEntry,
} from './workspace.model';

function project(id: string, created_at: string): Project {
  return { id, name: `Project ${id}`, directory: `/work/${id}`, created_at };
}

function thread(id: string, project_id: string, created_at: string, status: Thread['status'] = 'idle'): Thread {
  return { id, project_id, title: '', agent: 'claude', status, unread: false, commands: [], branch: null, created_at };
}

function press(overrides: Partial<KeyPress> = {}): KeyPress {
  return { key: 'c', ctrlKey: false, metaKey: false, altKey: false, element: 'BODY', editable: false, ...overrides };
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
  it('uses the reason the service gave', () => {
    expect(refusalText({ name: 'ServiceRefusal', message: 'A project needs a name.' }, 'Fallback')).toBe('A project needs a name.');
  });

  it.each([
    [null],
    [undefined],
    ['text'],
    [{}],
    [new Error('a programming mistake')],
    [{ name: 'ServiceRefusal', message: '  ' }],
    [{ name: 'ServiceRefusal', message: 5 }],
    [{ name: 'TypeError', message: 'A project needs a name.' }],
  ])(
    'falls back for %j',
    (failure) => {
      expect(refusalText(failure, 'Fallback')).toBe('Fallback');
    },
  );
});

describe('composerAction', () => {
  function composerPress(overrides: Partial<ComposerPress> = {}): ComposerPress {
    return { key: 'Enter', shiftKey: false, isComposing: false, ...overrides };
  }

  it('sends on Enter', () => {
    expect(composerAction(composerPress())).toBe('send');
  });

  it('starts a new line on Shift+Enter, which the box does itself', () => {
    expect(composerAction(composerPress({ shiftKey: true }))).toBeNull();
  });

  it('leaves the box on Escape, with or without Shift', () => {
    expect(composerAction(composerPress({ key: 'Escape' }))).toBe('leave');
    expect(composerAction(composerPress({ key: 'Escape', shiftKey: true }))).toBe('leave');
  });

  it.each(['Enter', 'Escape'])('leaves %s to the input method while a character is composed', (key) => {
    expect(composerAction(composerPress({ key, isComposing: true }))).toBeNull();
  });

  it.each(['h', 'c', 'n', '/', 'Tab', 'ArrowUp'])('types %s as usual', (key) => {
    expect(composerAction(composerPress({ key }))).toBeNull();
  });
});

describe('isBareLetter', () => {
  it('is the letter on its own, in either case', () => {
    expect(isBareLetter(press(), 'c')).toBe(true);
    expect(isBareLetter(press({ key: 'C' }), 'c')).toBe(true);
  });

  it('is no other key', () => {
    expect(isBareLetter(press({ key: 'x' }), 'c')).toBe(false);
    expect(isBareLetter(press({ key: 'Control' }), 'c')).toBe(false);
  });

  it.each(['ctrlKey', 'metaKey', 'altKey'] as const)('is not the letter held with %s, which belongs to the browser', (modifier) => {
    expect(isBareLetter(press({ [modifier]: true }), 'c')).toBe(false);
  });

  it.each(['INPUT', 'TEXTAREA', 'SELECT', 'textarea'])('is not the letter typed into a %s', (element) => {
    expect(isBareLetter(press({ element }), 'c')).toBe(false);
  });

  it('is not the letter typed into text edited in place', () => {
    expect(isBareLetter(press({ element: 'DIV', editable: true }), 'c')).toBe(false);
  });

  it('is the letter pressed on a button', () => {
    expect(isBareLetter(press({ element: 'BUTTON' }), 'c')).toBe(true);
  });
});

describe('barTitle', () => {
  const selected = project('a', '2026-11-01T10:00:00');

  it('is empty without a project', () => {
    expect(barTitle(null, null)).toBe('');
  });

  it('names the project alone when no thread is open', () => {
    expect(barTitle(selected, null)).toBe('Project a');
  });

  it('names the open thread after its project', () => {
    expect(barTitle(selected, { ...thread('t', 'a', '2026-11-03T10:00:00'), title: 'Fix the parser' })).toBe('Project a · Fix the parser');
    expect(barTitle(selected, thread('t', 'a', '2026-11-03T10:00:00'))).toBe('Project a · New thread');
  });
});

describe('runningText', () => {
  const projects = [project('a', '2026-11-01T10:00:00'), project('b', '2026-11-02T10:00:00')];

  it.each([
    [[], ''],
    [['idle', 'idle'], ''],
    [['running', 'idle'], '1 running'],
    [['running', 'running'], '2 running'],
  ] as const)('for threads that are %j says %j', (statuses, text) => {
    const threads = statuses.map((status, index) => thread(`t${index}`, index ? 'b' : 'a', '2026-11-03T10:00:00', status));
    expect(runningText(groupThreads(projects, threads))).toBe(text);
  });

  it('is empty without projects', () => {
    expect(runningText([])).toBe('');
  });
});

describe('waitingThreads and waitingText', () => {
  const projects = [project('a', '2026-11-01T10:00:00'), project('b', '2026-11-02T10:00:00')];

  function unread(id: string, created_at: string, title: string, status: Thread['status'] = 'idle'): Thread {
    return { ...thread(id, id < 't3' ? 'a' : 'b', created_at, status), title, unread: true };
  }

  const one = unread('t1', '2026-11-03T10:00:00', 'One');
  const two = unread('t2', '2026-11-04T10:00:00', '');
  const three = unread('t3', '2026-11-05T10:00:00', 'Three');
  const four = unread('t4', '2026-11-06T10:00:00', 'Four');

  it('finds nothing without projects, or when every thread has been looked at', () => {
    expect(waitingThreads([])).toEqual([]);
    expect(waitingText([])).toBe('');
    const read = groupThreads(projects, [thread('t1', 'a', '2026-11-03T10:00:00'), thread('t2', 'b', '2026-11-04T10:00:00', 'running')]);
    expect(waitingThreads(read)).toEqual([]);
    expect(waitingText(read)).toBe('');
  });

  it('finds the unread threads of every project, newest first', () => {
    const groups = groupThreads(projects, [one, thread('t0', 'a', '2026-11-07T10:00:00'), three, two]);
    expect(waitingThreads(groups).map((item) => item.id)).toEqual(['t3', 't2', 't1']);
  });

  it('leaves out a thread an agent is working in again', () => {
    const groups = groupThreads(projects, [one, unread('t2', '2026-11-04T10:00:00', 'Two', 'running')]);
    expect(waitingThreads(groups).map((item) => item.id)).toEqual(['t1']);
    expect(waitingText(groups)).toBe('waiting: One');
  });

  it.each([
    [[one], 'waiting: One'],
    [[one, two], 'waiting: New thread, One'],
    [[one, two, three], 'waiting: Three, New thread, One'],
    [[one, two, three, four], '4 waiting'],
  ])('names up to three and counts more', (threads, text) => {
    expect(waitingText(groupThreads(projects, threads))).toBe(text);
  });
});

describe('seenNow', () => {
  const unread: Thread = { ...thread('t', 'a', '2026-11-03T10:00:00'), unread: true };

  it('is a thread marked as unread, on screen, in a tab that is showing', () => {
    expect(seenNow(unread, 'thread', true)).toBe(true);
  });

  it.each([
    ['no thread', null, 'thread', true],
    ['a thread already looked at', { ...unread, unread: false }, 'thread', true],
    ['a thread an agent works in', { ...unread, status: 'running' }, 'thread', true],
    ['a thread behind the files', unread, 'files', true],
    ['a thread behind the history', unread, 'history', true],
    ['a thread in a tab that is not showing', unread, 'thread', false],
  ] as const)('is not %s', (_, open, screen, visible) => {
    expect(seenNow(open, screen, visible)).toBe(false);
  });
});

describe('screenAfter', () => {
  it.each([
    ['thread', 'files', 'files'],
    ['thread', 'history', 'history'],
    ['files', 'history', 'history'],
    ['history', 'files', 'files'],
    ['files', 'files', 'thread'],
    ['history', 'history', 'thread'],
  ] as const)('from the %s, asking for the %s shows the %s', (current, asked, shown) => {
    expect(screenAfter(current, asked)).toBe(shown);
  });
});

describe('placeIn', () => {
  it('keeps a choice made in the same project', () => {
    const place = { projectId: 'a', branch: 'feature', commit: 'a3f9c21' };
    expect(placeIn(place, 'a')).toEqual(place);
  });

  it('starts over in another project', () => {
    expect(placeIn({ projectId: 'a', branch: 'feature', commit: 'a3f9c21' }, 'b')).toEqual({ projectId: 'b', branch: null, commit: null });
  });

  it('starts from the directory as it is when nothing was chosen', () => {
    expect(placeIn(NO_PLACE, 'a')).toEqual({ projectId: 'a', branch: null, commit: null });
  });
});

describe('connectionText', () => {
  it('says Teryon is being reached', () => {
    expect(connectionText('connecting')).toBe('Connecting to Teryon…');
  });

  it('says how to open Teryon when this window cannot know where it is', () => {
    expect(connectionText('no_address')).toContain('teryon service --print-address');
  });

  it('says nothing once connected', () => {
    expect(connectionText('connected')).toBe('');
  });
});
