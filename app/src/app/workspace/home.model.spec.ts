import {
  homeChoices,
  HOME_LIMIT,
  homeList,
  homeWord,
  NO_HOME_CHOICE,
  readHomeChoice,
  runningLines,
  startChoice,
  startProject,
} from './home.model';
import { type Change, type ChangeState, groupThreads, NO_MODELS, type Project, type Starting, type Thread } from './workspace.model';

function project(id: string): Project {
  return { id, name: `Project ${id}`, directory: `/work/${id}`, created_at: '2026-11-01T10:00:00' };
}

function thread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't',
    project_id: 'a',
    title: 'A thread',
    agent: 'claude',
    status: 'idle',
    unread: false,
    commands: [],
    change: null,
    finished: null,
    reviewer: null,
    created_at: '2026-11-03T10:00:00',
    ...overrides,
  };
}

function changed(state: Partial<ChangeState> = {}, name = 'fix-it'): Change {
  return {
    name,
    path: `/work/a/.teryon/worktrees/${name}`,
    base: 'main',
    start: 'abc',
    branch: name,
    state: { copy_present: true, base_present: true, files_changed: 2, fit: 'applies', unfinished: false, ...state },
    review: null,
  };
}

describe('homeWord', () => {
  it('lists nothing for a thread nobody needs to look at', () => {
    expect(homeWord(thread())).toBeNull();
  });

  it('never lists a thread at work, whatever else is true', () => {
    expect(homeWord(thread({ status: 'running', unread: true, asking: true, ended: 'failed', stage: 'ready' }))).toBeNull();
  });

  it.each(['failed', 'limited'] as const)('says failed when the last run %s', (ended) => {
    expect(homeWord(thread({ ended }))).toBe('failed');
  });

  it.each(['finished', 'stopped', null] as const)('does not say failed when the last run ended %s', (ended) => {
    expect(homeWord(thread({ ended }))).toBeNull();
  });

  it.each([
    [{ fit: 'overlaps' as const }, 'overlaps newer work'],
    [{ unfinished: true }, 'was left half way'],
    [{ base_present: false }, 'has nowhere to apply'],
  ])('says failed when the change %j %s', (state, _why) => {
    expect(homeWord(thread({ change: changed(state), unread: true }))).toBe('failed');
  });

  it('says apply for a change that is ready to apply', () => {
    expect(homeWord(thread({ change: changed() }))).toBe('apply');
  });

  it('says apply for a thread in the ready stage', () => {
    expect(homeWord(thread({ stage: 'ready' }))).toBe('apply');
  });

  it('says failed before apply when a ready thread failed', () => {
    expect(homeWord(thread({ stage: 'ready', ended: 'failed' }))).toBe('failed');
  });

  it('says reply when the agent asked questions, even after the thread was looked at', () => {
    expect(homeWord(thread({ asking: true, unread: false }))).toBe('reply');
  });

  it.each(['talking', 'planning'] as const)('says reply for an unseen turn in a conversation stage (%s)', (stage) => {
    expect(homeWord(thread({ unread: true, stage }))).toBe('reply');
  });

  it.each(['implementing', 'review'] as const)('says new for unseen results in %s', (stage) => {
    expect(homeWord(thread({ unread: true, stage }))).toBe('new');
  });

  it('treats a thread without a stage as talking', () => {
    expect(homeWord(thread({ unread: true }))).toBe('reply');
  });

  it('leaves once looked at: an unseen result that was opened is not listed', () => {
    expect(homeWord(thread({ unread: false, stage: 'implementing' }))).toBeNull();
  });

  it('says new for a finished thread only while its result is unseen', () => {
    expect(homeWord(thread({ stage: 'done', unread: true }))).toBe('new');
    expect(homeWord(thread({ stage: 'done', unread: false }))).toBeNull();
  });

  it('leaves a finish off once the board has shown it', () => {
    expect(homeWord(thread({ stage: 'done', unread: true, done_seen: true }))).toBeNull();
    expect(homeWord(thread({ stage: 'done', unread: true, done_seen: false }))).toBe('new');
  });

  it('leaves a put-away thread off, even when its last turn was never opened', () => {
    expect(homeWord(thread({ put_away: true, unread: true, asking: true, ended: 'failed' }))).toBeNull();
  });

  it('ignores an old failure on a finished thread', () => {
    expect(homeWord(thread({ stage: 'done', ended: 'failed' }))).toBeNull();
  });

  it('says reply when a review stopped or declined a change that was not looked at', () => {
    const declined = changed();
    expect(homeWord(thread({ change: { ...declined, review: { number: 1, url: 'u', state: 'declined' } }, unread: true }))).toBe('reply');
  });
});

describe('homeList', () => {
  const projects = [project('a'), project('b')];

  function listed(threads: Thread[], limit?: number) {
    return homeList(groupThreads(projects, threads), limit);
  }

  it('is empty without threads, and when nothing needs the person', () => {
    expect(listed([])).toEqual({ rows: [], more: 0, total: 0 });
    expect(listed([thread({ id: 'x' }), thread({ id: 'y', status: 'running', unread: true })]).total).toBe(0);
  });

  it('orders apply, failed, reply, new, each newest first', () => {
    const rows = listed([
      thread({ id: 'new-old', stage: 'implementing', unread: true, created_at: '2026-11-01T10:00:00' }),
      thread({ id: 'new-recent', stage: 'implementing', unread: true, created_at: '2026-11-05T10:00:00' }),
      thread({ id: 'reply', asking: true }),
      thread({ id: 'failed', ended: 'failed' }),
      thread({ id: 'apply', stage: 'ready' }),
    ]).rows.map((row) => [row.thread.id, row.word]);
    expect(rows).toEqual([
      ['apply', 'apply'],
      ['failed', 'failed'],
      ['reply', 'reply'],
      ['new-recent', 'new'],
      ['new-old', 'new'],
    ]);
  });

  it('orders by when a thread moved to its stage, when that is known', () => {
    const rows = listed([
      thread({ id: 'older-move', asking: true, created_at: '2026-11-09T10:00:00', stage_since: '2026-11-02T10:00:00' }),
      thread({ id: 'newer-move', asking: true, created_at: '2026-11-01T10:00:00', stage_since: '2026-11-04T10:00:00' }),
    ]).rows.map((row) => row.thread.id);
    expect(rows).toEqual(['newer-move', 'older-move']);
  });

  it.each([
    [HOME_LIMIT - 1, 0],
    [HOME_LIMIT, 0],
    [HOME_LIMIT + 1, 1],
    [HOME_LIMIT + 5, 5],
  ])('with %i waiting shows seven at most and counts %i more', (count, more) => {
    const threads = Array.from({ length: count }, (_, index) => thread({ id: `t${index}`, asking: true }));
    const list = listed(threads);
    expect(list.rows).toHaveLength(Math.min(count, HOME_LIMIT));
    expect(list.more).toBe(more);
    expect(list.total).toBe(count);
  });

  it('names the project only when the list spans several', () => {
    const one = listed([thread({ id: 'x', asking: true }), thread({ id: 'y', asking: true })]);
    expect(one.rows.map((row) => row.project)).toEqual(['', '']);
    const several = listed([thread({ id: 'x', asking: true }), thread({ id: 'y', asking: true, project_id: 'b' })]);
    expect(several.rows.map((row) => row.project).sort()).toEqual(['Project a', 'Project b']);
  });

  it('does not name projects for threads that are left out', () => {
    const list = listed([thread({ id: 'x', asking: true }), thread({ id: 'y', project_id: 'b' })]);
    expect(list.rows.map((row) => row.project)).toEqual(['']);
  });
});

describe('runningLines', () => {
  const projects = [project('a'), project('b')];

  it('is empty when nothing runs', () => {
    expect(runningLines(groupThreads(projects, [thread()]))).toEqual({ lines: [], more: 0 });
  });

  it('names a running thread, its project and what it is doing', () => {
    const doing = runningLines(groupThreads(projects, [thread({ id: 'r', status: 'running', change: changed() })]));
    expect(doing.lines.map((line) => [line.thread.id, line.project, line.doing])).toEqual([['r', 'Project a', 'changing']]);
  });

  it('says working for a thread without a change', () => {
    expect(runningLines(groupThreads(projects, [thread({ status: 'running' })])).lines[0].doing).toBe('working');
  });

  it('shows three and counts the rest', () => {
    const threads = Array.from({ length: 5 }, (_, index) => thread({ id: `r${index}`, status: 'running' }));
    const lines = runningLines(groupThreads(projects, threads));
    expect(lines.lines).toHaveLength(3);
    expect(lines.more).toBe(2);
  });
});

describe('startProject', () => {
  const projects = [project('a'), project('b'), project('c')];

  it('takes the remembered project while it exists', () => {
    expect(startProject(projects, { ...NO_HOME_CHOICE, projectId: 'c' }, projects[0])?.id).toBe('c');
  });

  it('falls back to the open project, then the first', () => {
    expect(startProject(projects, { ...NO_HOME_CHOICE, projectId: 'gone' }, projects[1])?.id).toBe('b');
    expect(startProject(projects, NO_HOME_CHOICE, null)?.id).toBe('a');
  });

  it('is nothing without projects', () => {
    expect(startProject([], NO_HOME_CHOICE, null)).toBeNull();
  });
});

describe('homeChoices', () => {
  it('lists installed agents in Teryon’s order, with each default before its models', () => {
    const models = { ...NO_MODELS, claude: ['claude-opus-5-5', 'sonnet'], codex: ['gpt-6-astra'] };
    expect(homeChoices(['grok', 'unknown', 'claude'], models)).toEqual([
      { agent: 'claude', model: null },
      { agent: 'claude', model: 'claude-opus-5-5' },
      { agent: 'claude', model: 'sonnet' },
      { agent: 'grok', model: null },
    ]);
  });

  it('lists no choices without a known installed agent', () => {
    expect(homeChoices(['unknown'], NO_MODELS)).toEqual([]);
    expect(homeChoices([], NO_MODELS)).toEqual([]);
  });

  it('adds models when they are discovered', () => {
    expect(homeChoices(['codex'], NO_MODELS)).toEqual([{ agent: 'codex', model: null }]);
    expect(homeChoices(['codex'], { ...NO_MODELS, codex: ['gpt-6-astra'] })).toEqual([
      { agent: 'codex', model: null },
      { agent: 'codex', model: 'gpt-6-astra' },
    ]);
  });
});

describe('startChoice', () => {
  const choices = [
    { agent: 'codex', model: null },
    { agent: 'claude', model: null },
    { agent: 'grok', model: null },
  ] as const;
  const grok: Starting = { type: 'provider', agent: 'grok', model: 'grok-4.6' };
  it.each(['gpt-6-astra', null])('keeps a picked model (%s) even if it is absent from discovery', (model) => {
    const picked = { agent: 'codex' as const, model };
    expect(startChoice(choices, picked, grok)).toEqual(picked);
  });

  it.each(['grok-4.6', null])('shows the default workflow model (%s) even if it is absent from discovery', (model) => {
    expect(startChoice(choices, null, { ...grok, model })).toEqual({ agent: 'grok', model });
  });

  it.each<Starting | null>([
    null,
    { type: 'unavailable', problem: 'All agents are at their limits.' },
    { type: 'provider', agent: 'gemini', model: 'gemini-4-neon-high' },
  ])('falls back to Claude without an installed provider from the service (%j)', (starting) => {
    expect(startChoice(choices, null, starting)).toEqual({ agent: 'claude', model: null });
  });

  it('falls back to the first installed agent when Claude is missing', () => {
    expect(startChoice(choices.slice(0, 1), null, null)).toEqual({ agent: 'codex', model: null });
    expect(startChoice(choices.slice(0, 1), null, grok)).toEqual({ agent: 'codex', model: null });
  });

  it('shows no agent with nothing installed, even with a remembered choice', () => {
    expect(startChoice([], null, grok)).toBeNull();
    expect(startChoice([], { agent: 'grok', model: null }, grok)).toBeNull();
  });
});

describe('readHomeChoice', () => {
  it('reads what was written', () => {
    expect(readHomeChoice('{"projectId":"p","workflow":"small"}')).toEqual({ projectId: 'p' });
  });

  it('ignores an agent an earlier version wrote, which the workspace owns now', () => {
    expect(readHomeChoice('{"projectId":"p","agent":"codex","workflow":"small"}')).toEqual({ projectId: 'p' });
  });

  it('keeps nothing from a choice it does not understand', () => {
    expect(readHomeChoice('{"projectId":4,"workflow":null}')).toEqual(NO_HOME_CHOICE);
  });

  it.each([null, '', 'not json', '42', 'null'])('reads %j as no choice', (stored) => {
    expect(readHomeChoice(stored)).toEqual(NO_HOME_CHOICE);
  });
});
