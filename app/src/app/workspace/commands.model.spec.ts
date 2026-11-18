import {
  type Command,
  commandList,
  keyHint,
  keyOutcome,
  matching,
  menuAsked,
  menuRows,
  type Situation,
  stepped,
} from './commands.model';
import { gitWordsIn } from './plain-words.testing';
import { type Change, groupThreads, type KeyPress, type Project, type Thread } from './workspace.model';

function project(id: string, name = `Project ${id}`): Project {
  return { id, name, directory: `/work/${id}`, created_at: `2026-11-0${id.length}T10:00:00` };
}

function thread(id: string, project_id: string, created_at: string, overrides: Partial<Thread> = {}): Thread {
  return { id, project_id, title: `Thread ${id}`, agent: 'claude', status: 'idle', unread: false, commands: [], change: null, created_at, ...overrides };
}

function press(overrides: Partial<KeyPress> = {}): KeyPress {
  return { key: '/', ctrlKey: false, metaKey: false, altKey: false, element: 'BODY', editable: false, ...overrides };
}

function situation(overrides: Partial<Situation> = {}): Situation {
  return { groups: [], project: null, thread: null, agent: 'claude', theme: 'light', opens: { editor: false, terminal: false }, ...overrides };
}

function entry(title: string, detail = '', overrides: Partial<Command> = {}): Command {
  return { id: title, title, detail, group: 'action', key: null, leader: null, live: false, action: { kind: 'toggleTheme' }, ...overrides };
}

const first = project('a', 'Payments');
const second = project('bb', 'Website');
const older = thread('t1', 'a', '2026-11-03T10:00:00');
const newest = thread('t2', 'bb', '2026-11-05T10:00:00', { status: 'running' });
const untitled = thread('t3', 'a', '2026-11-04T10:00:00', { title: '' });
const groups = groupThreads([first, second], [older, newest, untitled]);

function ids(commands: readonly Command[]): string[] {
  return commands.map((candidate) => candidate.id);
}

describe('commandList', () => {
  it('offers only what needs no project when there is none', () => {
    expect(ids(commandList(situation()))).toEqual([
      'action:toggleAgent',
      'action:newProject',
      'action:toggleTheme',
    ]);
  });

  it('lists every thread newest first, then every project, then the actions', () => {
    expect(ids(commandList(situation({ groups, project: first })))).toEqual([
      'thread:t2',
      'thread:t3',
      'thread:t1',
      'project:a',
      'project:bb',
      'action:newThread',
      'action:files',
      'action:history',
      'action:toggleAgent',
      'action:newProject',
      'action:removeProject',
      'action:toggleTheme',
    ]);
  });

  it('names a thread by its title and its project, and marks one an agent is working in', () => {
    const [running, blank] = commandList(situation({ groups, project: first }));
    expect(running).toMatchObject({ title: 'Thread t2', detail: 'Website', live: true, group: 'thread', key: null });
    expect(running.action).toEqual({ kind: 'openThread', thread: newest });
    expect(blank).toMatchObject({ title: 'New thread', detail: 'Payments', live: false });
  });

  it('names a project by its name and its directory, and marks one an agent is working in', () => {
    const commands = commandList(situation({ groups, project: first }));
    expect(commands[3]).toMatchObject({ title: 'Payments', detail: '/work/a', live: false, group: 'project' });
    expect(commands[4]).toMatchObject({ title: 'Website', detail: '/work/bb', live: true });
    expect(commands[4].action).toEqual({ kind: 'openProject', project: second });
  });

  it('lists the threads waiting to be looked at first, newest first, and nowhere else', () => {
    const waiting = groupThreads(
      [first, second],
      [{ ...older, unread: true }, newest, { ...untitled, unread: true }, thread('t4', 'bb', '2026-11-02T10:00:00')],
    );
    const commands = commandList(situation({ groups: waiting, project: first }));
    expect(commands.slice(0, 4).map((candidate) => [candidate.id, candidate.group])).toEqual([
      ['thread:t3', 'attention'],
      ['thread:t1', 'attention'],
      ['thread:t2', 'thread'],
      ['thread:t4', 'thread'],
    ]);
    expect(ids(commands).filter((id) => id.startsWith('thread:'))).toHaveLength(4);
  });

  it('does not count a thread as waiting while an agent works in it again', () => {
    const working = groupThreads([first], [{ ...older, unread: true, status: 'running' }]);
    expect(commandList(situation({ groups: working, project: first }))[0]).toMatchObject({ id: 'thread:t1', group: 'thread', live: true });
  });

  it.each([
    ['action:newThread', 'n', { kind: 'openProject', project: first }],
    ['action:files', 'c', { kind: 'showScreen', screen: 'files' }],
    ['action:history', 'h', { kind: 'showScreen', screen: 'history' }],
    ['action:removeProject', null, { kind: 'removeProject', project: first }],
  ] as const)('aims %s at the selected project, on the key %s', (id, key, action) => {
    const found = commandList(situation({ groups, project: first })).find((candidate) => candidate.id === id);
    expect(found?.key).toBe(key);
    expect(found?.action).toEqual(action);
  });

  it('names the project it would remove', () => {
    const commands = commandList(situation({ groups, project: second }));
    expect(commands.find((candidate) => candidate.id === 'action:removeProject')?.title).toBe('Remove project Website');
  });

  it('offers to stop the agent only in a thread one is working in', () => {
    const idle = commandList(situation({ groups, project: first, thread: older }));
    const running = commandList(situation({ groups, project: second, thread: newest }));
    expect(ids(idle)).not.toContain('action:stopAgent');
    expect(running.find((candidate) => candidate.id === 'action:stopAgent')).toMatchObject({
      title: 'Stop the agent',
      detail: 'Thread t2',
      action: { kind: 'stopAgent', thread: newest },
    });
    expect(ids(running).indexOf('action:stopAgent')).toBe(ids(running).indexOf('action:history') + 1);
  });

  describe('the agent commands', () => {
    const compacting = { ...older, commands: ['compact', 'user:plan'] };
    const reviewing = { ...newest, commands: ['review'] };
    const grokking = thread('t5', 'a', '2026-11-01T10:00:00', { agent: 'grok', commands: ['goal'] });
    const known = groupThreads([first, second], [compacting, reviewing, untitled, grokking]);

    function agentRows(overrides: Partial<Situation>): [string, string, string][] {
      return commandList(situation({ groups: known, ...overrides }))
        .filter((candidate) => candidate.group === 'agent')
        .map((candidate) => [candidate.id, candidate.title, candidate.detail]);
    }

    it('come last, each putting its name in the message box, ready for what follows it', () => {
      const commands = commandList(situation({ groups: known, project: first, thread: compacting }));
      expect(commands.slice(-2)).toEqual([
        expect.objectContaining({ id: 'agent:compact', title: '/compact', detail: 'Claude command', key: null, action: { kind: 'draft', text: '/compact ' } }),
        expect.objectContaining({ id: 'agent:user:plan', title: '/user:plan', action: { kind: 'draft', text: '/user:plan ' } }),
      ]);
    });

    it('are the ones of the open thread, under the name of its agent', () => {
      expect(agentRows({ project: first, thread: grokking, agent: 'claude' })).toEqual([['agent:goal', '/goal', 'Grok command']]);
    });

    it('are none for an open thread whose agent has not run yet', () => {
      expect(agentRows({ project: first, thread: untitled })).toEqual([]);
    });

    it('are borrowed for a new thread from the newest thread of the same agent in the project', () => {
      expect(agentRows({ project: first, agent: 'claude' }).map(([id]) => id)).toEqual(['agent:compact', 'agent:user:plan']);
      expect(agentRows({ project: first, agent: 'grok' }).map(([id]) => id)).toEqual(['agent:goal']);
      expect(agentRows({ project: second, agent: 'claude' }).map(([id]) => id)).toEqual(['agent:review']);
    });

    it('are borrowed from another project when the project has none for the agent', () => {
      expect(agentRows({ project: second, agent: 'grok' })).toEqual([['agent:goal', '/goal', 'Grok command']]);
    });

    it('are none when no thread of the agent has run, or there is no project', () => {
      const fresh = groupThreads([first], [untitled]);
      expect(agentRows({ groups: fresh, project: first })).toEqual([]);
      expect(agentRows({ groups: [], project: null })).toEqual([]);
    });

    it('are found by their name with or without the slash', () => {
      const commands = commandList(situation({ groups: known, project: first, thread: compacting }));
      expect(matching(commands, '/comp')[0].id).toBe('agent:compact');
      expect(matching(commands, 'comp')[0].id).toBe('agent:compact');
      expect(matching(commands, 'plan')[0].id).toBe('agent:user:plan');
    });
  });

  it.each([
    ['claude', 'Use Grok for new threads'],
    ['grok', 'Use Claude for new threads'],
  ] as const)('with %s chosen offers the other agent', (agent, title) => {
    expect(commandList(situation({ agent })).find((candidate) => candidate.id === 'action:toggleAgent')?.title).toBe(title);
  });

  it.each([
    ['light', 'Dark theme'],
    ['dark', 'Light theme'],
  ] as const)('in the %s theme offers the other one', (theme, title) => {
    expect(commandList(situation({ theme })).find((candidate) => candidate.id === 'action:toggleTheme')?.title).toBe(title);
  });
});

describe('matching', () => {
  const commands = [
    entry('History', 'Payments'),
    entry('Fix the search', 'Website'),
    entry('Payments', '/work/pay'),
    entry('Sign out'),
    entry('New thread', 'Payments'),
  ];

  function titles(typed: string): string[] {
    return matching(commands, typed).map((candidate) => candidate.title);
  }

  it.each([[''], ['   ']])('keeps everything in order for %j', (typed) => {
    expect(titles(typed)).toEqual(commands.map((candidate) => candidate.title));
  });

  it('finds a title whatever the case and the spaces around what was typed', () => {
    expect(titles('  SIGN ')).toEqual(['Sign out']);
  });

  it('puts a title that starts with it first, then a word that does, then the letters together, then spread out', () => {
    const ranked = [entry('a best'), entry('the best'), entry('nobest'), entry('b-e-s-t'), entry('best')];
    expect(matching(ranked, 'best').map((candidate) => candidate.title)).toEqual(['best', 'a best', 'the best', 'nobest', 'b-e-s-t']);
  });

  it('puts every fitting title before a fitting detail', () => {
    expect(titles('pay')).toEqual(['Payments', 'History', 'New thread']);
  });

  it('finds letters typed in order across a title', () => {
    expect(titles('fxsr')).toEqual(['Fix the search']);
  });

  it('does not find letters spread out across a detail', () => {
    expect(titles('wrkpy')).toEqual([]);
    expect(titles('wbst')).toEqual([]);
  });

  it('finds letters held together inside a detail', () => {
    expect(titles('ebsi')).toEqual(['Fix the search']);
  });

  it('finds a word after a slash in a detail', () => {
    expect(titles('work')).toEqual(['Payments']);
  });

  it('keeps equal fits in their listed order', () => {
    expect(titles('t')).toEqual(['Fix the search', 'New thread', 'History', 'Payments', 'Sign out']);
  });

  it('finds nothing for letters no command holds in that order', () => {
    expect(titles('zz')).toEqual([]);
    expect(titles('tuo ngis')).toEqual([]);
  });
});

describe('menuRows', () => {
  const commands = [
    entry('Zero', '', { group: 'attention' }),
    entry('One', '', { group: 'thread' }),
    entry('Two', '', { group: 'thread' }),
    entry('Three', '', { group: 'project' }),
    entry('Four', '', { group: 'action' }),
    entry('Five', '', { group: 'agent' }),
  ];

  it('names each section once while nothing is typed', () => {
    expect(menuRows(commands, ' ').map((row) => row.heading)).toEqual(['Waiting for you', 'Threads', null, 'Projects', 'Actions', 'Agent commands']);
  });

  it('names no section for a search', () => {
    expect(menuRows(commands, 'o').map((row) => row.heading)).toEqual([null, null, null, null, null, null]);
  });

  it('keeps the commands in order', () => {
    expect(menuRows(commands, '').map((row) => row.command)).toEqual(commands);
  });

  it('is empty for no commands', () => {
    expect(menuRows([], '')).toEqual([]);
  });
});

describe('menuAsked', () => {
  it.each(['BODY', 'BUTTON', 'DIV'])('takes a slash pressed on a %s', (element) => {
    expect(menuAsked(press({ element }), 'some text')).toBe(true);
  });

  it.each(['', '  ', '\n'])('takes a slash in the message box holding %j', (draft) => {
    expect(menuAsked(press({ element: 'TEXTAREA' }), draft)).toBe(true);
  });

  it.each(['src', ' a', '/'])('leaves a slash typed in the message box after %j', (draft) => {
    expect(menuAsked(press({ element: 'TEXTAREA' }), draft)).toBe(false);
  });

  it.each(['INPUT', 'SELECT'])('leaves a slash typed into a %s, even with an empty message box', (element) => {
    expect(menuAsked(press({ element }), '')).toBe(false);
  });

  it('leaves a slash typed into text edited in place', () => {
    expect(menuAsked(press({ element: 'DIV', editable: true }), '')).toBe(false);
  });

  it.each(['ctrlKey', 'metaKey', 'altKey'] as const)('leaves a slash held with %s', (modifier) => {
    expect(menuAsked(press({ [modifier]: true }), '')).toBe(false);
  });

  it.each([
    ['k', 'ctrlKey', 'BODY'],
    ['K', 'ctrlKey', 'BODY'],
    ['k', 'metaKey', 'BODY'],
    ['k', 'ctrlKey', 'TEXTAREA'],
    ['k', 'ctrlKey', 'INPUT'],
  ] as const)('takes %s held with %s on a %s, whatever is typed', (key, modifier, element) => {
    expect(menuAsked(press({ key, [modifier]: true, element }), 'some text')).toBe(true);
  });

  it('leaves a K on its own, with Alt, or with both Ctrl and Command', () => {
    expect(menuAsked(press({ key: 'k' }), '')).toBe(false);
    expect(menuAsked(press({ key: 'k', ctrlKey: true, altKey: true }), '')).toBe(false);
    expect(menuAsked(press({ key: 'k', ctrlKey: true, metaKey: true }), '')).toBe(false);
  });

  it.each(['c', 'Enter', 'Escape', '?'])('leaves %s', (key) => {
    expect(menuAsked(press({ key }), '')).toBe(false);
  });
});

describe('keyOutcome', () => {
  const files = entry('Files', '', { key: 'c' });
  const history = entry('History', '', { key: 'h' });
  const copy = entry('Copy location', '', { key: 'y', leader: 'w' });
  const folder = entry('Open folder', '', { key: 'f', leader: 'w' });
  const commands = [entry('Settings'), files, history, copy, folder];

  describe('without a leader pressed', () => {
    it.each([
      ['c', files],
      ['C', files],
      ['h', history],
    ])('runs the command bound to %s', (key, expected) => {
      expect(keyOutcome(press({ key }), commands, null)).toEqual({ kind: 'run', command: expected });
    });

    it('waits for a second key after a leader', () => {
      expect(keyOutcome(press({ key: 'w' }), commands, null)).toEqual({ kind: 'wait', leader: 'w' });
      expect(keyOutcome(press({ key: 'W' }), commands, null)).toEqual({ kind: 'wait', leader: 'w' });
    });

    it.each(['x', 'y', 'f', '/', 'Enter', 'Escape'])('takes %s for nothing, even the second key of a family', (key) => {
      expect(keyOutcome(press({ key }), commands, null)).toBeNull();
    });

    it('takes a letter typed as text, or held with Ctrl, for nothing', () => {
      expect(keyOutcome(press({ key: 'c', element: 'TEXTAREA' }), commands, null)).toBeNull();
      expect(keyOutcome(press({ key: 'c', element: 'INPUT' }), commands, null)).toBeNull();
      expect(keyOutcome(press({ key: 'c', ctrlKey: true }), commands, null)).toBeNull();
      expect(keyOutcome(press({ key: 'w', element: 'TEXTAREA' }), commands, null)).toBeNull();
    });

    it('takes a letter for nothing when its command is not offered', () => {
      expect(keyOutcome(press({ key: 'c' }), [entry('Settings')], null)).toBeNull();
      expect(keyOutcome(press({ key: 'w' }), [files], null)).toBeNull();
      expect(keyOutcome(press({ key: 'c' }), [], null)).toBeNull();
    });
  });

  describe('after a leader', () => {
    it.each([
      ['y', copy],
      ['F', folder],
    ])('runs the command of the family bound to %s', (key, expected) => {
      expect(keyOutcome(press({ key }), commands, 'w')).toEqual({ kind: 'run', command: expected });
    });

    it.each(['c', 'x', 'w', 'Escape', 'Enter'])('gives up on %s, a key outside the family', (key) => {
      expect(keyOutcome(press({ key }), commands, 'w')).toEqual({ kind: 'cancel' });
    });

    it('gives up on a letter held with Ctrl', () => {
      expect(keyOutcome(press({ key: 'y', ctrlKey: true }), commands, 'w')).toEqual({ kind: 'cancel' });
    });

    it.each(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'])('keeps waiting through %s on its own', (key) => {
      expect(keyOutcome(press({ key }), commands, 'w')).toBeNull();
    });
  });
});

describe('keyHint', () => {
  it('shows a key on its own', () => {
    expect(keyHint(entry('Files', '', { key: 'c' }))).toBe('C');
  });

  it('shows the leader before the key', () => {
    expect(keyHint(entry('Copy location', '', { key: 'y', leader: 'w' }))).toBe('W Y');
  });

  it('shows nothing for a command without a key', () => {
    expect(keyHint(entry('Settings'))).toBeNull();
  });
});

describe('stepped', () => {
  it.each([
    [0, 1, 3, 1],
    [1, 1, 3, 2],
    [2, 1, 3, 0],
    [0, -1, 3, 2],
    [2, -1, 3, 1],
    [0, 1, 1, 0],
    [0, -1, 1, 0],
    [5, 1, 3, 0],
    [0, 1, 0, 0],
    [0, -1, 0, 0],
  ])('from line %i by %i in a list of %i lands on %i', (index, step, length, landed) => {
    expect(stepped(index, step, length)).toBe(landed);
  });
});

describe('the commands for a thread\'s change', () => {
  function change(overrides: Partial<Change> = {}): Change {
    return {
      name: 'fix-login',
      path: '/work/a/.teryon/worktrees/fix-login',
      base: 'main',
      start: 'abc',
      branch: 'fix-login',
      state: { copy_present: true, base_present: true, files_changed: 1, fit: 'applies', unfinished: false },
      ...overrides,
    };
  }

  function offered(thread: Thread | null, opens = { editor: false, terminal: false }): Command[] {
    const groupsWith = thread ? groupThreads([first], [thread]) : groups;
    return commandList(situation({ groups: groupsWith, project: first, thread, opens })).filter(
      (candidate) => candidate.group === 'change',
    );
  }

  const changed = thread('t9', 'a', '2026-11-06T10:00:00', { change: change() });
  /** What is offered for a change whose copy is there, whatever is set up. */
  const ALWAYS = ['change:apply', 'change:catchUp', 'change:discard', 'change:copyLocation', 'change:openFolder'];

  it('offers nothing without an open thread, or for a thread with no change', () => {
    expect(offered(null)).toEqual([]);
    expect(offered(older)).toEqual([]);
  });

  it('offers applying, catching up, discarding, copying the location and opening the folder, after the projects and before the actions', () => {
    const commands = commandList(situation({ groups: groupThreads([first], [changed]), project: first, thread: changed }));
    expect(ids(commands)).toEqual([
      'thread:t9',
      'project:a',
      'change:apply',
      'change:catchUp',
      'change:discard',
      'change:copyLocation',
      'change:openFolder',
      'action:newThread',
      'action:files',
      'action:history',
      'action:toggleAgent',
      'action:newProject',
      'action:removeProject',
      'action:toggleTheme',
    ]);
  });

  it('names the change beside each command, with keys after W', () => {
    expect(offered(changed).map((candidate) => [candidate.title, candidate.detail, keyHint(candidate)])).toEqual([
      ['Apply to the project', 'fix-login', 'W A'],
      ['Catch up with the project', 'fix-login', 'W U'],
      ['Discard changes', 'fix-login', 'W D'],
      ['Copy location', 'fix-login', 'W Y'],
      ['Open folder', 'fix-login', 'W F'],
    ]);
  });

  it('applies, catches up and discards the open thread\'s change', () => {
    const [apply, catchUp, discard] = offered(changed);
    expect(apply.action).toEqual({ kind: 'applyChange', thread: changed });
    expect(catchUp.action).toEqual({ kind: 'catchUp', thread: changed });
    expect(discard.action).toEqual({ kind: 'discardChange', thread: changed });
  });

  it('copies the copy\'s full location and opens the open thread\'s copy', () => {
    const [, , , copy, folder] = offered(changed);
    expect(copy.action).toEqual({ kind: 'copyLocation', path: '/work/a/.teryon/worktrees/fix-login' });
    expect(folder.action).toEqual({ kind: 'openCopy', thread: changed, opener: 'folder' });
  });

  it.each([
    [{ editor: true, terminal: false }, [...ALWAYS, 'change:openEditor']],
    [{ editor: false, terminal: true }, [...ALWAYS, 'change:openTerminal']],
    [{ editor: true, terminal: true }, [...ALWAYS, 'change:openEditor', 'change:openTerminal']],
  ])('offers the editor and the terminal only when set up: %o', (opens, expected) => {
    expect(ids(offered(changed, opens))).toEqual(expected);
  });

  it('offers the editor and the terminal on W E and W T', () => {
    const [, , , , , editor, terminal] = offered(changed, { editor: true, terminal: true });
    expect([keyHint(editor), keyHint(terminal)]).toEqual(['W E', 'W T']);
    expect(editor.action).toEqual({ kind: 'openCopy', thread: changed, opener: 'editor' });
  });

  it('offers only discarding once the copy is gone', () => {
    const gone = { ...changed, change: change({ state: { copy_present: false, base_present: false, files_changed: 0, fit: 'unknown', unfinished: false } }) };
    expect(ids(offered(gone, { editor: true, terminal: true }))).toEqual(['change:discard']);
  });

  it('offers them all before Teryon has looked at the change', () => {
    const unread = { ...changed, change: change({ state: null }) };
    expect(ids(offered(unread))).toEqual(ALWAYS);
  });

  it.each([
    ['nothing changed', { files_changed: 0 }],
    ['it overlaps', { fit: 'overlaps' as const }],
    ['the agent left it unfinished', { unfinished: true }],
    ['its starting point is gone', { base_present: false }],
  ])('still offers applying when %s, to say why it cannot be done', (_, state) => {
    const whole = { copy_present: true, base_present: true, files_changed: 1, fit: 'applies' as const, unfinished: false };
    const stuck = { ...changed, change: change({ state: { ...whole, ...state } }) };
    expect(ids(offered(stuck))).toEqual(ALWAYS);
  });

  it('heads them as the thread\'s changes', () => {
    const rows = menuRows(offered(changed), '');
    expect(rows[0].heading).toBe('This thread’s changes');
  });

  it('never puts a git word in a title or a heading', () => {
    const everything = commandList(
      situation({ groups: groupThreads([first], [changed]), project: first, thread: changed, opens: { editor: true, terminal: true } }),
    );
    const texts = [...everything.map((candidate) => candidate.title), ...menuRows(everything, '').map((row) => row.heading ?? '')];
    expect(texts.flatMap(gitWordsIn)).toEqual([]);
  });
});
