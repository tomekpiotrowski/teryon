import {
  AGENTS,
  agentLabel,
  allChoices,
  choiceLabel,
  choicesOf,
  chosen,
  choosing,
  FIRST_NEW_THREADS,
  modelLabel,
  sameChoice,
  switchingAgent,
  threadChoice,
  countText,
  reviewerLabel,
  reviewerOf,
  reviewersFor,
  barTitle,
  CATCH_UP_REQUEST,
  type Change,
  type ChangeState,
  changeBadge,
  changeCard,
  findingsDraft,
  type EntryLine,
  discardWarning,
  CONNECTED,
  CONNECTING,
  connectionText,
  NO_ADDRESS,
  NO_REASON,
  offersRestart,
  RESTARTING,
  unreachable,
  composerAction,
  type ComposerPress,
  durationText,
  entryLine,
  groupThreads,
  isBareLetter,
  type KeyPress,
  NO_PLACE,
  NOTHING_SELECTED,
  nextAgent,
  type Place,
  placeIn,
  codePlace,
  copyPlace,
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
import { gitWordsIn } from './plain-words.testing';

function project(id: string, created_at: string): Project {
  return { id, name: `Project ${id}`, directory: `/work/${id}`, created_at };
}

function thread(id: string, project_id: string, created_at: string, status: Thread['status'] = 'idle'): Thread {
  return { id, project_id, title: '', agent: 'claude', status, unread: false, commands: [], change: null, finished: null, reviewer: null, created_at };
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
    expect(AGENTS.map(agentLabel)).toEqual(['Claude', 'Codex', 'Grok']);
  });

  it('switches to the next agent, going round', () => {
    expect(AGENTS.map(nextAgent)).toEqual(['codex', 'grok', 'claude']);
  });
});

describe('reviewersFor', () => {
  const plain = thread('t', 'a', '2026-11-03T10:00:00');

  it.each([
    ['claude', ['codex', 'grok']],
    ['codex', ['claude', 'grok']],
    ['grok', ['claude', 'codex']],
  ] as const)('offers every agent but %s, in order', (agent, reviewers) => {
    expect(reviewersFor({ ...plain, agent })).toEqual(reviewers);
    expect(reviewerOf({ ...plain, agent })).toBe(reviewers[0]);
  });

  it('puts the agent that reviewed before first', () => {
    const reviewed = { ...plain, reviewer: { agent: 'grok' as const, reviewing: false, verdict: null } };
    expect(reviewersFor(reviewed)).toEqual(['grok', 'codex']);
    expect(reviewerOf(reviewed)).toBe('grok');
  });

  it('never offers the author, even when it is recorded as the reviewer', () => {
    const odd = { ...plain, reviewer: { agent: 'claude' as const, reviewing: false, verdict: null } };
    expect(reviewersFor(odd)).toEqual(['codex', 'grok']);
  });
});

describe('entryLine', () => {
  it('shows a user message', () => {
    expect(entryLine(entry('user_message', { text: 'Fix it' }))).toEqual({ kind: 'user', text: 'Fix it', reviewer: false });
  });

  it('shows an agent message', () => {
    expect(entryLine(entry('agent_message', { text: 'Done' }))).toEqual({ kind: 'agent', text: 'Done', reviewer: false });
  });

  it('shows a tool with what it was used on', () => {
    expect(entryLine(entry('activity', { tool: 'Bash', summary: 'Run the tests' }))).toEqual({
      kind: 'activity',
      text: 'Bash  Run the tests',
      reviewer: false,
    });
  });

  it('shows a tool without a summary by its name alone', () => {
    expect(entryLine(entry('activity', { tool: 'Bash', summary: '' })).text).toBe('Bash');
  });

  it('shows a notice', () => {
    expect(entryLine(entry('notice', { text: 'Stopped by you.' }))).toEqual({ kind: 'notice', text: 'Stopped by you.', reviewer: false });
  });

  it('shows an error', () => {
    expect(entryLine(entry('error', { text: 'Not signed in' }))).toEqual({ kind: 'error', text: 'Not signed in', reviewer: false });
  });

  it('shows the end of a turn', () => {
    expect(entryLine(entry('turn_summary', { duration_ms: 2000, tokens: { read: 86_322, written: 229 } }))).toEqual({
      kind: 'summary',
      text: 'Done in 2s · 86k tokens read, 229 written',
      reviewer: false,
    });
  });

  it('shows the model of a turn, with its full name on hover', () => {
    expect(entryLine(entry('turn_summary', { duration_ms: 2000, model: 'claude-opus-5-5' }))).toEqual({
      kind: 'summary',
      text: 'Done in 2s · Opus 5.5',
      reviewer: false,
      title: 'claude-opus-5-5',
    });
  });

  it('marks what the reviewer wrote, or was asked', () => {
    expect(entryLine(entry('agent_message', { text: 'Findings: 0', reviewer: true }))).toEqual({ kind: 'agent', text: 'Findings: 0', reviewer: true });
    expect(entryLine(entry('user_message', { text: 'Why?', reviewer: true })).reviewer).toBe(true);
    expect(entryLine(entry('agent_message', { text: 'Done', reviewer: 'yes' })).reviewer).toBe(false);
  });

  it('keeps the agent a reviewer line names, and only on a reviewer line', () => {
    expect(entryLine(entry('agent_message', { text: 'Findings: 1', reviewer: true, agent: 'grok' })).agent).toBe('grok');
    expect(entryLine(entry('agent_message', { text: 'Done', agent: 'grok' })).agent).toBeUndefined();
    expect(entryLine(entry('agent_message', { text: 'Done', reviewer: true, agent: 'gemini' })).agent).toBeUndefined();
    expect(entryLine(entry('agent_message', { text: 'Done', reviewer: true })).agent).toBeUndefined();
  });

  it('labels a reviewer line by the agent it names, not by whoever reviews now', () => {
    const reviewedAgain = { ...thread('t', 'a', '2026-11-03T10:00:00'), reviewer: { agent: 'codex' as const, reviewing: false, verdict: null } };
    const earlier = entryLine(entry('agent_message', { text: 'Findings: 1', reviewer: true, agent: 'grok' }));
    const older = entryLine(entry('agent_message', { text: 'Findings: 1', reviewer: true }));
    expect(reviewerLabel(earlier, reviewedAgain)).toBe('Grok');
    expect(reviewerLabel(older, reviewedAgain)).toBe('Codex');
  });

  it('shows where a review starts apart from other notices', () => {
    expect(entryLine(entry('notice', { text: 'Grok reviews the change.', reviewer: true, starts_review: true }))).toEqual({
      kind: 'review',
      text: 'Grok reviews the change.',
      reviewer: true,
    });
  });

  it('shows empty text when a message has none', () => {
    expect(entryLine(entry('agent_message', {})).text).toBe('');
  });
});

describe('countText', () => {
  it.each([
    [0, '0'],
    [999, '999'],
    [1000, '1k'],
    [86_322, '86k'],
    [999_499, '999k'],
    [999_500, '1M'],
    [1_250_000, '1.3M'],
    [12_000_000, '12M'],
  ])('shows %s as %s', (count, text) => {
    expect(countText(count)).toBe(text);
  });
});

describe('summaryText', () => {
  it('shows duration and tokens', () => {
    expect(summaryText({ duration_ms: 72_000, tokens: { read: 86_322, written: 229 } })).toBe('Done in 1m 12s · 86k tokens read, 229 written');
  });

  it('leaves out tokens that were not reported', () => {
    expect(summaryText({ duration_ms: 3000, tokens: null })).toBe('Done in 3s');
  });

  it('leaves out tokens reported only in part', () => {
    expect(summaryText({ duration_ms: 3000, tokens: { read: 5 } })).toBe('Done in 3s');
    expect(summaryText({ duration_ms: 3000, tokens: { read: '5', written: 1 } })).toBe('Done in 3s');
  });

  it('leaves out a duration that was not reported', () => {
    expect(summaryText({ tokens: { read: 0, written: 0 } })).toBe('Done · 0 tokens read, 0 written');
  });

  it('ignores a price, which subscriptions do not charge', () => {
    expect(summaryText({ duration_ms: 3000, cost_usd: 0.5 })).toBe('Done in 3s');
  });

  it('says only that the turn ended when nothing was reported', () => {
    expect(summaryText({})).toBe('Done');
  });

  it('names the model that ran after the duration', () => {
    expect(summaryText({ duration_ms: 72_000, model: 'claude-opus-5-5', tokens: { read: 86_322, written: 229 } })).toBe(
      'Done in 1m 12s · Opus 5.5 · 86k tokens read, 229 written',
    );
    expect(summaryText({ model: 'grok-4.7-build + grok-4.7-build-fast' })).toBe('Done · Grok 4.7 build + Grok 4.7 build fast');
  });

  it('leaves out a model that is missing, empty or not a name', () => {
    expect(summaryText({ duration_ms: 3000, model: null })).toBe('Done in 3s');
    expect(summaryText({ duration_ms: 3000, model: ' ' })).toBe('Done in 3s');
    expect(summaryText({ duration_ms: 3000, model: 5 })).toBe('Done in 3s');
  });
});

describe('modelLabel', () => {
  it.each([
    ['claude-opus-5-5', 'Opus 5.5'],
    ['claude-fable-5-2', 'Fable 5.2'],
    ['opus', 'Opus'],
    ['grok-4.7-build-fast', 'Grok 4.7 build fast'],
    ['gpt-6-astra', 'GPT 6 astra'],
    ['gpt-6.2-sol', 'GPT 6.2 sol'],
    ['claude-haiku-4-5-1', 'Haiku 4.5.1'],
    ['5-5', '5.5'],
    ['', ''],
    ['grok-4.7-build + grok-4.7-build-fast', 'Grok 4.7 build + Grok 4.7 build fast'],
  ])('reads %s as "%s"', (name, label) => {
    expect(modelLabel(name)).toBe(label);
  });
});

describe('choices', () => {
  const models = { claude: ['opus', 'sonnet'], codex: [], grok: ['grok-4.6'] };

  it('labels an agent and its model, or its default', () => {
    expect(choiceLabel({ agent: 'claude', model: 'claude-opus-5-5' })).toBe('Claude · Opus 5.5');
    expect(choiceLabel({ agent: 'codex', model: null })).toBe('Codex · default');
  });

  it('offers an agent\'s default first, then its models', () => {
    expect(choicesOf('claude', models)).toEqual([
      { agent: 'claude', model: null },
      { agent: 'claude', model: 'opus' },
      { agent: 'claude', model: 'sonnet' },
    ]);
    expect(choicesOf('codex', models)).toEqual([{ agent: 'codex', model: null }]);
  });

  it('offers every agent\'s choices in the order agents are offered', () => {
    expect(allChoices(models).map(choiceLabel)).toEqual([
      'Claude · default', 'Claude · Opus', 'Claude · Sonnet', 'Codex · default', 'Grok · default', 'Grok · Grok 4.6',
    ]);
  });

  it('tells choices apart by agent and model', () => {
    expect(sameChoice({ agent: 'claude', model: null }, { agent: 'claude', model: null })).toBe(true);
    expect(sameChoice({ agent: 'claude', model: null }, { agent: 'claude', model: 'opus' })).toBe(false);
    expect(sameChoice({ agent: 'claude', model: null }, { agent: 'codex', model: null })).toBe(false);
  });

  it('starts new threads on Claude\'s default', () => {
    expect(chosen(FIRST_NEW_THREADS)).toEqual({ agent: 'claude', model: null });
  });

  it('remembers the model chosen for each agent, across switching agents', () => {
    const onOpus = choosing(FIRST_NEW_THREADS, { agent: 'claude', model: 'opus' });
    const onCodex = switchingAgent(onOpus);
    expect(chosen(onCodex)).toEqual({ agent: 'codex', model: null });
    const back = switchingAgent(switchingAgent(onCodex));
    expect(chosen(back)).toEqual({ agent: 'claude', model: 'opus' });
    const grokked = choosing(back, { agent: 'grok', model: 'grok-4.6' });
    expect(grokked.models).toEqual({ claude: 'opus', codex: null, grok: 'grok-4.6' });
  });

  it('reads the choice of a thread\'s agent and of its reviewer', () => {
    const plain = thread('t', 'a', '2026-11-03T10:00:00');
    const reviewed = { ...plain, model: 'opus', reviewer: { agent: 'grok' as const, model: 'grok-4.6', reviewing: false, verdict: null } };
    expect(threadChoice(plain, 'author')).toEqual({ agent: 'claude', model: null });
    expect(threadChoice(plain, 'reviewer')).toBeNull();
    expect(threadChoice(reviewed, 'author')).toEqual({ agent: 'claude', model: 'opus' });
    expect(threadChoice(reviewed, 'reviewer')).toEqual({ agent: 'grok', model: 'grok-4.6' });
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
  const chosen: Place = { projectId: 'a', threadId: 't', branch: 'feature', commit: 'a3f9c21' };

  it('keeps a choice made in the same project', () => {
    expect(placeIn(chosen, 'a')).toEqual(chosen);
  });

  it('starts over in another project', () => {
    expect(placeIn(chosen, 'b')).toEqual({ projectId: 'b', threadId: null, branch: null, commit: null });
  });

  it('starts from the directory as it is when nothing was chosen', () => {
    expect(placeIn(NO_PLACE, 'a')).toEqual({ projectId: 'a', threadId: null, branch: null, commit: null });
  });
});

describe('codePlace', () => {
  const chosen: Place = { projectId: 'a', threadId: null, branch: 'feature', commit: 'a3f9c21' };
  const whole: ChangeState = { copy_present: true, base_present: true, files_changed: 2, fit: 'applies', unfinished: false };

  function changed(state: Partial<ChangeState> | null, branch: string | null = 'fix-login'): Thread {
    const change: Change = { name: 'fix-login', path: '/work/a/.teryon/worktrees/fix-login', base: 'main', start: 'abc', branch, state: state && { ...whole, ...state }, review: null };
    return { ...thread('t', 'a', '2026-11-02T10:00:00'), change };
  }

  it('shows the copy of the open thread, by its thread, with its branch for the history', () => {
    expect(codePlace(chosen, 'a', changed({}))).toEqual({ projectId: 'a', threadId: 't', branch: 'fix-login', commit: null });
  });

  it('shows the copy before Teryon has looked at it', () => {
    expect(codePlace(NO_PLACE, 'a', changed(null))).toEqual({ projectId: 'a', threadId: 't', branch: 'fix-login', commit: null });
  });

  it('shows a copy that is on no branch, such as half way through a rebase, by its thread alone', () => {
    expect(codePlace(NO_PLACE, 'a', changed({}, null))).toEqual({ projectId: 'a', threadId: 't', branch: null, commit: null });
  });

  it.each([
    ['no thread is open', null],
    ['the thread never changed anything', thread('t', 'a', '2026-11-02T10:00:00')],
    ['the copy is gone', changed({ copy_present: false })],
  ])('keeps the choice made in the project when %s', (_, open) => {
    expect(codePlace(chosen, 'a', open)).toEqual(chosen);
  });

  it('starts from the directory as it is in another project when the thread is of a different one', () => {
    expect(codePlace(chosen, 'b', changed({}))).toEqual({ projectId: 'b', threadId: null, branch: null, commit: null });
  });
});

describe('copyPlace', () => {
  it('names the copy by its thread and keeps the branch it is on', () => {
    const change: Change = { name: 'fix-login', path: '/work/a/.teryon/worktrees/fix-login', base: 'main', start: 'abc', branch: 'fix-login-2', state: null, review: null };
    expect(copyPlace(thread('t', 'a', '2026-11-02T10:00:00'), change)).toEqual({ projectId: 'a', threadId: 't', branch: 'fix-login-2', commit: null });
  });
});

describe('connectionText', () => {
  it('says Teryon is being reached', () => {
    expect(connectionText(CONNECTING)).toBe('Connecting to Teryon…');
  });

  it('says how to open Teryon when this window cannot know where it is', () => {
    expect(connectionText(NO_ADDRESS)).toContain('teryon service --print-address');
  });

  it('gives the reason the service could not be reached', () => {
    expect(connectionText({ kind: 'unreachable', reason: 'A different version of Teryon is running.', restartable: true })).toBe(
      'A different version of Teryon is running.',
    );
  });

  it('says Teryon is restarting', () => {
    expect(connectionText(RESTARTING)).toBe('Restarting Teryon…');
  });

  it('says nothing once connected', () => {
    expect(connectionText(CONNECTED)).toBe('');
  });

  it('uses no git words', () => {
    for (const state of [CONNECTING, NO_ADDRESS, RESTARTING, unreachable(null)]) expect(gitWordsIn(connectionText(state))).toEqual([]);
  });
});

describe('unreachable', () => {
  it('keeps the reason and whether restarting helps', () => {
    expect(unreachable({ reason: 'Older.', restartable: true })).toEqual({ kind: 'unreachable', reason: 'Older.', restartable: true });
  });

  it('does not offer a restart unless told it helps', () => {
    expect(unreachable({ reason: 'Too slow.' })).toEqual({ kind: 'unreachable', reason: 'Too slow.', restartable: false });
    expect(unreachable({ reason: 'Too slow.', restartable: 'yes' })).toEqual({ kind: 'unreachable', reason: 'Too slow.', restartable: false });
  });

  it('falls back to a plain reason when none meant for the person was given', () => {
    for (const problem of [new Error('invoke failed'), 'text', null, undefined, { reason: '  ' }, { reason: 3 }]) {
      expect(unreachable(problem)).toEqual({ kind: 'unreachable', reason: NO_REASON, restartable: false });
    }
  });
});

describe('offersRestart', () => {
  it('offers a restart only when the service is unreachable and restarting helps', () => {
    expect(offersRestart({ kind: 'unreachable', reason: 'Older.', restartable: true })).toBe(true);
    expect(offersRestart({ kind: 'unreachable', reason: 'Too slow.', restartable: false })).toBe(false);
    for (const state of [CONNECTING, CONNECTED, NO_ADDRESS, RESTARTING]) expect(offersRestart(state)).toBe(false);
  });
});

describe('changeCard', () => {
  const local: Project = { id: 'a', name: 'App', directory: '/work/a', created_at: '2026-11-01T10:00:00' };
  const hosted: Project = { ...local, on_github: true };
  const whole: ChangeState = { copy_present: true, base_present: true, files_changed: 2, fit: 'applies', unfinished: false };

  function change(state: Partial<ChangeState> | null, review: Change['review'] = null): Change {
    return { name: 'fix-login', path: '/work/a/.teryon/worktrees/fix-login', base: 'main', start: 'abc', branch: 'fix-login', state: state && { ...whole, ...state }, review };
  }

  function review(state: 'in_review' | 'accepted' | 'declined'): Change['review'] {
    return { number: 12, url: 'https://github.com/owner/app/pull/12', state };
  }

  function working(overrides: Partial<Thread>): Thread {
    return { ...thread('t', 'a', '2026-11-02T10:00:00'), ...overrides };
  }

  it('is nothing for a thread that never changed anything', () => {
    expect(changeCard(working({}), local)).toBeNull();
  });

  it.each([
    ['applied', { title: 'Applied', labelled: true, note: 'in the project', tone: 'done', next: [] }],
    ['accepted', { title: 'Accepted', labelled: true, note: 'on GitHub, in the project', tone: 'done', next: [] }],
    ['discarded', { title: 'Discarded', labelled: true, note: '', tone: 'quiet', next: [] }],
    ['removed', { title: 'Copy removed', labelled: true, note: '', tone: 'quiet', next: [] }],
  ] as const)('remembers a change that was %s', (finished, expected) => {
    expect(changeCard(working({ finished }), local)).toEqual(expected);
  });

  it('is changing before Teryon has looked', () => {
    expect(changeCard(working({ change: change(null) }), local)).toEqual({ title: 'Changing', labelled: true, note: '', tone: 'working', next: [] });
  });

  it('takes a removed copy calmly, whatever else is wrong', () => {
    const gone = working({ change: change({ copy_present: false, unfinished: true, fit: 'overlaps' }), status: 'running' });
    expect(changeCard(gone, local)).toEqual({ title: 'Copy removed', labelled: true, note: '', tone: 'quiet', next: [] });
  });

  it.each(['in_review', 'accepted'] as const)('is in review once the copy is gone while GitHub has it %s', (state) => {
    const gone = working({ change: change({ copy_present: false }, review(state)) });
    expect(changeCard(gone, hosted)).toEqual({ title: 'In review', labelled: true, note: 'on GitHub', tone: 'waiting', next: ['change:openReview'] });
  });

  it('takes a removed copy calmly once its review was declined', () => {
    const gone = working({ change: change({ copy_present: false }, review('declined')) });
    expect(changeCard(gone, hosted)?.title).toBe('Copy removed');
  });

  it('says when the agent left it unfinished, ahead of anything but a missing copy', () => {
    const left = working({ change: change({ unfinished: true, base_present: false, fit: 'overlaps' }, review('in_review')), status: 'running' });
    expect(changeCard(left, local)).toEqual({ title: 'Unfinished', labelled: true, note: 'the agent left it half way', tone: 'attention', next: [] });
  });

  it.each([
    [2, '2 files changed so far'],
    [1, '1 file changed so far'],
    [0, ''],
  ])('is changing while the agent works, even when ready otherwise: %i files', (files_changed, note) => {
    expect(changeCard(working({ change: change({ files_changed }), status: 'running' }), local)).toEqual({ title: 'Changing', labelled: true, note, tone: 'working', next: [] });
  });

  it('is changing while no file has changed', () => {
    const subject = working({ change: change({ files_changed: 0, fit: 'overlaps', base_present: false }) });
    expect(changeCard(subject, local)).toEqual({ title: 'Changing', labelled: true, note: 'no files changed yet', tone: 'working', next: [] });
  });

  it('says when the version of the project it started from is gone', () => {
    expect(changeCard(working({ change: change({ base_present: false, fit: 'overlaps' }) }), local)).toEqual({
      title: 'Nowhere to apply', labelled: true,
      note: 'the version it started from is gone',
      tone: 'attention',
      next: [],
    });
  });

  it('asks for catching up when it overlaps, even in review', () => {
    expect(changeCard(working({ change: change({ fit: 'overlaps' }, review('in_review')) }), hosted)).toEqual({
      title: 'Needs catching up', labelled: true,
      note: 'overlaps with newer work',
      tone: 'attention',
      next: ['change:catchUp'],
    });
  });

  it.each(['in_review', 'accepted'] as const)('waits on GitHub while %s and not yet tidied up', (state) => {
    expect(changeCard(working({ change: change({}, review(state)) }), hosted)).toEqual({
      title: 'In review', labelled: true,
      note: '2 files changed, on GitHub',
      tone: 'waiting',
      next: ['change:openReview'],
    });
  });

  it('says when the review was declined', () => {
    expect(changeCard(working({ change: change({}, review('declined')) }), hosted)).toEqual({
      title: 'Declined', labelled: true,
      note: '2 files changed, on GitHub',
      tone: 'attention',
      next: ['change:openReview'],
    });
  });

  it.each(['applies', 'unknown'] as const)('is ready to apply when it %s, which its buttons say without a label', (fit) => {
    expect(changeCard(working({ change: change({ fit }) }), local)).toEqual({ title: 'Ready to apply', labelled: false, note: '2 files changed', tone: 'ready', next: ['change:askReview', 'change:apply'] });
  });

  it('counts one file as one', () => {
    expect(changeCard(working({ change: change({ files_changed: 1 }) }), local)?.note).toBe('1 file changed');
  });

  it('offers sending for review next to applying in a project on GitHub', () => {
    expect(changeCard(working({ change: change({}) }), hosted)?.next).toEqual(['change:askReview', 'change:apply', 'change:sendForReview']);
  });

  describe('reviewed by the other agent', () => {
    function reviewed(verdict: { findings: number | null; answered: boolean; cut?: boolean } | null, reviewing = false): Thread {
      return working({ change: change({}), reviewer: { agent: 'grok', reviewing, verdict: verdict && { cut: false, ...verdict } } });
    }

    it('offers a review before applying', () => {
      expect(changeCard(working({ change: change({}) }), local)?.next).toEqual(['change:askReview', 'change:apply']);
    });

    it('is reviewing while the reviewer reads, and changing while the author works', () => {
      expect(changeCard({ ...reviewed(null, true), status: 'running' }, local)).toEqual({
        title: 'Reviewing',
        labelled: true,
        note: 'Grok is reading the change',
        tone: 'waiting',
        next: [],
      });
      expect(changeCard({ ...reviewed(null), status: 'running' }, local)?.title).toBe('Changing');
    });

    it.each([
      [2, '2 findings'],
      [1, '1 finding'],
      [null, 'Findings'],
    ])('counts %s findings and offers passing them to the author', (findings, title) => {
      expect(changeCard(reviewed({ findings, answered: false }), hosted)).toEqual({
        title,
        labelled: true,
        note: 'from Grok',
        tone: 'attention',
        next: ['change:passFindings', 'change:apply', 'change:sendForReview'],
      });
    });

    it('says the review found nothing, and offers applying', () => {
      expect(changeCard(reviewed({ findings: 0, answered: false }), local)).toEqual({
        title: 'Reviewed',
        labelled: true,
        note: '2 files changed, Grok found nothing',
        tone: 'ready',
        next: ['change:apply'],
      });
    });

    it('says the review stopped before finishing, and offers asking again', () => {
      expect(changeCard(reviewed({ findings: null, answered: false, cut: true }), local)).toEqual({
        title: 'Review stopped',
        labelled: true,
        note: 'Grok stopped before finishing',
        tone: 'attention',
        next: ['change:askReview', 'change:apply'],
      });
    });

    it('is ready to review again once the author has worked on it', () => {
      expect(changeCard(reviewed({ findings: 2, answered: true }), local)?.next).toEqual(['change:askReview', 'change:apply']);
    });

    it('puts overlapping newer work ahead of a review', () => {
      expect(changeCard({ ...reviewed({ findings: 2, answered: false }), change: change({ fit: 'overlaps' }) }, local)?.title).toBe('Needs catching up');
    });
  });

  it('never uses a git word', () => {
    const states: Partial<ChangeState>[] = [];
    for (const copy_present of [true, false])
      for (const base_present of [true, false])
        for (const files_changed of [0, 1, 2])
          for (const fit of ['applies', 'overlaps', 'unknown'] as const)
            for (const unfinished of [true, false]) states.push({ copy_present, base_present, files_changed, fit, unfinished });
    const reviews = [null, review('in_review'), review('accepted'), review('declined')];
    const threads = [
      ...states.flatMap((state) =>
        reviews.flatMap((reviewed) => (['idle', 'running'] as const).map((status) => working({ change: change(state, reviewed), status }))),
      ),
      working({ change: change(null) }),
      ...(['applied', 'accepted', 'discarded', 'removed'] as const).map((finished) => working({ finished })),
    ];
    const texts = threads
      .flatMap((subject) => [local, hosted].map((project) => changeCard(subject, project)))
      .flatMap((card) => (card ? [card.title, card.note] : []));
    expect(texts.flatMap((text) => gitWordsIn(text))).toEqual([]);
  });
});

describe('findingsDraft', () => {
  const line = (kind: EntryLine['kind'], text: string, reviewer: boolean): EntryLine => ({ kind, text, reviewer });

  it('asks the author about the reviewer\'s last message, without the count', () => {
    const lines = [
      line('agent', 'old review\n\nFindings: 1', true),
      line('agent', 'the author answers', false),
      line('agent', '1. A bug.\n2. No test.\n\n**Findings: 2**', true),
      line('summary', 'Done', true),
    ];
    expect(findingsDraft(lines)).toBe(
      "A reviewer looked at the change and found:\n\n1. A bug.\n2. No test.\n\nFix what you agree with, and say why for anything you don't.",
    );
  });

  it('takes the review, not an answer to a question asked after it', () => {
    const lines = [
      line('review', 'Grok reviews the change.', true),
      line('agent', '1. A bug.\n\nFindings: 1', true),
      line('user', 'is it real?', true),
      line('agent', 'Yes.', true),
    ];
    expect(findingsDraft(lines)).toContain('found:\n\n1. A bug.\n\nFix');
  });

  it('takes the first message of the latest review when the reviewer gave no count', () => {
    const lines = [
      line('review', 'Grok reviews the change.', true),
      line('agent', 'old', true),
      line('review', 'Grok reviews the change.', true),
      line('agent', 'A bug.', true),
      line('agent', 'Also this.', true),
    ];
    expect(findingsDraft(lines)).toContain('found:\n\nA bug.\n\nFix');
  });

  it('is nothing before the reviewer has written', () => {
    expect(findingsDraft([line('agent', 'Findings: 2', false), line('review', 'Grok reviews the change.', true)])).toBe('');
  });

  it('never uses a git word', () => {
    expect(gitWordsIn(findingsDraft([line('agent', 'x', true)]))).toEqual([]);
  });
});

describe('changeBadge', () => {
  const base = { ...thread('t', 'a', '2026-11-02T10:00:00') };

  it('is the card\'s title in lower case', () => {
    expect(changeBadge({ ...base, finished: 'applied' })).toBe('applied');
    expect(changeBadge({ ...base, status: 'running', change: { name: 'n', path: 'p', base: null, start: 's', branch: 'n', state: null, review: null } })).toBe('changing');
  });

  it('is nothing for a thread with no change, or one whose change was thrown away or removed', () => {
    expect(changeBadge(base)).toBe('');
    expect(changeBadge({ ...base, finished: 'discarded' })).toBe('');
    expect(changeBadge({ ...base, finished: 'removed' })).toBe('');
  });
});

describe('gitWordsIn', () => {
  it('finds git words whole, in any case and form', () => {
    expect(gitWordsIn('Merged into the Branch after two commits, then rebasing')).toEqual(['branch', 'commit', 'merge', 'rebase']);
  });

  it('leaves words that only contain one alone', () => {
    expect(gitWordsIn('Pushover headline commitment')).toEqual([]);
  });
});

describe('discardWarning', () => {
  function change(state: Partial<ChangeState> | null): Change {
    const whole: ChangeState = { copy_present: true, base_present: true, files_changed: 2, fit: 'applies', unfinished: false };
    return { name: 'fix-login', path: '/work/a/.teryon/worktrees/fix-login', base: 'main', start: 'abc', branch: 'fix-login', state: state && { ...whole, ...state }, review: null };
  }

  it('counts the changed files that will be lost', () => {
    expect(discardWarning(change({}))).toBe('All changes in this thread will be lost: 2 changed files. This cannot be undone.');
  });

  it('counts one file as one', () => {
    expect(discardWarning(change({ files_changed: 1 }))).toBe('All changes in this thread will be lost: 1 changed file. This cannot be undone.');
  });

  it('counts nothing when no file changed, the copy is gone, or nobody has looked', () => {
    const plain = 'All changes in this thread will be lost. This cannot be undone.';
    expect(discardWarning(change({ files_changed: 0 }))).toBe(plain);
    expect(discardWarning(change({ copy_present: false }))).toBe(plain);
    expect(discardWarning(change(null))).toBe(plain);
  });

  it('and the request to catch up never use a git word', () => {
    expect([discardWarning(change({})), CATCH_UP_REQUEST].flatMap(gitWordsIn)).toEqual([]);
  });
});
