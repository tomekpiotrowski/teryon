import { modelOptionForTest as model } from './model.testing';
import { NO_MODELS } from './workspace.model';
import { BOARD_START, boardCard, boardColumns, cardAt, cursorOn, movedCursor, rememberedFinishes, settledCursor, type BoardColumn, type SeenFinish } from './board.model';
import { NO_WORKFLOWS, type Project, type Stage, type Thread, type Workflows } from './workspace.model';

function project(id: string): Project {
  return { id, name: `Project ${id}`, directory: `/work/${id}`, created_at: '2026-11-01T10:00:00' };
}

function thread(id: string, overrides: Partial<Thread> = {}): Thread {
  return {
    id,
    project_id: 'a',
    title: `Thread ${id}`,
    agent: 'claude',
    status: 'idle',
    unread: false,
    commands: [],
    change: null,
    finished: null,
    reviewer: null,
    created_at: `2026-11-0${id.length}T10:00:00`,
    ...overrides,
  };
}

const WORKFLOWS: Workflows = {
  default: 'default',
  workflows: [
    { name: 'default', description: '', stages: ['talking', 'planning', 'implementing', 'review', 'ready', 'done'] },
    { name: 'small', description: '', stages: ['talking', 'implementing', 'ready', 'done'] },
  ],
  labels: [
    { stage: 'talking', label: 'Talking', chooses: null },
    { stage: 'planning', label: 'Planning', chooses: null },
    { stage: 'implementing', label: 'Implementing', chooses: null },
    { stage: 'review', label: 'In review', chooses: null },
    { stage: 'ready', label: 'Ready to merge', chooses: null },
    { stage: 'done', label: 'Done', chooses: null },
  ],
};

function stages(columns: readonly BoardColumn[]): Stage[] {
  return columns.map((column) => column.stage);
}

describe('boardColumns', () => {
  it('shows every stage of the default workflow except an empty Done', () => {
    expect(stages(boardColumns([], [], WORKFLOWS, 'a'))).toEqual(['talking', 'planning', 'implementing', 'review', 'ready']);
  });

  it('shows only the stages the threads’ workflows use, without an empty Done', () => {
    expect(stages(boardColumns([thread('t', { workflow: 'small' })], [], WORKFLOWS, 'a'))).toEqual(['talking', 'implementing', 'ready']);
  });

  it('joins the stages of mixed workflows, in the fixed order, without an empty Done', () => {
    const threads = [thread('t', { workflow: 'small' }), thread('u', { workflow: 'default' })];
    expect(stages(boardColumns(threads, [], WORKFLOWS, 'a'))).toEqual(['talking', 'planning', 'implementing', 'review', 'ready']);
  });

  it('treats a workflow no longer set up as the default', () => {
    expect(stages(boardColumns([thread('t', { workflow: 'gone' })], [], WORKFLOWS, 'a'))).toContain('planning');
  });

  it('puts each thread in its stage, and threads from before stages in talking', () => {
    const columns = boardColumns([thread('t', { stage: 'review' }), thread('u')], [], WORKFLOWS, 'a');
    expect(columns.find((column) => column.stage === 'review')?.cards.map((card) => card.thread.id)).toEqual(['t']);
    expect(columns.find((column) => column.stage === 'talking')?.cards.map((card) => card.thread.id)).toEqual(['u']);
  });

  it('uses the labels the service gave', () => {
    expect(boardColumns([], [], WORKFLOWS, 'a').map((column) => column.label)).toContain('Ready to merge');
  });

  it('shows only the project’s threads, without its name', () => {
    const columns = boardColumns([thread('t'), thread('u', { project_id: 'b' })], [project('a'), project('b')], WORKFLOWS, 'a');
    expect(columns[0].cards.map((card) => [card.thread.id, card.project])).toEqual([['t', '']]);
  });

  it('shows every project’s threads with their project’s name on the board of every project', () => {
    const columns = boardColumns([thread('t'), thread('uu', { project_id: 'b' })], [project('a'), project('b')], WORKFLOWS, null);
    expect(columns[0].cards.map((card) => [card.thread.id, card.project])).toEqual([
      ['uu', 'Project b'],
      ['t', 'Project a'],
    ]);
  });

  it('puts the thread that moved to its stage last first, counting creation for threads from before that was recorded', () => {
    const columns = boardColumns(
      [thread('t', { stage_since: '2026-11-09T10:00:00' }), thread('ttt'), thread('tt', { stage_since: null })],
      [],
      WORKFLOWS,
      'a',
    );
    expect(columns[0].cards.map((card) => card.thread.id)).toEqual(['t', 'ttt', 'tt']);
  });

  it('shows every unseen finish, newest first, however long ago one started', () => {
    const older = Array.from({ length: 12 }, (_, index) =>
      thread(`n${index}`, { stage: 'done', created_at: '2026-11-08T10:00:00', stage_since: '2026-11-08T11:00:00' }),
    );
    const newest = thread('old', { stage: 'done', created_at: '2026-10-01T10:00:00', stage_since: '2026-11-21T10:00:00Z' });
    const column = boardColumns([...older, newest], [], WORKFLOWS, 'a').find((candidate) => candidate.stage === 'done');
    expect(column?.cards.map((card) => card.thread.id)).toEqual(['old', ...older.map((item) => item.id)]);
  });

  it('leaves a quiet thread that has not been put away in Talking', () => {
    const columns = boardColumns([thread('t', { unread: true, held: true })], [], WORKFLOWS, 'a');
    expect(columns.find((column) => column.stage === 'talking')?.cards.map((card) => card.thread.id)).toEqual(['t']);
  });

  it('leaves out a thread that was put away, and a finish that was already shown', () => {
    const columns = boardColumns(
      [
        thread('away', { put_away: true }),
        thread('seen', { stage: 'done', done_seen: true, stage_since: '2026-11-21T10:00:00Z' }),
        thread('fresh', { stage: 'done', done_seen: false, stage_since: '2026-11-21T11:00:00Z' }),
      ],
      [],
      WORKFLOWS,
      'a',
    );
    expect(columns.find((column) => column.stage === 'talking')?.cards).toEqual([]);
    expect(columns.find((column) => column.stage === 'done')?.cards.map((card) => card.thread.id)).toEqual(['fresh']);
  });

  it('keeps the other empty stages, and the columns of threads that were all put away', () => {
    const away = thread('t', { workflow: 'small', put_away: true });
    expect(stages(boardColumns([away], [], WORKFLOWS, 'a'))).toEqual(['talking', 'implementing', 'ready']);
    expect(stages(boardColumns([], [], WORKFLOWS, 'a'))).not.toContain('done');
    expect(stages(boardColumns([], [], WORKFLOWS, 'a'))).toContain('planning');
  });

  it('works before the service has said which workflows there are', () => {
    expect(stages(boardColumns([thread('t')], [], NO_WORKFLOWS, 'a'))).toHaveLength(5);
  });
});

describe('boardCard', () => {
  it('shows the title, the agent and its model', () => {
    const card = boardCard(thread('t', { model: 'opus' }), '');
    expect([card.title, card.choice, card.live, card.asking]).toEqual(['Thread t', 'Claude · Opus', false, false]);
  });

  it('calls a thread without a title a new thread', () => {
    expect(boardCard(thread('t', { title: '' }), '').title).toBe('New thread');
  });

  it('marks a running thread', () => {
    expect(boardCard(thread('t', { status: 'running' }), '').live).toBe(true);
  });

  it('marks a thread waiting for answers, but not while its agent works', () => {
    expect(boardCard(thread('t', { asking: true }), '').asking).toBe(true);
    expect(boardCard(thread('t', { asking: true, status: 'running' }), '').asking).toBe(false);
  });

  it('shows the reviewer while a review runs', () => {
    const reviewer = { agent: 'grok' as const, model: null, reviewing: true, verdict: null };
    const catalog = { ...NO_MODELS, grok: [model('grok-4.7', 'Grok 4.7', ['medium', 'xhigh'], 'medium')] };
    expect(boardCard(thread('t', { status: 'running', reviewer: { ...reviewer, effort: 'xhigh' } }), '', catalog).choice).toBe('Grok · Grok 4.7 · Extra high');
    expect(boardCard(thread('t', { reviewer }), '', catalog).choice).toBe('Claude');
  });

  it('shows the change’s name and how many files it changed', () => {
    const change = (files: number | undefined) =>
      ({ name: 'fix-login', state: files === undefined ? null : { files_changed: files } }) as unknown as Thread['change'];
    expect(boardCard(thread('t', { change: change(3) }), '').change).toBe('fix-login · 3 files');
    expect(boardCard(thread('t', { change: change(1) }), '').change).toBe('fix-login · 1 file');
    expect(boardCard(thread('t', { change: change(undefined) }), '').change).toBe('fix-login');
    expect(boardCard(thread('t'), '').change).toBe('');
  });

  it('shows what the last review found until the author works on it', () => {
    const reviewer = (findings: number | null, answered = false, cut = false) => ({ agent: 'grok' as const, model: null, reviewing: false, verdict: { findings, answered, cut } });
    expect(boardCard(thread('t', { reviewer: reviewer(2) }), '').findings).toBe('2 findings');
    expect(boardCard(thread('t', { reviewer: reviewer(1) }), '').findings).toBe('1 finding');
    expect(boardCard(thread('t', { reviewer: reviewer(0) }), '').findings).toBe('0 findings');
    expect(boardCard(thread('t', { reviewer: reviewer(2, true) }), '').findings).toBe('');
    expect(boardCard(thread('t', { reviewer: reviewer(2, false, true) }), '').findings).toBe('');
    expect(boardCard(thread('t', { reviewer: reviewer(null) }), '').findings).toBe('');
  });

  it('marks a thread with a plan', () => {
    expect(boardCard(thread('t'), '').planned).toBe(false);
    expect(boardCard(thread('t', { plan: { title: 'P', fingerprint: 'f', too_large: false } }), '').planned).toBe(true);
  });

  it('counts the agents started without the person', () => {
    expect(boardCard(thread('t', { rounds: 0 }), '').rounds).toBe('');
    expect(boardCard(thread('t', { rounds: 1 }), '').rounds).toBe('1 round without you');
    expect(boardCard(thread('t', { rounds: 2 }), '').rounds).toBe('2 rounds without you');
  });
});

describe('the board’s keys', () => {
  // Talking holds two threads, planning none, implementing one; the rest are empty.
  const columns = boardColumns(
    [thread('t1'), thread('t22'), thread('i', { stage: 'implementing' })],
    [],
    WORKFLOWS,
    'a',
  );

  it('moves between columns, keeping the row where it can', () => {
    expect(movedCursor(columns, { column: 0, row: 1 }, 'l')).toEqual({ column: 1, row: 0 });
    expect(movedCursor(columns, { column: 1, row: 0 }, 'l')).toEqual({ column: 2, row: 0 });
    expect(movedCursor(columns, { column: 2, row: 0 }, 'h')).toEqual({ column: 1, row: 0 });
  });

  it('stops at either end of the board', () => {
    expect(movedCursor(columns, BOARD_START, 'h')).toEqual(BOARD_START);
    expect(movedCursor(columns, { column: 4, row: 0 }, 'l')).toEqual({ column: 4, row: 0 });
  });

  it('moves within a column and stops at its ends', () => {
    expect(movedCursor(columns, BOARD_START, 'j')).toEqual({ column: 0, row: 1 });
    expect(movedCursor(columns, { column: 0, row: 1 }, 'j')).toEqual({ column: 0, row: 1 });
    expect(movedCursor(columns, BOARD_START, 'k')).toEqual(BOARD_START);
    expect(movedCursor(columns, { column: 1, row: 0 }, 'j')).toEqual({ column: 1, row: 0 });
  });

  it('stays put on an empty board', () => {
    expect(movedCursor([], { column: 3, row: 2 }, 'l')).toEqual(BOARD_START);
  });

  it('keeps a cursor inside the board after its threads changed', () => {
    expect(settledCursor(columns, { column: 9, row: 9 })).toEqual({ column: 4, row: 0 });
    expect(settledCursor(columns, { column: 0, row: 7 })).toEqual({ column: 0, row: 1 });
  });

  it('gives the thread under the cursor, and nothing in an empty column', () => {
    expect(cardAt(columns, { column: 0, row: 1 })?.id).toBe('t1');
    expect(cardAt(columns, { column: 1, row: 0 })).toBeNull();
    expect(cardAt([], BOARD_START)).toBeNull();
  });

  it('starts on the thread that was open, or at the start', () => {
    expect(cursorOn(columns, 'i')).toEqual({ column: 2, row: 0 });
    expect(cursorOn(columns, 'gone')).toEqual(BOARD_START);
    expect(cursorOn(columns, null)).toEqual(BOARD_START);
  });
});

describe('rememberedFinishes', () => {
  const done = thread('d', { stage: 'done', stage_since: '2026-11-21T10:00:00Z' });
  const now = '2026-11-21T12:00:00.000Z';

  function remembered(threads: Thread[], projectId: string | null, at: string, before: SeenFinish[] = []): SeenFinish[] {
    return rememberedFinishes(before, threads, boardColumns(threads, [project('a'), project('b')], WORKFLOWS, projectId), at);
  }

  it('remembers a thread rendered in Done', () => {
    expect(remembered([done], 'a', now)).toEqual([{ id: 'd', at: now }]);
  });

  it('forgets a thread that leaves Done during the visit', () => {
    const talking = { ...done, stage: 'talking' as const };
    expect(remembered([talking], 'a', '2026-11-21T13:00:00.000Z', [{ id: 'd', at: now }])).toEqual([]);
  });

  it('keeps a finish seen under every project after switching back to one', () => {
    const other = thread('o', { project_id: 'b', stage: 'done', stage_since: '2026-11-21T09:00:00Z' });
    const both = remembered([done, other], null, now);
    const back = remembered([done, other], 'a', '2026-11-21T13:00:00.000Z', both);
    expect(back.find((finish) => finish.id === 'o')?.at).toBe(now);
    expect(back.find((finish) => finish.id === 'd')?.at).toBe('2026-11-21T13:00:00.000Z');
  });

  it('remembers a later finish only once that card has been rendered again', () => {
    const later = { ...done, stage_since: '2026-11-21T14:00:00.000Z' };
    expect(remembered([later], 'b', '2026-11-21T15:00:00.000Z', [{ id: 'd', at: now }])).toEqual([]);
    expect(remembered([later], 'a', '2026-11-21T15:00:00.000Z')).toEqual([{ id: 'd', at: '2026-11-21T15:00:00.000Z' }]);
  });

  it('keeps a finish whose instant is the same moment written two ways', () => {
    const same = { ...done, stage_since: '2026-11-21T10:00:00Z' };
    expect(remembered([same], 'b', '2026-11-21T15:00:00.000Z', [{ id: 'd', at: '2026-11-21T10:00:00.000Z' }])).toEqual([
      { id: 'd', at: '2026-11-21T10:00:00.000Z' },
    ]);
  });
});
