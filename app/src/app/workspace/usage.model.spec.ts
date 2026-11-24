import {
  agentsWithTurns,
  agoText,
  byModel,
  byProject,
  chosenAgent,
  dayBars,
  levelOf,
  limitBlocks,
  limitRow,
  type Limits,
  resetText,
  type UsageLine,
  type UsageReport,
  type Window,
  withLimits,
  withLine,
} from './usage.model';

/** Local noon on Friday 20 November 2026. */
const NOW = new Date(2026, 10, 20, 12, 0);

function later(milliseconds: number): string {
  return new Date(NOW.getTime() + milliseconds).toISOString();
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

function window(overrides: Partial<Window> = {}): Window {
  return { name: 'five_hour', label: '5 hours', minutes: 300, used: 26, resets_at: later(2 * HOUR + 14 * MINUTE), reached: false, ...overrides };
}

function limits(agent: string, overrides: Partial<Limits> = {}): Limits {
  return { agent, plan: null, windows: [window()], taken_at: later(-12 * MINUTE), failed: false, advice: null, ...overrides };
}

function line(overrides: Partial<UsageLine> = {}): UsageLine {
  return {
    at: NOW.toISOString(),
    agent: 'claude',
    model: null,
    project_id: 'p1',
    project_name: 'Teryon',
    thread_id: 't1',
    read: 1000,
    written: 10,
    ...overrides,
  };
}

describe('limitBlocks', () => {
  it('gives one block per agent, in the menu order, whatever order the readings come in', () => {
    expect(limitBlocks([limits('grok'), limits('claude')], NOW).map((block) => block.agent)).toEqual(['claude', 'codex', 'grok', 'gemini']);
  });

  it('names the plan beside the agent when known', () => {
    expect(limitBlocks([limits('codex', { plan: 'Plus' })], NOW)[1].title).toBe('Codex · Plus');
    expect(limitBlocks([limits('codex')], NOW)[1].title).toBe('Codex');
  });

  it('says how old the reading is', () => {
    expect(limitBlocks([limits('claude')], NOW)[0].asOf).toBe('as of 12 minutes ago');
  });

  it('shows every window of a reading, with no note', () => {
    const block = limitBlocks([limits('claude', { windows: [window(), window({ label: 'week', used: 17 })] })], NOW)[0];
    expect(block.rows.map((row) => row.label)).toEqual(['5 hours', 'week']);
    expect(block.note).toBeNull();
  });

  it('says when nothing has been seen yet', () => {
    const block = limitBlocks([], NOW)[0];
    expect(block.rows).toEqual([]);
    expect(block.asOf).toBe('');
    expect(block.note).toBe('No limits seen yet. They appear after the agent’s next turn.');
  });

  it('says Grok’s limits appear after its next turn, like any agent’s', () => {
    expect(limitBlocks([], NOW)[2].note).toBe('No limits seen yet. They appear after the agent’s next turn.');
  });

  it('says Gemini does not report its usage limits', () => {
    expect(limitBlocks([], NOW)[3].note).toBe('Gemini does not report its usage limits.');
  });

  it('shows Grok’s windows once it reports some', () => {
    expect(limitBlocks([limits('grok')], NOW)[2].rows.length).toBe(1);
  });

  it('says when the limits could not be read', () => {
    const block = limitBlocks([limits('codex', { windows: [], failed: true })], NOW)[1];
    expect(block.note).toBe('Codex’s limits could not be read.');
    expect(block.asOf).toBe('');
  });

  it('says what to do when a failed reading has advice', () => {
    const advice = 'Run grok login in a terminal so Teryon can read Grok’s limits.';
    expect(limitBlocks([limits('grok', { windows: [], failed: true, advice })], NOW)[2].note).toBe(
      `Grok’s limits could not be read. ${advice}`,
    );
    expect(limitBlocks([limits('grok', { failed: true, advice })], NOW)[2].note).toBe(
      `Reading Grok’s limits after its last turn failed, so these may be older. ${advice}`,
    );
  });

  it('gives no advice with a reading that did not fail', () => {
    expect(limitBlocks([limits('grok', { advice: 'Stale advice.' })], NOW)[2].note).toBeNull();
  });

  it('keeps the old windows of a failed reading and says they may be older', () => {
    const block = limitBlocks([limits('codex', { failed: true })], NOW)[1];
    expect(block.rows.length).toBe(1);
    expect(block.note).toBe('Reading Codex’s limits after its last turn failed, so these may be older.');
  });
});

describe('limitRow', () => {
  it('rounds the share used to a whole percentage', () => {
    expect(limitRow(window({ used: 26.4 }), NOW)).toEqual({
      label: '5 hours',
      used: 26,
      usedText: '26%',
      level: 'normal',
      resetText: 'resets in 2h 14m',
    });
  });

  it('keeps the share between 0 and 100', () => {
    expect(limitRow(window({ used: 140 }), NOW).used).toBe(100);
    expect(limitRow(window({ used: -3 }), NOW).used).toBe(0);
  });

  it('shows a window whose reset time has passed as empty and open', () => {
    expect(limitRow(window({ used: 100, reached: true, resets_at: later(-MINUTE) }), NOW)).toEqual({
      label: '5 hours',
      used: 0,
      usedText: '0%',
      level: 'normal',
      resetText: 'reset since the last reading',
    });
  });

  it('marks a used-up window as reached', () => {
    const row = limitRow(window({ used: 100, reached: true, resets_at: later(40 * MINUTE) }), NOW);
    expect(row.level).toBe('reached');
    expect(row.resetText).toBe('reached, resets in 40m');
  });
});

describe('levelOf', () => {
  it('is normal below 80%', () => expect(levelOf(79, false)).toBe('normal'));
  it('warns from 80%', () => expect(levelOf(80, false)).toBe('warning'));
  it('warns at 100% while not said to be used up', () => expect(levelOf(100, false)).toBe('warning'));
  it('is reached when said to be used up, whatever the share', () => expect(levelOf(10, true)).toBe('reached'));
});

describe('resetText', () => {
  const at = (milliseconds: number) => new Date(NOW.getTime() + milliseconds);

  it('is empty without a reset time', () => expect(resetText(null, false, NOW)).toBe(''));
  it('says reached without a reset time when used up', () => expect(resetText(null, true, NOW)).toBe('reached'));
  it('says the window has reset once the time has passed', () => expect(resetText(at(0), false, NOW)).toBe('reset since the last reading'));
  it('counts minutes within an hour, rounded up', () => expect(resetText(at(13 * MINUTE + 1), false, NOW)).toBe('resets in 14m'));
  it('says one minute for under a minute', () => expect(resetText(at(5000), false, NOW)).toBe('resets in 1m'));
  it('counts hours and minutes within a day', () => expect(resetText(at(2 * HOUR + 14 * MINUTE), false, NOW)).toBe('resets in 2h 14m'));
  it('leaves out no minutes', () => expect(resetText(at(3 * HOUR), false, NOW)).toBe('resets in 3h'));
  it('gives the weekday and local time from a day on', () =>
    expect(resetText(new Date(2026, 10, 27, 18, 30), false, NOW)).toBe('resets Fri 18:30'));
  it('gives the weekday at exactly a day', () => expect(resetText(at(DAY), false, NOW)).toBe('resets Sat 12:00'));
});

describe('agoText', () => {
  const before = (milliseconds: number) => new Date(NOW.getTime() - milliseconds);

  it('says just now within a minute', () => expect(agoText(before(59_000), NOW)).toBe('just now'));
  it('says just now for a time ahead of the clock', () => expect(agoText(new Date(NOW.getTime() + HOUR), NOW)).toBe('just now'));
  it('counts one minute', () => expect(agoText(before(MINUTE), NOW)).toBe('1 minute ago'));
  it('counts minutes', () => expect(agoText(before(12 * MINUTE), NOW)).toBe('12 minutes ago'));
  it('counts hours', () => expect(agoText(before(3 * HOUR + 59 * MINUTE), NOW)).toBe('3 hours ago'));
  it('counts days', () => expect(agoText(before(2 * DAY), NOW)).toBe('2 days ago'));
});

describe('agentsWithTurns', () => {
  it('is empty without turns', () => expect(agentsWithTurns([])).toEqual([]));
  it('names one agent', () => expect(agentsWithTurns([line(), line()])).toEqual(['claude']));
  it('names several in the menu order', () =>
    expect(agentsWithTurns([line({ agent: 'grok' }), line({ agent: 'claude' })])).toEqual(['claude', 'grok']));
  it('leaves out agents Teryon does not know', () => expect(agentsWithTurns([line({ agent: 'mistral' })])).toEqual([]));
});

describe('chosenAgent', () => {
  it('is nothing without agents', () => expect(chosenAgent([], 'claude')).toBeNull());
  it('is the first agent when none was asked for', () => expect(chosenAgent(['codex', 'grok'], null)).toBe('codex'));
  it('is the one asked for', () => expect(chosenAgent(['codex', 'grok'], 'grok')).toBe('grok'));
  it('is the first when the one asked for has no turns', () => expect(chosenAgent(['codex', 'grok'], 'claude')).toBe('codex'));
});

describe('dayBars', () => {
  const daysAgo = (days: number, hour = 10) => new Date(2026, 10, 20 - days, hour).toISOString();

  it('gives 14 days ending today, oldest first, even without turns', () => {
    const bars = dayBars([], 'claude', NOW);
    expect(bars.length).toBe(14);
    expect(bars[0].date).toBe('2026-11-07');
    expect(bars[13].date).toBe('2026-11-20');
    expect(bars[13].label).toBe('Fri 20');
    expect(bars.every((bar) => bar.height === 0 && bar.detail.endsWith(': nothing'))).toBe(true);
  });

  it('adds up one agent’s turns per local day, against the busiest day', () => {
    const bars = dayBars(
      [
        line({ at: daysAgo(0, 9), written: 10, read: 500 }),
        line({ at: daysAgo(0, 11), written: 30, read: 1500 }),
        line({ at: daysAgo(1), written: 20 }),
        line({ at: daysAgo(1), agent: 'codex', written: 9000 }),
      ],
      'claude',
      NOW,
    );
    expect(bars[13]).toEqual({
      date: '2026-11-20',
      label: 'Fri 20',
      weekday: 'Fri',
      day: 20,
      turns: 2,
      read: 2000,
      written: 40,
      height: 100,
      detail: 'Fri 20: 2 turns, 2k read, 40 written',
    });
    expect(bars[12].height).toBe(50);
    expect(bars[12].detail).toBe('Thu 19: 1 turn, 1k read, 20 written');
  });

  it('leaves out turns older than 14 days', () => {
    expect(dayBars([line({ at: daysAgo(14) })], 'claude', NOW).every((bar) => bar.turns === 0)).toBe(true);
  });

  it('shows nothing for no agent', () => {
    expect(dayBars([line()], null, NOW).every((bar) => bar.turns === 0)).toBe(true);
  });

  it('keeps bars flat when every turn wrote nothing', () => {
    expect(dayBars([line({ written: 0 })], 'claude', NOW)[13].height).toBe(0);
  });
});

describe('byProject', () => {
  it('groups one agent’s turns by project, most written first', () => {
    const rows = byProject(
      [
        line({ project_id: 'p1', project_name: 'Teryon', written: 10 }),
        line({ project_id: 'p2', project_name: 'Site', written: 500, read: 2_100_000 }),
        line({ project_id: 'p1', project_name: 'Teryon', written: 5 }),
        line({ project_id: 'p3', project_name: 'Other', agent: 'grok', written: 9999 }),
      ],
      'claude',
    );
    expect(rows).toEqual([
      { key: 'p2', name: 'Site', turns: 1, read: '2.1M', written: '500' },
      { key: 'p1', name: 'Teryon', turns: 2, read: '2k', written: '15' },
    ]);
  });

  it('names a project after its latest turn', () => {
    const rows = byProject([line({ project_name: 'Old' }), line({ project_name: 'New' })], 'claude');
    expect(rows.map((row) => row.name)).toEqual(['New']);
  });

  it('puts projects with as much written in name order', () => {
    const rows = byProject([line({ project_id: 'b', project_name: 'B' }), line({ project_id: 'a', project_name: 'A' })], 'claude');
    expect(rows.map((row) => row.name)).toEqual(['A', 'B']);
  });

  it('is empty without turns', () => expect(byProject([], 'claude')).toEqual([]));
});

describe('byModel', () => {
  it('groups by model, calling the program’s own default "default"', () => {
    const rows = byModel([line({ model: 'claude-opus-5-5', written: 50 }), line({ model: null }), line({ model: null })], 'claude');
    expect(rows.map((row) => [row.name, row.turns])).toEqual([
      ['Opus 5.5', 1],
      ['default', 2],
    ]);
  });
});

describe('withLimits and withLine', () => {
  const report: UsageReport = { limits: [limits('claude'), limits('codex')], lines: [line()] };

  it('replaces an agent’s limits', () => {
    const changed = withLimits(report, limits('claude', { plan: 'Max' }));
    expect(changed.limits.map((reading) => [reading.agent, reading.plan])).toEqual([
      ['codex', null],
      ['claude', 'Max'],
    ]);
  });

  it('adds a new agent’s limits', () => expect(withLimits(report, limits('grok')).limits.length).toBe(3));

  it('adds a turn at the end', () => expect(withLine(report, line({ thread_id: 't2' })).lines.map((added) => added.thread_id)).toEqual(['t1', 't2']));
});
