import { commandList, type Situation } from './commands.model';
import { filteredSections, SHARED_KEYS, shortcutSections, type ShortcutSection } from './shortcuts.model';
import { changeBadge, FIRST_NEW_THREADS, NO_MODELS, NO_WORKFLOWS, type Change, type Project, type Thread } from './workspace.model';

const project: Project = { id: 'p', name: 'App', directory: '/work/app', created_at: '2026-11-01T10:00:00', on_github: true };
const thread: Thread = {
  id: 't',
  project_id: project.id,
  title: 'Fix login',
  agent: 'claude',
  status: 'idle',
  unread: false,
  commands: [],
  change: null,
  finished: null,
  reviewer: null,
  created_at: '2026-11-01T10:00:00',
};
const change: Change = {
  name: 'fix-login',
  path: '/work/app/.teryon/worktrees/fix-login',
  base: 'main',
  start: 'abc',
  branch: 'fix-login',
  state: { copy_present: true, base_present: true, files_changed: 1, fit: 'applies', unfinished: false },
  review: null,
};
const review = { number: 1, url: 'https://example.com/review/1', state: 'in_review' } as const;

function commands(overrides: Partial<Situation> = {}) {
  return commandList({
    groups: [],
    project,
    thread: null,
    newThreads: FIRST_NEW_THREADS,
    models: NO_MODELS,
    to: 'author',
    theme: 'light',
    opens: { editor: false, terminal: false },
    questionsPutAway: false,
    workflows: NO_WORKFLOWS,
    ...overrides,
  });
}

function keys(section: ShortcutSection): string[] {
  return section.rows.map((row) => row.keys.join(' '));
}

describe('shortcutSections', () => {
  it.each([
    ['home', 'Home', ['↵', '@']],
    ['thread', 'Thread', ['esc']],
    ['board', 'Board', ['h j k l', '↵', 'a', 'esc']],
    ['history', 'History', ['j k', 'esc']],
    ['files', 'Code', ['esc']],
    ['usage', 'Usage', ['esc']],
    ['settings', 'Settings', ['esc']],
    ['plan', 'Plan', ['esc']],
  ] as const)('shows the keys handled on %s', (place, name, localKeys) => {
    const sections = shortcutSections(commands(), place, '');
    expect(sections.map((section) => section.heading)).toEqual([`Here · ${name}`, 'Everywhere', 'Go to']);
    expect(keys(sections[0])).toEqual(localKeys);
    expect(keys(sections[1])).toEqual(['/ Ctrl+K', 'esc', '?']);
  });

  it('offers the Home list keys when threads need attention', () => {
    const [empty] = shortcutSections(commands(), 'home', '');
    const [waiting] = shortcutSections(commands(), 'home', '', { waitingThreads: true });
    expect(empty.rows[0].text).toBe('Run');
    expect(keys(waiting)).toEqual(['tab', 'j k', '↵', '@']);
    expect(waiting.rows[2].text).toBe('Open or run');
  });

  it('leaves out Home controls without a project, and the Board switch without a project to switch to', () => {
    const offered = commands({ project: null });
    expect(shortcutSections(offered, 'home', '').map((section) => section.heading)).toEqual(['Everywhere', 'Go to']);
    expect(keys(shortcutSections(offered, 'board', '')[0])).toEqual(['h j k l', '↵', 'esc']);
  });

  it.each([0, 1, 3, 9, 10])('shows only available Usage numbers with %i agents', (usageAgents) => {
    const [here] = shortcutSections(commands(), 'usage', '', { usageAgents });
    const numbers = ['1', '2', '3', '4', '5', '6', '7', '8', '9'].slice(0, usageAgents).join(' ');
    expect(keys(here)).toEqual(usageAgents ? [numbers, 'esc'] : ['esc']);
  });

  it.each([
    [false, false, []],
    [true, false, ['w e']],
    [false, true, ['w t']],
    [true, true, ['w e', 'w t']],
  ] as const)('shows only keyed change actions with editor %s and terminal %s', (editor, terminal, extra) => {
    const changed = { ...thread, change };
    const offered = commands({ thread: changed, opens: { editor, terminal } });
    const [here] = shortcutSections(offered, 'thread', changeBadge(changed));
    expect(here.heading).toBe('Here · ready to apply');
    expect(keys(here)).toEqual(['w v', 'w a', 'w u', 'w c', 'w r', 'w d', 'w y', 'w f', ...extra, 'esc']);
    for (const row of here.rows.filter((row) => row.keys[0] === 'w')) {
      const command = offered.find((command) => command.leader === 'w' && command.key === row.keys[1]);
      expect(row.text).toBe(command?.title);
    }
    expect(shortcutSections(offered, 'files', changeBadge(changed))[0].heading).toBe('Here · Code');
  });

  it.each([
    [null, ['w d', 'esc']],
    [review, ['w o', 'w d', 'esc']],
  ] as const)('lists what can still be done after the copy is gone with review %j', (review, expected) => {
    const changed: Thread = {
      ...thread,
      change: { ...change, review, state: { ...change.state!, copy_present: false } },
    };
    const [here] = shortcutSections(commands({ thread: changed, opens: { editor: true, terminal: true } }), 'thread', changeBadge(changed));
    expect(keys(here)).toEqual(expected);
  });

  it('shows the keys to open a review and pass findings when they are offered', () => {
    const reviewed: Thread = {
      ...thread,
      change: { ...change, review },
      reviewer: { agent: 'codex', reviewing: false, verdict: { findings: 1, answered: false, cut: false } },
    };
    const [here] = shortcutSections(commands({ thread: reviewed }), 'thread', changeBadge(reviewed));
    expect(keys(here)).toEqual(['w v', 'w a', 'w u', 'w c', 'w p', 'w o', 'w d', 'w y', 'w f', 'esc']);
  });

  it.each([
    [false, false, []],
    [true, false, ['q']],
    [false, true, ['p']],
    [true, true, ['q', 'p']],
  ] as const)('shows the question and plan keys only when offered (%s, %s)', (questionsPutAway, hasPlan, expected) => {
    const plan = hasPlan ? { title: 'Login fix', fingerprint: 'abc', too_large: false } : null;
    const sections = shortcutSections(commands({ thread: { ...thread, plan }, questionsPutAway }), 'thread', '');
    expect(keys(sections[0])).toEqual([...expected, 'esc']);
    expect(keys(sections[2]).includes('p')).toBe(hasPlan);
    expect(keys(sections[2])).not.toContain('q');
  });

  it('gets navigation from the keyed commands, with the Board’s History key spelled as uppercase H', () => {
    const offered = commands();
    const withProject = shortcutSections(offered, 'home', '').at(-1)!;
    const afterBoard = shortcutSections(offered, 'board', '').at(-1)!;
    const withoutProject = shortcutSections(commands({ project: null }), 'home', '').at(-1)!;
    expect(withProject.rows.map((row) => [row.keys.join(' '), row.text])).toEqual([
      ['g h', 'Home'], ['n', 'New thread'], ['c', 'Code'], ['h', 'History'], ['b', 'Board'], ['u', 'Usage'], ['s', 'Settings'],
    ]);
    expect(afterBoard.rows.find((row) => row.text === 'History')?.keys).toEqual(['H']);
    expect(keys(withoutProject)).toEqual(['g h', 'b', 'u', 's']);
  });

  it('shows the same meanings in the note on every screen', () => {
    expect(keys({ heading: '', rows: SHARED_KEYS })).toEqual(['/', 'esc', 'w', 'p', '?']);
    expect(SHARED_KEYS.map((row) => row.text)).toEqual(['menu', 'back or close', 'this thread’s changes', 'plan', 'this list']);
  });
});

describe('filteredSections', () => {
  const sections = shortcutSections(commands({ thread: { ...thread, change } }), 'thread', 'ready to apply');

  it.each(['', '  '])('keeps all sections and rows for %j', (typed) => {
    expect(filteredSections(sections, typed)).toEqual(sections);
  });

  it('matches a description case-insensitively and removes empty sections', () => {
    expect(filteredSections(sections, '  SeTTingS ')).toEqual([
      { heading: 'Go to', rows: [{ keys: ['s'], text: 'Settings' }] },
    ]);
  });

  it.each([['w f', 'Open folder'], ['?', 'Show shortcuts'], ['Ctrl+K', 'Open the menu to search, jump or start a thread']])(
    'finds %s among the keys',
    (key, text) => expect(filteredSections(sections, key).flatMap((section) => section.rows.map((row) => row.text))).toContain(text),
  );

  it('ranks descriptions the same way as the menu, before matching keys', () => {
    const rows = [
      { keys: ['ab'], text: 'Keys' },
      { keys: ['2'], text: 'Aboard' },
      { keys: ['3'], text: 'Able' },
      { keys: ['4'], text: 'At a bend' },
      { keys: ['5'], text: 'Go aboard' },
    ];
    expect(filteredSections([{ heading: 'Here', rows }], 'ab')[0].rows.map((row) => row.text)).toEqual([
      'Aboard', 'Able', 'Go aboard', 'At a bend', 'Keys',
    ]);
  });

  it('returns no sections if nothing matches', () => {
    expect(filteredSections(sections, 'zzzz')).toEqual([]);
  });
});
