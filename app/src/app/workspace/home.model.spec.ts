import {
  agentOptions,
  CONTINUE_LIMIT,
  continueMatches,
  HOME_LIMIT,
  homeList,
  homeWord,
  NO_HOME_CHOICE,
  promptOf,
  readHomeChoice,
  runningLines,
  startAgent,
  startProject,
  startWorkflow,
} from './home.model';
import { type Change, type ChangeState, groupThreads, NO_WORKFLOWS, type Project, type Starting, type Thread, type Workflows } from './workspace.model';

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

describe('continueMatches', () => {
  const projects = [project('a'), project('b')];
  const groups = groupThreads(projects, [
    thread({ id: '1', title: 'Fix the parser', created_at: '2026-11-01T10:00:00' }),
    thread({ id: '2', title: 'Add sensor node', change: changed({}, 'parents-bedroom-sensor'), created_at: '2026-11-02T10:00:00' }),
    thread({ id: '3', title: 'Water label', project_id: 'b', created_at: '2026-11-03T10:00:00' }),
  ]);

  it('offers the newest threads for nothing typed', () => {
    expect(continueMatches(groups, '').map((row) => row.thread.id)).toEqual(['3', '2', '1']);
    expect(continueMatches(groups, '   ').map((row) => row.thread.id)).toEqual(['3', '2', '1']);
  });

  it('matches titles, whatever the case', () => {
    expect(continueMatches(groups, 'PARSER').map((row) => row.thread.id)).toEqual(['1']);
  });

  it('matches a change name, and says it', () => {
    const found = continueMatches(groups, 'bedroom');
    expect(found.map((row) => [row.thread.id, row.change])).toEqual([['2', 'parents-bedroom-sensor']]);
  });

  it('names the project of each match', () => {
    expect(continueMatches(groups, 'water')[0].project).toBe('Project b');
  });

  it('finds nothing for a text no thread has', () => {
    expect(continueMatches(groups, 'zzz')).toEqual([]);
  });

  it('still offers a thread that was put away', () => {
    const away = groupThreads(projects, [thread({ id: 'away', title: 'Old chat', put_away: true })]);
    expect(continueMatches(away, 'old').map((row) => row.thread.id)).toEqual(['away']);
  });

  it('offers a handful at most', () => {
    const many = groupThreads(projects, Array.from({ length: 10 }, (_, index) => thread({ id: `m${index}`, created_at: `2026-11-0${index % 9 + 1}T10:00:00` })));
    expect(continueMatches(many, '')).toHaveLength(CONTINUE_LIMIT);
  });
});

describe('promptOf', () => {
  const chosen = thread({ id: 'c' });

  it('starts a thread with plain text', () => {
    expect(promptOf('Fix it', null)).toEqual({ kind: 'new', text: 'Fix it' });
    expect(promptOf('', null)).toEqual({ kind: 'new', text: '' });
  });

  it('searches threads after an @ at the start', () => {
    expect(promptOf('@', null)).toEqual({ kind: 'searching', query: '' });
    expect(promptOf('@sen', null)).toEqual({ kind: 'searching', query: 'sen' });
  });

  it('takes an @ later in the text as plain text', () => {
    expect(promptOf('mail me @ noon', null)).toEqual({ kind: 'new', text: 'mail me @ noon' });
  });

  it('continues a chosen thread, and then an @ is only text', () => {
    expect(promptOf('@home', chosen)).toEqual({ kind: 'continuing', thread: chosen, text: '@home' });
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

describe('agentOptions', () => {
  it('lists the installed agents in Teryon\'s order', () => {
    expect(agentOptions(['grok', 'claude'])).toEqual(['claude', 'grok']);
  });

  it('ignores names Teryon does not know, and lists nothing when none is installed', () => {
    expect(agentOptions(['mistral'])).toEqual([]);
    expect(agentOptions([])).toEqual([]);
  });
});

describe('startAgent', () => {
  const agents = ['claude', 'codex', 'grok'] as const;
  const grok: Starting = { type: 'provider', agent: 'grok', model: 'grok-4.6' };
  const workflows: Workflows = {
    ...NO_WORKFLOWS,
    workflows: [
      { name: 'default', description: '', stages: ['talking', 'planning', 'done'] },
      { name: 'small', description: '', stages: ['talking', 'done'] },
      { name: 'planned', description: '', stages: ['planning', 'done'] },
    ],
  };

  it('shows a picked agent ahead of the stage choice, on any workflow', () => {
    expect(startAgent(agents, 'codex', grok, 'default', workflows)).toBe('codex');
    expect(startAgent(agents, 'codex', grok, 'planned', workflows)).toBe('codex');
  });

  it.each(['default', 'small'])('shows the service choice on %s', (workflow) => {
    expect(startAgent(agents, null, grok, workflow, workflows)).toBe('grok');
  });

  it.each(['planned', 'missing'])('falls back to Claude when the stage choice does not describe %s', (workflow) => {
    expect(startAgent(agents, null, grok, workflow, workflows)).toBe('claude');
  });

  it.each<Starting | null>([
    null,
    { type: 'unavailable', problem: 'All agents are at their limits.' },
    { type: 'provider', agent: 'unknown', model: null },
  ])('falls back to Claude without an installed provider from the service (%j)', (starting) => {
    expect(startAgent(agents, null, starting, 'default', workflows)).toBe('claude');
  });

  it('falls back to the first installed agent when Claude is missing', () => {
    expect(startAgent(['codex', 'gemini'], null, null, 'default', workflows)).toBe('codex');
    expect(startAgent(['codex', 'gemini'], null, grok, 'default', workflows)).toBe('codex');
  });

  it('shows no agent with nothing installed, even with a remembered choice', () => {
    expect(startAgent([], null, grok, 'default', workflows)).toBeNull();
    expect(startAgent([], 'grok', grok, 'default', workflows)).toBeNull();
  });
});

describe('startWorkflow', () => {
  const known = ['default', 'small'];

  it('takes the remembered workflow while it is set up', () => {
    expect(startWorkflow(known, 'default', { projectId: null, workflow: 'small' })).toBe('small');
  });

  it.each([null, 'removed'])('falls back to the default for %j', (workflow) => {
    expect(startWorkflow(known, 'default', { projectId: null, workflow })).toBe('default');
  });

  it('falls back to the default when a workflow is renamed away', () => {
    expect(startWorkflow(['default', 'tiny'], 'default', { projectId: null, workflow: 'small' })).toBe('default');
  });
});

describe('readHomeChoice', () => {
  it('reads what was written', () => {
    expect(readHomeChoice('{"projectId":"p","workflow":"small"}')).toEqual({ projectId: 'p', workflow: 'small' });
  });

  it('ignores an agent an earlier version wrote, which the workspace owns now', () => {
    expect(readHomeChoice('{"projectId":"p","agent":"codex","workflow":"small"}')).toEqual({ projectId: 'p', workflow: 'small' });
  });

  it('keeps nothing from a choice it does not understand', () => {
    expect(readHomeChoice('{"projectId":4,"workflow":null}')).toEqual(NO_HOME_CHOICE);
  });

  it.each([null, '', 'not json', '42', 'null'])('reads %j as no choice', (stored) => {
    expect(readHomeChoice(stored)).toEqual(NO_HOME_CHOICE);
  });
});
