import {
  askDraft,
  changeLetter,
  changeTag,
  directoryName,
  effectiveMode,
  type FileChange,
  type FileEntry,
  type FileLine,
  isFilesShortcut,
  type KeyPress,
  languageOf,
  lineCountText,
  liveText,
  modeAvailable,
  rowIndent,
  sameAnswer,
  selectedEntry,
  shownLines,
  splitPath,
  type TreeRow,
  treeRows,
} from './files.model';

function file(path: string, change: FileChange | null = null): FileEntry {
  return { path, change };
}

/** The rows as short text: `▾ src ●`, `  adapter.ts M`, indented two spaces a level. */
function drawn(rows: readonly TreeRow[]): string[] {
  return rows.map((row) => {
    const mark = row.kind === 'folder' ? (row.open ? '▾ ' : '▸ ') : '';
    const status = row.kind === 'folder' ? (row.changed ? ' ●' : '') : row.change ? ` ${changeLetter(row.change)}` : '';
    return `${'  '.repeat(row.depth)}${mark}${row.name}${status}`;
  });
}

function line(change: FileLine['change'], number: number | null, text: string): FileLine {
  return { change, number, text };
}

function press(overrides: Partial<KeyPress> = {}): KeyPress {
  return { key: 'c', ctrlKey: false, metaKey: false, altKey: false, element: 'BODY', editable: false, ...overrides };
}

const NOTHING_TOGGLED: ReadonlyMap<string, boolean> = new Map();

const FILES: readonly FileEntry[] = [
  file('.github/workflows/ci.yml'),
  file('README.md'),
  file('migrations/0041_ledger.sql'),
  file('migrations/0042_drop_legacy.sql', 'added'),
  file('package.json'),
  file('src/api/routes.ts'),
  file('src/ledger/adapter.ts', 'modified'),
  file('src/ledger/entry.ts', 'added'),
  file('src/ledger/index.ts'),
  file('src/legacy/types.ts'),
];

describe('treeRows', () => {
  it('opens the folders that hold changes and leaves the others closed', () => {
    expect(drawn(treeRows(FILES, '', NOTHING_TOGGLED, null))).toEqual([
      '▸ .github',
      '▾ migrations ●',
      '  0041_ledger.sql',
      '  0042_drop_legacy.sql A',
      '▾ src ●',
      '  ▸ api',
      '  ▾ ledger ●',
      '    adapter.ts M',
      '    entry.ts A',
      '    index.ts',
      '  ▸ legacy',
      'package.json',
      'README.md',
    ]);
  });

  it('opens the folders around the selected file', () => {
    expect(drawn(treeRows([file('a/b/c.txt'), file('a/d.txt'), file('e/f.txt')], '', NOTHING_TOGGLED, 'a/b/c.txt'))).toEqual([
      '▾ a',
      '  ▾ b',
      '    c.txt',
      '  d.txt',
      '▸ e',
    ]);
  });

  it('does not open a folder whose name only starts like the selected one', () => {
    expect(drawn(treeRows([file('ab/c.txt'), file('a/c.txt')], '', NOTHING_TOGGLED, 'ab/c.txt'))).toEqual(['▸ a', '▾ ab', '  c.txt']);
  });

  it('keeps a folder the way the person left it', () => {
    const toggled = new Map([
      ['src', false],
      ['.github', true],
    ]);
    expect(drawn(treeRows(FILES, '', toggled, null))).toEqual([
      '▾ .github',
      '  ▸ workflows',
      '▾ migrations ●',
      '  0041_ledger.sql',
      '  0042_drop_legacy.sql A',
      '▸ src ●',
      'package.json',
      'README.md',
    ]);
  });

  it('shows only matching files while a filter is typed, with their folders open', () => {
    expect(drawn(treeRows(FILES, '  LEDGER ', NOTHING_TOGGLED, null))).toEqual([
      '▾ migrations',
      '  0041_ledger.sql',
      '▾ src ●',
      '  ▾ ledger ●',
      '    adapter.ts M',
      '    entry.ts A',
      '    index.ts',
    ]);
  });

  it('lets a folder be closed while a filter is typed', () => {
    expect(drawn(treeRows(FILES, 'ledger', new Map([['src', false]]), null))).toEqual([
      '▾ migrations',
      '  0041_ledger.sql',
      '▸ src ●',
    ]);
  });

  it('matches a filter against the whole path', () => {
    expect(drawn(treeRows(FILES, 'src/api', NOTHING_TOGGLED, null))).toEqual(['▾ src', '  ▾ api', '    routes.ts']);
  });

  it('has no rows when nothing matches', () => {
    expect(treeRows(FILES, 'nothing like this', NOTHING_TOGGLED, null)).toEqual([]);
  });

  it('has no rows without files', () => {
    expect(treeRows([], '', NOTHING_TOGGLED, null)).toEqual([]);
  });

  it('puts folders before files and ignores capitals when sorting', () => {
    expect(drawn(treeRows([file('b.txt'), file('Zebra/x.txt'), file('A.txt'), file('apple/y.txt')], '', NOTHING_TOGGLED, null))).toEqual([
      '▸ apple',
      '▸ Zebra',
      'A.txt',
      'b.txt',
    ]);
  });

  it('marks a deleted file and the folder it was in', () => {
    expect(drawn(treeRows([file('old/gone.txt', 'deleted')], '', NOTHING_TOGGLED, null))).toEqual(['▾ old ●', '  gone.txt D']);
  });

  it('gives each row its whole path', () => {
    expect(treeRows([file('a/b/c.txt')], '', NOTHING_TOGGLED, 'a/b/c.txt').map((row) => row.path)).toEqual(['a', 'a/b', 'a/b/c.txt']);
  });
});

describe('rowIndent', () => {
  const rows = treeRows([file('a/b/c.txt'), file('d.txt')], '', NOTHING_TOGGLED, 'a/b/c.txt');

  it('moves each level further right, with files lined up with folder names', () => {
    expect(rows.map((row) => `${row.name} ${rowIndent(row)}`)).toEqual(['a 8', 'b 22', 'c.txt 50', 'd.txt 22']);
  });
});

describe('selectedEntry', () => {
  it('is the chosen file', () => {
    expect(selectedEntry(FILES, 'README.md')).toEqual(file('README.md'));
  });

  it('is the first changed file when nothing is chosen', () => {
    expect(selectedEntry(FILES, null)).toEqual(file('migrations/0042_drop_legacy.sql', 'added'));
  });

  it('is the first changed file when the chosen one is gone', () => {
    expect(selectedEntry(FILES, 'src/removed.ts')).toEqual(file('migrations/0042_drop_legacy.sql', 'added'));
  });

  it('is nothing when nothing is chosen and nothing changed', () => {
    expect(selectedEntry([file('README.md')], null)).toBeNull();
  });

  it('is nothing without files', () => {
    expect(selectedEntry([], 'README.md')).toBeNull();
  });
});

describe('splitPath', () => {
  it('separates the folders from the name', () => {
    expect(splitPath('src/ledger/adapter.ts')).toEqual({ folders: 'src / ledger /', name: 'adapter.ts' });
  });

  it('has no folders for a file at the top', () => {
    expect(splitPath('README.md')).toEqual({ folders: '', name: 'README.md' });
  });
});

describe('changeLetter', () => {
  it.each([
    ['modified', 'M'],
    ['added', 'A'],
    ['deleted', 'D'],
    [null, ''],
  ] as const)('for %s is "%s"', (change, letter) => {
    expect(changeLetter(change)).toBe(letter);
  });
});

describe('changeTag', () => {
  it('names the branch the changes are counted from', () => {
    expect(changeTag('modified', 'main')).toBe('modified since main');
    expect(changeTag('added', 'master')).toBe('added since master');
    expect(changeTag('deleted', 'main')).toBe('deleted since main');
  });

  it('says the change is not committed when only uncommitted work counts', () => {
    expect(changeTag('modified', null)).toBe('modified, not committed');
    expect(changeTag('added', null)).toBe('added, not committed');
  });

  it('is empty for an unchanged file', () => {
    expect(changeTag(null, 'main')).toBe('');
    expect(changeTag(null, null)).toBe('');
  });
});

describe('effectiveMode', () => {
  it.each([
    [null, 'modified', 'diff'],
    [null, 'added', 'diff'],
    ['file', 'modified', 'file'],
    ['diff', 'modified', 'diff'],
    ['file', 'added', 'file'],
    [null, null, 'file'],
    ['diff', null, 'file'],
    [null, 'deleted', 'diff'],
    ['file', 'deleted', 'diff'],
  ] as const)('with %s chosen for a file that is %s shows the %s', (chosen, change, mode) => {
    expect(effectiveMode(chosen, change)).toBe(mode);
  });
});

describe('modeAvailable', () => {
  it.each([
    ['file', null, true],
    ['diff', null, false],
    ['file', 'modified', true],
    ['diff', 'modified', true],
    ['file', 'added', true],
    ['diff', 'added', true],
    ['file', 'deleted', false],
    ['diff', 'deleted', true],
  ] as const)('%s for a file that is %s: %s', (mode, change, available) => {
    expect(modeAvailable(mode, change)).toBe(available);
  });
});

describe('shownLines', () => {
  const lines = [line(null, 1, 'one'), line('removed', null, 'two'), line('added', 2, 'TWO'), line(null, 3, 'three')];

  it('keeps every line and its mark when showing changes', () => {
    expect(shownLines(lines, 'diff')).toEqual(lines);
  });

  it('shows the file as it is, unmarked, otherwise', () => {
    expect(shownLines(lines, 'file')).toEqual([line(null, 1, 'one'), line(null, 2, 'TWO'), line(null, 3, 'three')]);
  });

  it('has nothing to show of a deleted file as it is', () => {
    expect(shownLines([line('removed', null, 'gone')], 'file')).toEqual([]);
  });
});

describe('lineCountText', () => {
  it('counts the lines of the file as it is now', () => {
    expect(lineCountText([line(null, 1, 'one'), line('removed', null, 'two'), line('added', 2, 'TWO')])).toBe('2 lines');
  });

  it('says "1 line" for one', () => {
    expect(lineCountText([line(null, 1, 'one')])).toBe('1 line');
  });

  it('says "0 lines" for none', () => {
    expect(lineCountText([])).toBe('0 lines');
    expect(lineCountText([line('removed', null, 'gone')])).toBe('0 lines');
  });
});

describe('languageOf', () => {
  it.each([
    ['src/ledger/adapter.ts', 'TypeScript'],
    ['api/src/main.rs', 'Rust'],
    ['README.MD', 'Markdown'],
    ['a.b/archive.tar.yml', 'YAML'],
    ['Dockerfile', 'Dockerfile'],
    ['tools/Makefile', 'Makefile'],
    ['LICENSE', ''],
    ['.gitignore', ''],
    ['notes.unknown', ''],
    ['folder.ts/plain', ''],
  ])('of %s is "%s"', (path, language) => {
    expect(languageOf(path)).toBe(language);
  });
});

describe('liveText', () => {
  it.each([
    [0, ''],
    [-1, ''],
    [1, '1 agent live'],
    [2, '2 agents live'],
  ])('for %i running is "%s"', (running, text) => {
    expect(liveText(running)).toBe(text);
  });
});

describe('directoryName', () => {
  it.each([
    ['/home/someone/Projects/teryon', 'teryon'],
    ['/home/someone/Projects/teryon/', 'teryon'],
    ['teryon', 'teryon'],
    ['/', '/'],
  ])('of %s is %s', (directory, name) => {
    expect(directoryName(directory)).toBe(name);
  });
});

describe('askDraft', () => {
  it('starts a question about the file in an empty box', () => {
    expect(askDraft('', 'src/app.ts')).toBe('About `src/app.ts`: ');
    expect(askDraft('   ', 'src/app.ts')).toBe('About `src/app.ts`: ');
  });

  it('adds the file after what was already typed', () => {
    expect(askDraft('Why does this fail in ', 'src/app.ts')).toBe('Why does this fail in `src/app.ts` ');
  });
});

describe('isFilesShortcut', () => {
  it('is a bare C, in either case', () => {
    expect(isFilesShortcut(press())).toBe(true);
    expect(isFilesShortcut(press({ key: 'C' }))).toBe(true);
  });

  it('is no other key', () => {
    expect(isFilesShortcut(press({ key: 'x' }))).toBe(false);
    expect(isFilesShortcut(press({ key: 'Control' }))).toBe(false);
  });

  it.each(['ctrlKey', 'metaKey', 'altKey'] as const)('is not C held with %s, which belongs to the browser', (modifier) => {
    expect(isFilesShortcut(press({ [modifier]: true }))).toBe(false);
  });

  it.each(['INPUT', 'TEXTAREA', 'SELECT', 'textarea'])('is not a C typed into a %s', (element) => {
    expect(isFilesShortcut(press({ element }))).toBe(false);
  });

  it('is not a C typed into text edited in place', () => {
    expect(isFilesShortcut(press({ element: 'DIV', editable: true }))).toBe(false);
  });

  it('is a C pressed on a button', () => {
    expect(isFilesShortcut(press({ element: 'BUTTON' }))).toBe(true);
  });
});

describe('sameAnswer', () => {
  it('is true for answers that say the same', () => {
    expect(sameAnswer({ files: [file('a.txt')] }, { files: [file('a.txt')] })).toBe(true);
  });

  it('is false once anything differs', () => {
    expect(sameAnswer({ files: [file('a.txt')] }, { files: [file('a.txt', 'modified')] })).toBe(false);
    expect(sameAnswer(null, { files: [] })).toBe(false);
  });
});
