import { modelOptionForTest as model } from './model.testing';
import {
  cardSubtitle,
  homeAgentState,
  homeChoices,
  HOME_LIMIT,
  homeList,
  homeQueue,
  homeWord,
  NO_HOME_CHOICE,
  primaryCardAction,
  readHomeChoice,
  runningLines,
  splitLatestTurn,
  startProject,
} from './home.model';
import {
  type Change,
  type ChangeState,
  type Choice,
  type EntryLine,
  groupThreads,
  NO_MODELS,
  NO_WORKFLOWS,
  type Project,
  type Starting,
  type Thread,
  type Workflows,
} from './workspace.model';

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

  it('orders every waiting thread oldest first, whatever its word', () => {
    const rows = listed([
      thread({ id: 'apply', stage: 'ready', created_at: '2026-11-05T10:00:00' }),
      thread({ id: 'new-recent', stage: 'implementing', unread: true, created_at: '2026-11-04T10:00:00' }),
      thread({ id: 'failed', ended: 'failed', created_at: '2026-11-03T10:00:00' }),
      thread({ id: 'reply', asking: true, created_at: '2026-11-02T10:00:00' }),
      thread({ id: 'new-old', stage: 'implementing', unread: true, created_at: '2026-11-01T10:00:00' }),
    ]).rows.map((row) => [row.thread.id, row.word]);
    expect(rows).toEqual([
      ['new-old', 'new'],
      ['reply', 'reply'],
      ['failed', 'failed'],
      ['new-recent', 'new'],
      ['apply', 'apply'],
    ]);
  });

  it('keeps the oldest seven and counts the newest as more', () => {
    const threads = Array.from({ length: HOME_LIMIT + 2 }, (_, index) =>
      thread({ id: `t${index}`, asking: true, created_at: `2026-11-${String(20 - index).padStart(2, '0')}T10:00:00` }),
    );
    const list = listed(threads);
    expect(list.rows.map((row) => row.thread.id)).toEqual(['t8', 't7', 't6', 't5', 't4', 't3', 't2']);
    expect(list.more).toBe(2);
  });

  const earlierThreads = Array.from({ length: HOME_LIMIT - 1 }, (_, index) =>
    thread({ id: `earlier-${index}`, asking: true, created_at: '2026-11-02T10:00:00Z' }),
  );

  it.each([
    ['00Z', '00.001Z'],
    ['00.001Z', '00.001001Z'],
    ['00.001001Z', '00.001001001Z'],
  ])('keeps the older thread at the cap when times end in %s and %s', (older, newer) => {
    const list = listed([
      ...earlierThreads,
      thread({
        id: 'newer', stage: 'ready', created_at: '2026-11-04T10:00:00Z', stage_since: `2026-11-03T10:00:${newer}`,
      }),
      thread({ id: 'older', asking: true, created_at: `2026-11-03T10:00:${older}` }),
    ]);

    expect(list.rows.map((row) => row.thread.id)).toEqual([...earlierThreads.map((thread) => thread.id), 'older']);
    expect(list.more).toBe(1);
  });

  it.each(['00Z', '00.000Z'])('keeps the first thread when different words tie for the last slot (%s)', (applyTime) => {
    const list = listed([
      ...earlierThreads,
      thread({
        id: 'reply', asking: true, created_at: '2026-11-02T10:00:00Z', stage_since: '2026-11-03T10:00:00Z',
      }),
      thread({
        id: 'apply', stage: 'ready', created_at: '2026-11-02T10:00:00Z', stage_since: `2026-11-03T10:00:${applyTime}`,
      }),
    ]);

    expect(list.rows.map((row) => row.thread.id)).toEqual([...earlierThreads.map((thread) => thread.id), 'reply']);
    expect(list.more).toBe(1);
  });

  it('orders by when a thread moved to its stage, when that is known', () => {
    const rows = listed([
      thread({ id: 'older-move', asking: true, created_at: '2026-11-09T10:00:00', stage_since: '2026-11-02T10:00:00' }),
      thread({ id: 'newer-move', asking: true, created_at: '2026-11-01T10:00:00', stage_since: '2026-11-04T10:00:00' }),
    ]).rows.map((row) => row.thread.id);
    expect(rows).toEqual(['older-move', 'newer-move']);
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

  it('takes the remembered project, then the one picked from the menu, then the first, then nothing', () => {
    expect(startProject(projects, { ...NO_HOME_CHOICE, projectId: 'c' }, projects[0])?.id).toBe('c');
    expect(startProject(projects, { ...NO_HOME_CHOICE, projectId: 'gone' }, projects[1])?.id).toBe('b');
    expect(startProject(projects, NO_HOME_CHOICE, null)?.id).toBe('a');
    expect(startProject([], NO_HOME_CHOICE, null)).toBeNull();
  });
});

describe('homeChoices', () => {
  it('lists each installed provider’s usual model in Teryon’s order', () => {
    const models = { ...NO_MODELS, claude: ['claude-opus-5-5', 'sonnet'].map((id) => model(id)), codex: ['gpt-6-astra'].map((id) => model(id)) };
    expect(homeChoices(['grok', 'unknown', 'claude'], models)).toEqual([
      { agent: 'claude', model: 'claude-opus-5-5', effort: null },
      { agent: 'grok', model: null, effort: null },
    ]);
  });

  it('lists no choices without a known installed agent', () => {
    expect(homeChoices(['unknown'], NO_MODELS)).toEqual([]);
    expect(homeChoices([], NO_MODELS)).toEqual([]);
  });

  it('adds models when they are discovered', () => {
    expect(homeChoices(['codex'], NO_MODELS)).toEqual([{ agent: 'codex', model: null, effort: null }]);
    expect(homeChoices(['codex'], { ...NO_MODELS, codex: ['gpt-6-astra'].map((id) => model(id)) })).toEqual([
      { agent: 'codex', model: 'gpt-6-astra', effort: null },
    ]);
  });
});

describe('homeAgentState', () => {
  const installed = ['codex', 'claude', 'grok'];
  const ruled: Workflows = {
    ...NO_WORKFLOWS,
    labels: NO_WORKFLOWS.labels.map((label) => (label.stage === 'talking' ? { ...label, chooses: 'Any · medium' } : label)),
  };
  const grok: Starting = { type: 'provider', agent: 'grok', model: 'grok-4.6', effort: 'medium' };
  const gemini: Starting = { type: 'provider', agent: 'gemini', model: 'gemini-pro', effort: null };
  const unavailable: Starting = { type: 'unavailable', problem: 'All agents are at their limits.' };
  const codex: Choice = { agent: 'codex', model: 'gpt-6-astra', effort: 'high' };
  const absent: Choice = { agent: 'gemini', model: null, effort: null };
  const claude: Choice = { agent: 'claude', model: null, effort: null };

  describe('while the first stage has a rule', () => {
    it.each<{ name: string; starting: Starting | null; choice: Choice | null; reason: string | null }>([
      { name: 'an installed pick', starting: grok, choice: { agent: 'grok', model: 'grok-4.6', effort: 'medium' }, reason: null },
      { name: 'a pick of an agent not installed here', starting: gemini, choice: { agent: 'gemini', model: 'gemini-pro', effort: null }, reason: null },
      { name: 'no possible pick', starting: unavailable, choice: null, reason: 'All agents are at their limits.' },
      { name: 'no pick yet', starting: null, choice: null, reason: null },
    ])('shows the service\'s pick and sends nothing without a manual pick, with $name', ({ starting, choice, reason }) => {
      for (const picked of [null, absent]) {
        expect(homeAgentState(installed, NO_MODELS, ruled, picked, starting)).toEqual({
          allowAutomatic: true, automatic: true, choice, unavailableReason: reason, sendChoice: null,
        });
      }
    });

    it.each<Starting | null>([grok, gemini, unavailable, null])('shows and sends a manual pick of an installed agent over %j', (starting) => {
      expect(homeAgentState(installed, NO_MODELS, ruled, codex, starting)).toEqual({
        allowAutomatic: true, automatic: false, choice: codex, unavailableReason: null, sendChoice: codex,
      });
    });

    it('stays automatic with nothing installed, even with a manual pick', () => {
      expect(homeAgentState([], NO_MODELS, ruled, codex, grok)).toMatchObject({ automatic: true, sendChoice: null });
    });

    it('fills the shown pick\'s default model from discovery', () => {
      const models = { ...NO_MODELS, grok: [model('grok-4.7', undefined, [], null, true)] };
      const state = homeAgentState(installed, models, ruled, null, { type: 'provider', agent: 'grok', model: null, effort: null });
      expect(state.choice?.model).toBe('grok-4.7');
      expect(state.sendChoice).toBeNull();
    });
  });

  describe('while the first stage keeps the thread\'s agent', () => {
    it.each<Starting | null>([grok, unavailable, null])('shows and sends a manual pick of an installed agent, ignoring %j', (starting) => {
      expect(homeAgentState(installed, NO_MODELS, NO_WORKFLOWS, codex, starting)).toEqual({
        allowAutomatic: false, automatic: false, choice: codex, unavailableReason: null, sendChoice: codex,
      });
    });

    it.each<Choice | null>([null, absent])('shows and sends Claude without a usable pick (%j)', (picked) => {
      expect(homeAgentState(installed, NO_MODELS, NO_WORKFLOWS, picked, grok)).toEqual({
        allowAutomatic: false, automatic: false, choice: claude, unavailableReason: null, sendChoice: claude,
      });
    });

    it('shows and sends the first installed agent when Claude is missing', () => {
      const first: Choice = { agent: 'codex', model: null, effort: null };
      expect(homeAgentState(['grok', 'codex'], NO_MODELS, NO_WORKFLOWS, null, null)).toMatchObject({ choice: first, sendChoice: first });
    });

    it('shows and sends no agent with nothing installed', () => {
      expect(homeAgentState([], NO_MODELS, NO_WORKFLOWS, codex, grok)).toEqual({
        allowAutomatic: false, automatic: false, choice: null, unavailableReason: null, sendChoice: null,
      });
    });

    it.each(['gpt-6-astra', null])('keeps a picked model (%s) even when discovery lacks it', (pickedModel) => {
      const picked: Choice = { agent: 'codex', model: pickedModel, effort: null };
      expect(homeAgentState(installed, NO_MODELS, NO_WORKFLOWS, picked, null).choice).toEqual(picked);
    });
  });

  it.each<[Workflows, Choice | null, Starting | null]>([
    [ruled, null, grok], [ruled, codex, grok], [ruled, absent, unavailable], [NO_WORKFLOWS, null, null], [NO_WORKFLOWS, codex, grok],
  ])('never shows one mode while sending another (%#)', (workflows, picked, starting) => {
    const state = homeAgentState(installed, NO_MODELS, workflows, picked, starting);
    expect(state.sendChoice).toEqual(state.automatic ? null : state.choice);
    expect(state.automatic && !state.allowAutomatic).toBe(false);
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

function line(kind: EntryLine['kind'], text = ''): EntryLine {
  return { kind, text, reviewer: false };
}

describe('homeQueue', () => {
  it('puts every waiting thread into one Waiting for you queue, oldest first', () => {
    const apply = thread({ id: 'apply', change: changed(), stage_since: '2026-11-01T00:00:00' });
    const failed = thread({ id: 'failed', ended: 'failed', stage_since: '2026-11-04T00:00:00' });
    const answer = thread({ id: 'answer', asking: true, stage: 'talking', unread: true, stage_since: '2026-11-03T00:00:00' });
    const read = thread({ id: 'read', unread: true, stage: 'implementing', stage_since: '2026-11-02T00:00:00' });
    const queue = homeQueue(groupThreads([project('a')], [read, apply, answer, failed]));

    expect(queue.sections.map((section) => section.title)).toEqual(['Waiting for you']);
    expect(queue.sections.map((section) => section.cards.map((card) => card.thread.id))).toEqual([['apply', 'read', 'answer', 'failed']]);
    expect(queue.cards.map((card) => card.word)).toEqual(['apply', 'new', 'reply', 'failed']);
    expect(queue.waiting).toBe(homeList(groupThreads([project('a')], [read, apply, answer, failed])).total);
    expect(queue.cards.map((card) => card.projectName)).toEqual(['', '', '', '']);
  });

  it('names the project when waiting threads span more than one, and always on a thread at work', () => {
    const waiting = thread({ id: 'w', asking: true });
    const other = thread({ id: 'o', asking: true, project_id: 'b' });
    const running = thread({ id: 'r', status: 'running', project_id: 'a' });
    const queue = homeQueue(groupThreads([project('a'), project('b')], [waiting, other, running]));

    expect(queue.sections.map((section) => section.title)).toEqual(['Waiting for you', 'In progress']);
    expect(queue.cards.map((card) => card.projectName)).toEqual(['Project a', 'Project b', 'Project a']);
    expect(queue.cards[2].subtitle).toBe('working');
  });

  it('orders running threads newest first when their times have different precision', () => {
    const whole = thread({ id: 'whole', status: 'running', stage_since: '2026-11-03T10:00:00Z' });
    const later = thread({ id: 'later', status: 'running', stage_since: '2026-11-03T10:00:00.001Z' });
    const queue = homeQueue(groupThreads([project('a')], [whole, later]));

    expect(queue.cards.map((card) => card.thread.id)).toEqual(['later', 'whole']);
  });

  it('caps waiting and running threads and counts the rest', () => {
    const waiting = Array.from({ length: 8 }, (_, index) => thread({ id: `w${index}`, asking: true, created_at: `2026-11-0${index + 1}T00:00:00` }));
    const running = Array.from({ length: 8 }, (_, index) => thread({ id: `r${index}`, status: 'running', created_at: `2026-11-${10 + index}T00:00:00` }));
    const queue = homeQueue(groupThreads([project('a')], [...waiting, ...running]));

    expect(queue.cards.filter((card) => card.section !== 'progress')).toHaveLength(7);
    expect(queue.moreWaiting).toBe(1);
    expect(queue.waiting).toBe(8);
    expect(queue.cards.filter((card) => card.section === 'progress')).toHaveLength(7);
    expect(queue.moreRunning).toBe(1);
    expect(queue.cards.filter((card) => card.section === 'progress').map((card) => card.thread.id)).toEqual(['r7', 'r6', 'r5', 'r4', 'r3', 'r2', 'r1']);
  });

  it('leaves Home empty when nothing waits and nothing runs', () => {
    const queue = homeQueue(groupThreads([project('a')], [thread()]));
    expect(queue.sections).toEqual([]);
    expect(queue.waiting).toBe(0);
  });
});

describe('cardSubtitle', () => {
  const stage = 'Talking';

  it('describes a change that is ready to apply by its copy', () => {
    const item = thread({ change: changed({ files_changed: 2 }, 'fix-login') });
    const card = homeQueue(groupThreads([project('a')], [item])).cards[0];
    expect(card.subtitle).toBe('Ready to apply · 2 files changed · fix-login');
  });

  it('says how a run failed when the thread never made a change', () => {
    expect(cardSubtitle(thread({ ended: 'failed' }), null, null, stage)).toBe('The run failed');
    expect(cardSubtitle(thread({ ended: 'limited' }), null, null, stage)).toBe('Hit a usage limit');
  });

  it('prefers the stored preview, then the plan title, then the stage', () => {
    expect(cardSubtitle(thread({ asking: true, preview: 'Which screen?' }), null, null, stage)).toBe('Which screen?');
    expect(cardSubtitle(thread({ asking: true, plan: { title: 'The plan', fingerprint: 'x', too_large: false } }), null, null, stage)).toBe('The plan');
    expect(cardSubtitle(thread({ asking: true, preview: '  ' }), null, null, stage)).toBe(stage);
  });

  it('says what a running thread is doing', () => {
    const item = thread({ status: 'running', change: changed({ files_changed: 2 }) });
    const card = homeQueue(groupThreads([project('a')], [item])).cards[0];
    expect(card.subtitle).toBe('changing · 2 files changed so far');
  });
});

describe('splitLatestTurn', () => {
  it('keeps the whole conversation when nobody has spoken', () => {
    const lines = [line('agent', 'hello')];
    expect(splitLatestTurn(lines)).toEqual({ earlier: [], latest: lines });
  });

  it('starts the latest turn at the newest message from the person or a review', () => {
    const lines = [line('user', 'first'), line('agent', 'one'), line('user', 'second'), line('agent', 'two')];
    expect(splitLatestTurn(lines).earlier.map((item) => item.text)).toEqual(['first', 'one']);
    expect(splitLatestTurn(lines).latest.map((item) => item.text)).toEqual(['second', 'two']);
    const reviewed = [line('agent', 'before'), line('review', 'Review'), line('agent', 'after')];
    expect(splitLatestTurn(reviewed).latest.map((item) => item.kind)).toEqual(['review', 'agent']);
  });
});

describe('primaryCardAction', () => {
  function card(overrides: Partial<Thread> = {}, word: 'apply' | 'failed' | 'reply' | 'new' | 'running' = 'reply') {
    const item = thread(overrides);
    const queue = homeQueue(groupThreads([project('a')], [word === 'running' ? thread({ ...overrides, status: 'running' }) : item]));
    return queue.cards[0];
  }

  it('applies on ready or overlapping changes and replies on other failures', () => {
    const ready = card({ change: changed() });
    expect(primaryCardAction(ready, [])).toBe('apply');
    expect(primaryCardAction(ready, [{ id: 'change:apply' }])).toBe('apply');
    expect(primaryCardAction(ready, [{ id: 'other' }])).toBe('reply');
    const overlap = card({ change: changed({ fit: 'overlaps' }) });
    expect(primaryCardAction(overlap, [])).toBe('apply');
    expect(primaryCardAction(overlap, [{ id: 'change:apply' }])).toBe('apply');
    expect(primaryCardAction(overlap, [{ id: 'change:catchUp' }])).toBe('reply');
    expect(primaryCardAction(card({ ended: 'failed' }), [])).toBe('reply');
  });

  it('focuses questions, marks read, or opens a thread at work', () => {
    expect(primaryCardAction(card({ asking: true }))).toBe('questions');
    expect(primaryCardAction(card({ unread: true, stage: 'talking' }))).toBe('reply');
    expect(primaryCardAction(card({ unread: true, stage: 'implementing' }))).toBe('markRead');
    expect(primaryCardAction(card({}, 'running'))).toBe('open');
  });
});
