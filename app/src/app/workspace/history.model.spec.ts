import type { FileLine } from './files.model';
import {
  aheadText,
  type ChangedFile,
  type ChangeRow,
  changeRows,
  chosenCommits,
  type Commit,
  commitCountText,
  countsText,
  type DiffRow,
  diffRows,
  type GraphRow,
  graphRows,
  looseRows,
  matchesFilter,
  momentText,
  moveStep,
  neighbour,
  revertDraft,
  selectedChange,
  shortHash,
  type SplitRow,
  splitRows,
  timeText,
  type Who,
  whoMade,
} from './history.model';
import type { KeyPress, Thread } from './workspace.model';

function commit(hash: string, overrides: Partial<Commit> = {}): Commit {
  return {
    hash,
    parents: [],
    subject: `Commit ${hash}`,
    author: 'Ada Lovelace',
    email: 'ada@example.com',
    time: 0,
    on_base: false,
    thread: null,
    files: [],
    ...overrides,
  };
}

function thread(id: string, title = 'Fix the flaky test', agent: Thread['agent'] = 'claude'): Thread {
  return { id, project_id: 'p', title, agent, status: 'idle', unread: false, commands: [], branch: null, created_at: '2026-11-14T10:00:00' };
}

const person: Who = { kind: 'person', name: 'Ada Lovelace', thread: null };
const agent: Who = { kind: 'agent', name: 'Claude', thread: thread('t1') };

function press(overrides: Partial<KeyPress> = {}): KeyPress {
  return { key: 'j', ctrlKey: false, metaKey: false, altKey: false, element: 'BODY', editable: false, ...overrides };
}

/** A moment of this machine's own day, in seconds since 1970, so the specs hold in every time zone. */
function at(year: number, month: number, day: number, hour = 12, minute = 0): number {
  return new Date(year, month - 1, day, hour, minute).getTime() / 1000;
}

/**
 * The picture as text, a row a line: the left lane's line, the right lane's line, then the lane the dot is on.
 *
 * A line is `│` through the row, `╷` from the dot down and `╵` down to the dot; `m` and `f` mark a merge and a fork.
 */
function drawn(rows: readonly GraphRow[]): string[] {
  const line = { none: ' ', below: '╷', through: '│', above: '╵' };
  const joint = { none: '', merge: ' m', fork: ' f' };
  return rows.map((row) => `${line[row.own]}${line[row.base]}|${row.lane}${joint[row.joint]}`);
}

function file(path: string, change: ChangedFile['change'] = 'modified', added: number | null = 1, removed: number | null = 1): ChangedFile {
  return { path, change, added, removed };
}

/** The rows of changed files as short text: `▾ src/ledger`, `  adapter.ts M +1 −1`. */
function listed(rows: readonly ChangeRow[]): string[] {
  return rows.map((row) => {
    if (row.kind === 'folder') return `${row.open ? '▾' : '▸'} ${row.path}`;
    return `${row.nested ? '  ' : ''}${row.name} ${row.change[0].toUpperCase()} ${row.counts}`.trimEnd();
  });
}

/** Lines written as `  text` unchanged, `+ text` added and `- text` removed, numbered as the server would. */
function lines(...written: string[]): FileLine[] {
  let number = 0;
  return written.map((text) => {
    const change = text.startsWith('+') ? 'added' : text.startsWith('-') ? 'removed' : null;
    if (change !== 'removed') number += 1;
    return { change, number: change === 'removed' ? null : number, text: text.slice(2) };
  });
}

/** Rows as text: the hunk's own text, or `before|after mark text`. */
function unified(rows: readonly DiffRow<FileLine>[]): string[] {
  return rows.map((row) => {
    if (row.kind === 'hunk') return row.text;
    const mark = row.line.change === 'added' ? '+' : row.line.change === 'removed' ? '-' : ' ';
    return `${row.before ?? ''}|${row.after ?? ''} ${mark} ${row.line.text}`;
  });
}

/** Rows as text: the hunk's own text, or `number text ‖ number text`, with `·` for an empty side. */
function sideBySide(rows: readonly SplitRow<FileLine>[]): string[] {
  return rows.map((row) => {
    if (row.kind === 'hunk') return row.text;
    const side = (half: typeof row.before): string => (half ? `${half.number} ${half.line.text}` : '·');
    return `${side(row.before)} ‖ ${side(row.after)}`;
  });
}

describe('shortHash', () => {
  it('keeps the first seven digits', () => {
    expect(shortHash('08fb0ad08ebe07fb6fa5f3f750649b2be9b21e31')).toBe('08fb0ad');
  });

  it('leaves a shorter name as it is', () => {
    expect(shortHash('08fb')).toBe('08fb');
    expect(shortHash('')).toBe('');
  });
});

describe('whoMade', () => {
  it('names the agent of the thread a commit came from', () => {
    const claude = thread('t1', 'Fix it', 'claude');
    const grok = thread('t2', 'Break it', 'grok');
    expect(whoMade(commit('a', { thread: 't1' }), [claude, grok], null)).toEqual({ kind: 'agent', name: 'Claude', thread: claude });
    expect(whoMade(commit('a', { thread: 't2' }), [claude, grok], null)).toEqual({ kind: 'agent', name: 'Grok', thread: grok });
  });

  it('still counts a commit as an agent’s when its thread is not known here', () => {
    expect(whoMade(commit('a', { thread: 'gone' }), [thread('t1')], null)).toEqual({ kind: 'agent', name: 'Agent', thread: null });
  });

  it('counts a commit as an agent’s even under the address of whoever is signed in', () => {
    expect(whoMade(commit('a', { thread: 't1' }), [thread('t1')], 'ada@example.com').kind).toBe('agent');
  });

  it('names a person by the name the commit is recorded under', () => {
    expect(whoMade(commit('a'), [], null)).toEqual({ kind: 'person', name: 'Ada Lovelace', thread: null });
    expect(whoMade(commit('a'), [], 'someone@example.com').name).toBe('Ada Lovelace');
  });

  it.each(['ada@example.com', 'ADA@Example.com', '  ada@example.com '])('calls a commit under the address %j yours', (ownEmail) => {
    expect(whoMade(commit('a'), [], ownEmail).name).toBe('you');
  });

  it('does not call a commit without an address yours', () => {
    expect(whoMade(commit('a', { email: '' }), [], '').name).toBe('Ada Lovelace');
  });

  it('falls back to the address, then to a word, for a commit without a name', () => {
    expect(whoMade(commit('a', { author: '' }), [], null).name).toBe('ada@example.com');
    expect(whoMade(commit('a', { author: '', email: '' }), [], null).name).toBe('unknown');
  });
});

describe('matchesFilter', () => {
  const made = commit('a3f9c21e', {
    subject: 'Add idempotency key to ledger writes',
    files: ['src/ledger/adapter.ts', 'src/ledger/entry.ts'],
  });

  it('keeps every commit without a filter', () => {
    expect(matchesFilter(made, person, '', 'all')).toBe(true);
    expect(matchesFilter(made, person, '   ', 'all')).toBe(true);
  });

  it.each([
    ['the first line of the message', 'idempotency'],
    ['the message in other letter case', 'LEDGER WRITES'],
    ['the start of the commit’s name', 'a3f9'],
    ['the start of the name in capitals', 'A3F9'],
    ['the recorded name', 'lovelace'],
    ['a changed file', 'adapter.ts'],
    ['a folder of a changed file', 'src/ledger'],
    ['text with spaces around it', '  entry.ts  '],
  ])('keeps a commit by %s', (_, filter) => {
    expect(matchesFilter(made, person, filter, 'all')).toBe(true);
  });

  it.each([
    ['text found nowhere', 'migration'],
    ['digits from the middle of the commit’s name', 'f9c2'],
    ['a word that only the name’s digits spell', 'c21e'],
  ])('drops a commit for %s', (_, filter) => {
    expect(matchesFilter(made, person, filter, 'all')).toBe(false);
  });

  it('keeps an agent’s commit by the agent and by the title of its thread', () => {
    expect(matchesFilter(made, agent, 'claude', 'all')).toBe(true);
    expect(matchesFilter(made, agent, 'flaky', 'all')).toBe(true);
    expect(matchesFilter(made, person, 'claude', 'all')).toBe(false);
  });

  it('keeps a person’s commit by `you`', () => {
    expect(matchesFilter(made, { kind: 'person', name: 'you', thread: null }, 'you', 'all')).toBe(true);
  });

  it.each([
    ['all', true, true],
    ['agents', true, false],
    ['people', false, true],
  ] as const)('with %s chosen keeps an agent’s commit: %s, and a person’s: %s', (maker, keepsAgent, keepsPerson) => {
    expect(matchesFilter(made, agent, '', maker)).toBe(keepsAgent);
    expect(matchesFilter(made, person, '', maker)).toBe(keepsPerson);
  });

  it('needs both the text and whose commits to fit', () => {
    expect(matchesFilter(made, agent, 'idempotency', 'people')).toBe(false);
    expect(matchesFilter(made, agent, 'migration', 'agents')).toBe(false);
    expect(matchesFilter(made, agent, 'idempotency', 'agents')).toBe(true);
  });
});

describe('graphRows', () => {
  it('draws nothing for no commits', () => {
    expect(graphRows([], false)).toEqual([]);
  });

  it('draws one lane for a branch that is not told apart from a main branch', () => {
    const commits = [commit('c', { parents: ['b'] }), commit('b', { parents: ['a'] }), commit('a')];
    expect(drawn(graphRows(commits, false))).toEqual(['╷ |own', '│ |own', '╵ |own']);
  });

  it('draws a dot alone for a single commit', () => {
    expect(drawn(graphRows([commit('a')], false))).toEqual(['  |own']);
  });

  it('runs the lane off the bottom when older commits follow', () => {
    const commits = [commit('b', { parents: ['a'] }), commit('a', { parents: ['z'] })];
    expect(drawn(graphRows(commits, true))).toEqual(['╷ |own', '│ |own']);
    expect(drawn(graphRows([commit('a', { parents: ['z'] })], true))).toEqual(['╷ |own']);
  });

  it('joins a branch to the main branch where it left it', () => {
    const commits = [
      commit('d', { parents: ['c'] }),
      commit('c', { parents: ['b'] }),
      commit('b', { parents: ['a'], on_base: true }),
      commit('a', { on_base: true }),
    ];
    expect(drawn(graphRows(commits, false))).toEqual(['╷ |own', '│ |own', '╵╷|base f', ' ╵|base']);
  });

  it('follows the design: a merge of the main branch, then the place the branch left it', () => {
    const commits = [
      commit('a3f9c21', { parents: ['7be0d14'] }),
      commit('7be0d14', { parents: ['9d20e5f'] }),
      commit('9d20e5f', { parents: ['4410bb2'] }),
      commit('4410bb2', { parents: ['51c8e2a', 'e19a6b3'] }),
      commit('51c8e2a', { parents: ['c03d771'] }),
      commit('e19a6b3', { parents: ['c03d771'], on_base: true }),
      commit('c03d771', { parents: ['older'], on_base: true }),
    ];
    expect(drawn(graphRows(commits, true))).toEqual(['╷ |own', '│ |own', '│ |own', '│╷|own m', '││|own', '││|base', '╵│|base f']);
  });

  it('ends the main branch’s lane at the last commit when no older ones follow', () => {
    const commits = [commit('b', { parents: ['a'] }), commit('a', { on_base: true })];
    expect(drawn(graphRows(commits, false))).toEqual(['╷ |own', '╵ |base f']);
  });

  it('draws only the right lane for a branch with no commits of its own', () => {
    const commits = [commit('b', { parents: ['a'], on_base: true }), commit('a', { on_base: true })];
    expect(drawn(graphRows(commits, false))).toEqual([' ╷|base', ' ╵|base']);
  });

  it('runs the left lane off the bottom when the place the branch left the main branch is not listed', () => {
    const commits = [commit('c', { parents: ['b'] }), commit('b', { parents: ['unlisted'] })];
    expect(drawn(graphRows(commits, true))).toEqual(['╷ |own', '│ |own']);
  });

  it('ends the left lane at a branch’s first commit when it never left the main branch', () => {
    const commits = [
      commit('c', { parents: ['b', 'm'] }),
      commit('b', { parents: ['a'] }),
      commit('a'),
      commit('m', { on_base: true }),
    ];
    expect(drawn(graphRows(commits, false))).toEqual(['╷╷|own m', '││|own', '╵│|own', ' ╵|base']);
  });

  it('marks every merge of the main branch and joins at the oldest place the branch left it', () => {
    const commits = [
      commit('f', { parents: ['e', 'm2'] }),
      commit('e', { parents: ['d', 'm1'] }),
      commit('d', { parents: ['m0'] }),
      commit('m2', { parents: ['m1'], on_base: true }),
      commit('m1', { parents: ['m0'], on_base: true }),
      commit('m0', { on_base: true }),
    ];
    expect(drawn(graphRows(commits, false))).toEqual(['╷╷|own m', '││|own m', '││|own', '││|base', '││|base', '╵╵|base f']);
  });
});

describe('looseRows', () => {
  it('draws dots only, each on its lane', () => {
    const commits = [commit('b', { parents: ['a'] }), commit('a', { on_base: true })];
    expect(drawn(looseRows(commits))).toEqual(['  |own', '  |base']);
  });

  it('draws nothing for no commits', () => {
    expect(looseRows([])).toEqual([]);
  });
});

describe('commitCountText', () => {
  it.each([
    [7, 7, false, '7 commits'],
    [1, 1, false, '1 commit'],
    [0, 0, false, '0 commits'],
    [2, 7, false, '2 of 7 commits'],
    [0, 7, false, '0 of 7 commits'],
    [1, 1, true, 'newest 1 commit'],
    [200, 200, true, 'newest 200 commits'],
    [3, 200, true, '3 of newest 200 commits'],
  ])('shows %i of %i listed, older ones following: %s, as %s', (shown, all, more, text) => {
    expect(commitCountText(shown, all, more)).toBe(text);
  });
});

describe('aheadText', () => {
  it.each([
    ['main', 4, '4 ahead of main'],
    ['master', 1, '1 ahead of master'],
    ['main', 0, '0 ahead of main'],
    [null, 0, ''],
    [null, 4, ''],
  ])('with the main branch %s and %i commits ahead says %j', (base, ahead, text) => {
    expect(aheadText(base, ahead)).toBe(text);
  });
});

describe('timeText', () => {
  const now = at(2026, 11, 14, 9, 30);

  it.each([
    ['earlier today', at(2026, 11, 14, 9, 22), '09:22'],
    ['at the start of today', at(2026, 11, 14, 0, 0), '00:00'],
    ['later today, on a clock that runs ahead', at(2026, 11, 14, 23, 5), '23:05'],
    ['late yesterday', at(2026, 11, 13, 23, 59), 'yesterday'],
    ['early yesterday', at(2026, 11, 13, 0, 0), 'yesterday'],
    ['two days ago', at(2026, 11, 12, 18, 0), '12 Nov'],
    ['at the start of this year', at(2026, 1, 1, 0, 0), '1 Jan'],
    ['at the end of last year', at(2025, 12, 31, 23, 59), '31 Dec 2025'],
    ['years ago', at(2019, 3, 5, 8, 0), '5 Mar 2019'],
  ])('shows a commit made %s', (_, time, text) => {
    expect(timeText(time, now)).toBe(text);
  });

  it('calls the last day of a year yesterday on the first day of the next', () => {
    expect(timeText(at(2025, 12, 31, 20, 0), at(2026, 1, 1, 8, 0))).toBe('yesterday');
  });
});

describe('momentText', () => {
  const now = at(2026, 11, 14, 9, 30);

  it.each([
    [at(2026, 11, 14, 9, 22), 'today 09:22'],
    [at(2026, 11, 13, 8, 31), 'yesterday 08:31'],
    [at(2026, 11, 3, 8, 31), '3 Nov 08:31'],
    [at(2025, 11, 3, 8, 31), '3 Nov 2025 08:31'],
  ])('shows the moment %i as %s', (time, text) => {
    expect(momentText(time, now)).toBe(text);
  });
});

describe('chosenCommits', () => {
  const shown = [commit('c'), commit('b'), commit('a')];

  it('shows the first commit when nothing was chosen', () => {
    expect(chosenCommits(shown, null, null)).toEqual({ commit: shown[0], from: null });
  });

  it('shows the chosen commit', () => {
    expect(chosenCommits(shown, 'b', null)).toEqual({ commit: shown[1], from: null });
  });

  it('falls back to the first commit when the chosen one is gone, and compares nothing', () => {
    expect(chosenCommits(shown, 'gone', null)).toEqual({ commit: shown[0], from: null });
    expect(chosenCommits(shown, 'gone', 'a')).toEqual({ commit: shown[0], from: null });
  });

  it('compares from the older commit to the newer, whichever was chosen first', () => {
    expect(chosenCommits(shown, 'c', 'a')).toEqual({ commit: shown[0], from: shown[2] });
    expect(chosenCommits(shown, 'a', 'c')).toEqual({ commit: shown[0], from: shown[2] });
  });

  it('does not compare a commit with itself, or with one that is gone', () => {
    expect(chosenCommits(shown, 'b', 'b')).toEqual({ commit: shown[1], from: null });
    expect(chosenCommits(shown, 'b', 'gone')).toEqual({ commit: shown[1], from: null });
  });

  it('shows nothing when there are no commits', () => {
    expect(chosenCommits([], 'b', 'a')).toEqual({ commit: null, from: null });
  });
});

describe('neighbour', () => {
  const shown = [commit('c'), commit('b'), commit('a')];

  it.each([
    ['c', 1, 'b'],
    ['b', 1, 'a'],
    ['b', -1, 'c'],
    ['a', 1, 'a'],
    ['c', -1, 'c'],
    ['c', 5, 'a'],
    ['gone', 1, 'c'],
    [null, -1, 'c'],
  ])('from %s, a step of %i leads to %s', (current, step, next) => {
    expect(neighbour(shown, current, step)).toBe(next);
  });

  it('leads nowhere when there are no commits', () => {
    expect(neighbour([], 'c', 1)).toBeNull();
  });
});

describe('moveStep', () => {
  it.each([
    ['j', 1],
    ['J', 1],
    ['k', -1],
    ['K', -1],
    ['x', 0],
    ['ArrowDown', 0],
  ])('takes %s as a step of %i', (key, step) => {
    expect(moveStep(press({ key }))).toBe(step);
  });

  it.each(['j', 'k'])('takes a %s typed as text as no step', (key) => {
    expect(moveStep(press({ key, element: 'INPUT' }))).toBe(0);
    expect(moveStep(press({ key, editable: true }))).toBe(0);
    expect(moveStep(press({ key, metaKey: true }))).toBe(0);
  });
});

describe('changeRows', () => {
  const files = [file('README.md', 'modified', 2, 0), file('src/ledger/adapter.ts'), file('src/ledger/entry.ts', 'added', 3, 0), file('src/old.ts', 'deleted', 0, 9)];

  it('lists each folder once by its whole path, with its files under it and top files first', () => {
    expect(listed(changeRows(files, new Set()))).toEqual([
      'README.md M +2',
      '▾ src',
      '  old.ts D −9',
      '▾ src/ledger',
      '  adapter.ts M +1 −1',
      '  entry.ts A +3',
    ]);
  });

  it('hides the files of a closed folder, and of that folder only', () => {
    expect(listed(changeRows(files, new Set(['src/ledger'])))).toEqual(['README.md M +2', '▾ src', '  old.ts D −9', '▸ src/ledger']);
    expect(listed(changeRows(files, new Set(['src'])))).toEqual(['README.md M +2', '▸ src', '▾ src/ledger', '  adapter.ts M +1 −1', '  entry.ts A +3']);
  });

  it('keeps the path of each file for opening it', () => {
    const paths = changeRows(files, new Set()).map((row) => row.path);
    expect(paths).toEqual(['README.md', 'src', 'src/old.ts', 'src/ledger', 'src/ledger/adapter.ts', 'src/ledger/entry.ts']);
  });

  it('lists nothing for no files', () => {
    expect(changeRows([], new Set())).toEqual([]);
  });
});

describe('selectedChange', () => {
  const files = [file('a.ts'), file('b.ts')];

  it('is the chosen file', () => {
    expect(selectedChange(files, 'b.ts')).toBe(files[1]);
  });

  it('is the first file when nothing was chosen, or the chosen one is not among them', () => {
    expect(selectedChange(files, null)).toBe(files[0]);
    expect(selectedChange(files, 'gone.ts')).toBe(files[0]);
  });

  it('is nothing when there are no files', () => {
    expect(selectedChange([], 'a.ts')).toBeNull();
  });
});

describe('countsText', () => {
  it.each([
    [1, 1, '+1 −1'],
    [3, 0, '+3'],
    [0, 2, '−2'],
    [0, 0, ''],
    [null, null, ''],
  ])('shows %s added and %s removed as %j', (added, removed, text) => {
    expect(countsText(file('a.ts', 'modified', added, removed))).toBe(text);
  });
});

describe('diffRows', () => {
  it('has no rows for a file without a changed line', () => {
    expect(diffRows(lines('  one', '  two'))).toEqual([]);
    expect(diffRows([])).toEqual([]);
  });

  it('keeps a few unchanged lines around a change and numbers both sides', () => {
    const file = lines('  1', '  2', '  3', '  4', '  5', '  6', '- old', '+ new', '  8', '  9', '  10', '  11', '  12');
    expect(unified(diffRows(file))).toEqual([
      '@@ -4,7 +4,7 @@',
      '4|4   4',
      '5|5   5',
      '6|6   6',
      '7| - old',
      '|7 + new',
      '8|8   8',
      '9|9   9',
      '10|10   10',
    ]);
  });

  it('counts the lines of each side apart once they differ', () => {
    const file = lines('  a', '+ b', '+ c', '  d', '- e', '  f');
    expect(unified(diffRows(file, 1))).toEqual(['@@ -1,4 +1,5 @@', '1|1   a', '|2 + b', '|3 + c', '2|4   d', '3| - e', '4|5   f']);
  });

  it('joins changes whose surroundings touch, and keeps apart those that do not', () => {
    const near = lines('+ a', '  1', '  2', '+ b');
    expect(unified(diffRows(near, 1))).toEqual(['@@ -1,2 +1,4 @@', '|1 + a', '1|2   1', '2|3   2', '|4 + b']);

    const far = lines('+ a', '  1', '  2', '  3', '+ b');
    expect(unified(diffRows(far, 1))).toEqual(['@@ -1,1 +1,2 @@', '|1 + a', '1|2   1', '@@ -3,1 +4,2 @@', '3|4   3', '|5 + b']);
  });

  it('places an empty side after the line before it, as git does', () => {
    expect(unified(diffRows(lines('+ one', '+ two')))).toEqual(['@@ -0,0 +1,2 @@', '|1 + one', '|2 + two']);
    expect(unified(diffRows(lines('- one', '- two')))).toEqual(['@@ -1,2 +0,0 @@', '1| - one', '2| - two']);
    expect(unified(diffRows(lines('  a', '  b', '+ c'), 0))).toEqual(['@@ -2,0 +3,1 @@', '|3 + c']);
  });

  it('shows only the changed lines with no surroundings asked for', () => {
    expect(unified(diffRows(lines('  a', '- b', '+ B', '  c'), 0))).toEqual(['@@ -2,1 +2,1 @@', '2| - b', '|2 + B']);
  });
});

describe('splitRows', () => {
  it('faces each removed line with the added line that replaced it', () => {
    const rows = diffRows(lines('  a', '- b', '- c', '+ B', '+ C', '  d'));
    expect(sideBySide(splitRows(rows))).toEqual(['@@ -1,4 +1,4 @@', '1 a ‖ 1 a', '2 b ‖ 2 B', '3 c ‖ 3 C', '4 d ‖ 4 d']);
  });

  it('leaves a side empty where lines were only added or only removed', () => {
    expect(sideBySide(splitRows(diffRows(lines('  a', '+ b', '  c'))))).toEqual(['@@ -1,2 +1,3 @@', '1 a ‖ 1 a', '· ‖ 2 b', '2 c ‖ 3 c']);
    expect(sideBySide(splitRows(diffRows(lines('  a', '- b', '  c'))))).toEqual(['@@ -1,3 +1,2 @@', '1 a ‖ 1 a', '2 b ‖ ·', '3 c ‖ 2 c']);
  });

  it('leaves the rest of the longer side unfaced', () => {
    expect(sideBySide(splitRows(diffRows(lines('- a', '+ A', '+ B', '+ C'))))).toEqual(['@@ -1,1 +1,3 @@', '1 a ‖ 1 A', '· ‖ 2 B', '· ‖ 3 C']);
    expect(sideBySide(splitRows(diffRows(lines('- a', '- b', '+ A'))))).toEqual(['@@ -1,2 +1,1 @@', '1 a ‖ 1 A', '2 b ‖ ·']);
  });

  it('does not face an added line with a line removed after it', () => {
    expect(sideBySide(splitRows(diffRows(lines('+ A', '- b'))))).toEqual(['@@ -1,1 +1,1 @@', '· ‖ 1 A', '1 b ‖ ·']);
  });

  it('does not face lines across the start of another stretch', () => {
    const rows = diffRows(lines('- a', '  1', '  2', '  3', '+ b'), 0);
    expect(sideBySide(splitRows(rows))).toEqual(['@@ -1,1 +0,0 @@', '1 a ‖ ·', '@@ -4,0 +4,1 @@', '· ‖ 4 b']);
  });

  it('has no rows for no rows', () => {
    expect(splitRows([])).toEqual([]);
  });
});

describe('revertDraft', () => {
  const made = commit('a3f9c21e55', { subject: 'Add idempotency key to ledger writes' });

  it('asks to revert the commit by its short name and first line', () => {
    expect(revertDraft('', made)).toBe('Revert commit `a3f9c21` (Add idempotency key to ledger writes). ');
    expect(revertDraft('   ', made)).toBe('Revert commit `a3f9c21` (Add idempotency key to ledger writes). ');
  });

  it('comes after anything already typed', () => {
    expect(revertDraft(' The retries are safe now. ', made)).toBe(
      'The retries are safe now. Revert commit `a3f9c21` (Add idempotency key to ledger writes). ',
    );
  });
});
