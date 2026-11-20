import {
  adopted,
  carriedListing,
  childPath,
  directoryBaseName,
  directoryEnter,
  type HeldListing,
  keptName,
  listingApplies,
  markedIndex,
  type DirectoryListing,
  pickerRows,
  pickerView,
  type PickerInput,
  shownPath,
} from './new-project.model';

function listing(overrides: Partial<DirectoryListing> = {}): DirectoryListing {
  return {
    directory: '/home/tomek/projects',
    parent: '/home/tomek',
    children: ['afolder', 'payments', 'website', '.hidden'],
    remainder: '',
    ...overrides,
  };
}

function input(overrides: Partial<PickerInput> = {}): PickerInput {
  return {
    path: '/home/tomek/projects',
    name: 'projects',
    marked: 0,
    followTyped: true,
    requested: '/home/tomek/projects',
    listing: listing(),
    notice: '',
    settled: true,
    ...overrides,
  };
}

describe('directoryBaseName', () => {
  it.each([
    ['/home/someone/Projects/teryon', 'teryon'],
    ['/home/someone/Projects/teryon/', 'teryon'],
    ['teryon', 'teryon'],
    ['/', '/'],
    ['', ''],
    ['C:\\Users\\tomek\\Projects', 'Projects'],
    ['C:\\Users\\tomek\\Projects\\', 'Projects'],
    ['C:\\', 'C:'],
  ])('of %s is %s', (directory, name) => {
    expect(directoryBaseName(directory)).toBe(name);
  });
});

describe('childPath', () => {
  it.each([
    ['/home/tomek', 'projects', '/home/tomek/projects'],
    ['/home/tomek/', 'projects', '/home/tomek/projects'],
    ['/', 'home', '/home'],
    ['\\', 'home', '/home'],
    ['C:\\Users', 'tomek', 'C:\\Users\\tomek'],
    ['C:\\Users\\', 'tomek', 'C:\\Users\\tomek'],
  ])('puts %s inside %s', (directory, name, path) => {
    expect(childPath(directory, name)).toBe(path);
  });
});

describe('pickerRows', () => {
  it('offers the parent and the visible children, in the order given', () => {
    expect(pickerRows(listing({ children: ['b', 'a', '.git'] }))).toEqual([
      { label: '..', path: '/home/tomek' },
      { label: 'b', path: '/home/tomek/projects/b' },
      { label: 'a', path: '/home/tomek/projects/a' },
    ]);
  });

  it('has no parent row at the top of the filesystem', () => {
    expect(pickerRows(listing({ directory: '/', parent: null, children: ['home'], remainder: '' }))).toEqual([
      { label: 'home', path: '/home' },
    ]);
  });

  it.each([
    ['pay', ['payments']],
    ['Pay', ['payments']],
    ['PAY', ['payments']],
    ['web', ['website']],
    ['nope', []],
    ['pay/ments', ['payments']],
  ])('narrows %s to %s', (remainder, names) => {
    expect(pickerRows(listing({ remainder })).map((row) => row.label)).toEqual(names);
  });

  it('shows hidden names, and not the others, once the ending starts with a dot', () => {
    expect(pickerRows(listing({ remainder: '.h' })).map((row) => row.label)).toEqual(['.hidden']);
  });

  it('offers the parent when the ending is the start of ..', () => {
    expect(pickerRows(listing({ remainder: '..' })).map((row) => row.label)).toEqual(['..']);
    expect(pickerRows(listing({ remainder: '.' })).map((row) => row.label)).toEqual(['..', '.hidden']);
  });
});

describe('markedIndex', () => {
  const rows = pickerRows(listing());

  it('is zero for an empty list', () => {
    expect(markedIndex(3, false, [], '')).toBe(0);
    expect(markedIndex(3, true, [], 'pay')).toBe(0);
  });

  it('marks an exact name while the letters decide, ignoring case', () => {
    const found = pickerRows(listing({ remainder: 'Website' }));
    expect(markedIndex(0, true, found, 'Website')).toBe(0);
    const several = pickerRows(listing({ remainder: 'a' }));
    expect(several.map((row) => row.label)).toEqual(['afolder']);
    expect(markedIndex(4, true, pickerRows(listing({ children: ['abc', 'ab'], remainder: 'ab' })), 'ab')).toBe(1);
  });

  it('marks the first row when nothing matches exactly', () => {
    expect(markedIndex(2, true, rows, '')).toBe(0);
    expect(markedIndex(2, true, pickerRows(listing({ remainder: 'p' })), 'p')).toBe(0);
  });

  it('keeps a mark from the arrows inside the list', () => {
    expect(markedIndex(2, false, rows, '')).toBe(2);
    expect(markedIndex(9, false, rows, '')).toBe(rows.length - 1);
    expect(markedIndex(-1, false, rows, '')).toBe(0);
  });
});

describe('shownPath', () => {
  const found = listing();

  it('leaves the field alone while a name is still being typed', () => {
    expect(shownPath('/home/tomek/projects/pay', listing({ remainder: 'pay' }))).toBeNull();
  });

  it.each(['/home/tomek/projects', '/home/tomek/projects/', '/home/tomek/projects//'])('keeps %s, which already names the directory', (typed) => {
    expect(shownPath(typed, found)).toBeNull();
  });

  it.each(['', '~', '~/projects', '/home/tomek/projects/../projects'])('shows the real directory for %s', (typed) => {
    expect(shownPath(typed, found)).toBe('/home/tomek/projects');
  });
});

describe('keptName', () => {
  it('follows the directory until a name is typed', () => {
    expect(keptName('', false, '/work/payments')).toEqual({ name: 'payments', custom: false });
    expect(keptName('work', false, '/work/payments')).toEqual({ name: 'payments', custom: false });
  });

  it('keeps a name the person typed', () => {
    expect(keptName('Custom', true, '/work/payments')).toEqual({ name: 'Custom', custom: true });
  });

  it('follows again once the typed name is cleared', () => {
    expect(keptName('', true, '/work/payments')).toEqual({ name: 'payments', custom: false });
    expect(keptName('   ', true, '/work/payments')).toEqual({ name: 'payments', custom: false });
  });
});

describe('carriedListing', () => {
  const pay: HeldListing = { requested: '/home/tomek/projects/pay', listing: listing({ remainder: 'pay' }) };

  it('needs a listing it already has', () => {
    expect(carriedListing('/home/tomek/projects/pay', null)).toBeNull();
  });

  it('keeps the children while the ending of a name changes', () => {
    expect(carriedListing('/home/tomek/projects/pays', pay)).toEqual(listing({ remainder: 'pays' }));
    expect(carriedListing('/home/tomek/projects/pa', pay)).toEqual(listing({ remainder: 'pa' }));
  });

  it('keeps an exact child as a filter until a slash or another part follows it', () => {
    expect(carriedListing('/home/tomek/projects/payments', pay)).toEqual(listing({ remainder: 'payments' }));
    expect(carriedListing('/home/tomek/projects/payments/', pay)).toBeNull();
    expect(carriedListing('/home/tomek/projects/payments/src', pay)).toBeNull();
  });

  it('keeps a name that is not a directory, including what is typed after it', () => {
    expect(carriedListing('/home/tomek/projects/nope/deeper', pay)).toEqual(listing({ remainder: 'nope/deeper' }));
  });

  it('keeps .. as a filter, and asks again once a path follows it', () => {
    expect(carriedListing('/home/tomek/projects/..', pay)).toEqual(listing({ remainder: '..' }));
    expect(carriedListing('/home/tomek/projects/../website', pay)).toBeNull();
  });

  it('asks again when the path leaves the directory', () => {
    expect(carriedListing('/var/log', pay)).toBeNull();
    expect(carriedListing('/home/tomek/projects-old', pay)).toBeNull();
  });

  it('follows a tilde the same way', () => {
    const tilde: HeldListing = { requested: '~/projects/pay', listing: listing({ remainder: 'pay' }) };
    expect(carriedListing('~/projects/pa', tilde)).toEqual(listing({ remainder: 'pa' }));
    expect(carriedListing('/home/tomek/projects', { requested: '~/projects', listing: listing() })).toEqual(listing());
  });

  it('treats the home listing as the start of a relative name', () => {
    const home = listing({ directory: '/home/tomek', parent: '/', children: ['projects', 'code'], remainder: '' });
    const held: HeldListing = { requested: '', listing: home };
    expect(carriedListing('proj', held)).toEqual({ ...home, remainder: 'proj' });
    expect(carriedListing('/home/tomek', held)).toEqual(home);
    expect(carriedListing('/usr', held)).toBeNull();
  });

  it('asks again when a path continues into a child of the top of the filesystem', () => {
    const root = listing({ directory: '/', parent: null, children: ['usr', 'home'], remainder: '' });
    const held: HeldListing = { requested: '/', listing: root };
    expect(carriedListing('/us', held)).toEqual({ ...root, remainder: 'us' });
    expect(carriedListing('/usr/bin', held)).toBeNull();
    expect(carriedListing('/..', held)).toEqual({ ...root, remainder: '..' });
  });

  it('asks again when the unfinished ending is not the end of what was asked', () => {
    const odd: HeldListing = { requested: '/home/tomek/projects', listing: listing({ remainder: 'nope' }) };
    expect(carriedListing('/home/tomek/projects/n', odd)).toBeNull();
  });

  it('keeps a windows path while its ending changes', () => {
    const folder = listing({ directory: 'C:\\Users\\tomek', parent: 'C:\\Users', children: ['projects'], remainder: 'pr' });
    expect(carriedListing('C:\\Users\\tomek\\pro', { requested: 'C:\\Users\\tomek\\pr', listing: folder })).toEqual({
      ...folder,
      remainder: 'pro',
    });
  });
});

describe('directoryEnter', () => {
  it('submits when the path is a directory', () => {
    expect(directoryEnter(pickerView(input()))).toBe('submit');
  });

  it('steps into the marked row while a name is being typed', () => {
    const view = pickerView(input({
      path: '/home/tomek/projects/pay',
      requested: '/home/tomek/projects/pay',
      listing: listing({ remainder: 'pay' }),
    }));
    expect(directoryEnter(view)).toBe('step');
  });

  it('stays when nothing matches', () => {
    const view = pickerView(input({
      path: '/home/tomek/projects/nope',
      requested: '/home/tomek/projects/nope',
      listing: listing({ remainder: 'nope' }),
    }));
    expect(view.rows).toEqual([]);
    expect(directoryEnter(view)).toBe('stay');
  });
});

describe('adopted', () => {
  const found = listing();

  it('ignores a listing for a path the field has left', () => {
    expect(adopted('/old', found, '/new', 'projects', false)).toBeNull();
  });

  it('ignores a missing listing', () => {
    expect(adopted('/home/tomek/projects', null, '/home/tomek/projects', 'projects', false)).toBeNull();
  });

  it('leaves the name alone while a name is still being typed', () => {
    expect(adopted('/home/tomek/projects/pay', listing({ remainder: 'pay' }), '/home/tomek/projects/pay', 'projects', false)).toBeNull();
  });

  it('names the project after the directory, without rewriting a path that already says it', () => {
    expect(adopted('/home/tomek/projects', found, '/home/tomek/projects', '', false)).toEqual({
      path: null,
      name: 'projects',
      customName: false,
      directory: '/home/tomek/projects',
    });
  });

  it('rewrites a path that names the directory another way', () => {
    expect(adopted('~/projects', found, '~/projects', '', false)?.path).toBe('/home/tomek/projects');
  });

  it('keeps a name the person typed', () => {
    expect(adopted('/home/tomek/projects', found, '/home/tomek/projects', 'Custom', true)).toEqual({
      path: null,
      name: 'Custom',
      customName: true,
      directory: '/home/tomek/projects',
    });
  });

  it('fills a cleared name from the directory', () => {
    expect(adopted('/home/tomek/projects', found, '  /home/tomek/projects  ', '  ', true)?.name).toBe('projects');
  });
});

describe('listingApplies', () => {
  const found = listing();

  it('needs a listing', () => {
    expect(listingApplies('/home/tomek/projects', '/home/tomek/projects', null)).toBe(false);
  });

  it('accepts the path that was asked for', () => {
    expect(listingApplies('~/projects', '~/projects', found)).toBe(true);
  });

  it.each([
    ['/home/tomek/projects', true],
    ['/home/tomek/projects/', true],
    ['/home/tomek/projects/payments', true],
    ['/home/tomek/projects-old', false],
    ['/var', false],
  ])('says %s belongs here: %s', (path, applies) => {
    expect(listingApplies(path, '/somewhere/else', found)).toBe(applies);
  });

  it('treats the top of the filesystem as containing everything under it', () => {
    const root = listing({ directory: '/', parent: null, children: ['home'] });
    expect(listingApplies('/usr', '/', root)).toBe(true);
    expect(listingApplies('/', '/', root)).toBe(true);
  });

  it('does not treat a backslash path as inside a shorter one', () => {
    const folder = listing({ directory: 'C:\\Users', parent: 'C:\\', children: ['tomek'] });
    expect(listingApplies('C:\\Users\\tomek', 'C:\\old', folder)).toBe(true);
    expect(listingApplies('C:\\Users2', 'C:\\old', folder)).toBe(false);
  });
});

describe('pickerView', () => {
  it('says it is looking until a listing arrives', () => {
    const view = pickerView(input({ settled: false, listing: null }));
    expect(view.rows).toEqual([]);
    expect(view.empty).toBe('Looking…');
    expect(view.notice).toBe('');
    expect(view.active).toBeNull();
  });

  it('lists the directory once the answer is for the field', () => {
    const view = pickerView(input());
    expect(view.rows.map((row) => row.label)).toEqual(['..', 'afolder', 'payments', 'website']);
    expect(view.empty).toBe('');
    expect(view.filtering).toBe(false);
    expect(view.marked).toBe(0);
    expect(view.active).toBe('directory-option-0');
  });

  it('does not call the parent a failed search when it is what was typed', () => {
    const view = pickerView(input({
      path: '/home/tomek/projects/..',
      requested: '/home/tomek/projects/..',
      listing: listing({ remainder: '..' }),
    }));
    expect(view.rows.map((row) => row.label)).toEqual(['..']);
    expect(view.empty).toBe('');
  });

  it('says when the directory is empty, and still offers its parent', () => {
    const view = pickerView(input({ path: '/empty', requested: '/empty', listing: listing({ directory: '/empty', parent: '/', children: [] }) }));
    expect(view.rows.map((row) => row.label)).toEqual(['..']);
    expect(view.empty).toBe('This directory is empty.');
  });

  it('says when nothing matches, and marks an exact name', () => {
    const missed = pickerView(input({ path: '/home/tomek/projects/nope', requested: '/home/tomek/projects/nope', listing: listing({ remainder: 'nope' }) }));
    expect(missed.rows).toEqual([]);
    expect(missed.empty).toBe('No directory matches.');
    expect(missed.filtering).toBe(true);

    const exact = pickerView(input({
      path: '/home/tomek/projects/website',
      requested: '/home/tomek/projects/website',
      listing: listing({ remainder: 'website' }),
      marked: 4,
    }));
    expect(exact.rows.map((row) => row.label)).toEqual(['website']);
    expect(exact.marked).toBe(0);
    expect(exact.filtering).toBe(true);
  });

  it('shows a refusal instead of an empty list', () => {
    const view = pickerView(input({ listing: null, notice: 'That is a file, not a directory.' }));
    expect(view.rows).toEqual([]);
    expect(view.empty).toBe('');
    expect(view.notice).toBe('That is a file, not a directory.');
    expect(view.filtering).toBe(false);
  });

  it('hides a refusal that arrived before the form was ready', () => {
    expect(pickerView(input({ settled: false, notice: 'Too soon.' })).notice).toBe('');
  });

  it('waits rather than showing children of a directory the field has left', () => {
    const view = pickerView(input({ path: '/var/log', requested: '/home/tomek/projects' }));
    expect(view.rows).toEqual([]);
    expect(view.empty).toBe('Looking…');
  });

  it('keeps a mark from the arrows', () => {
    const view = pickerView(input({ followTyped: false, marked: 2 }));
    expect(view.marked).toBe(2);
    expect(view.active).toBe('directory-option-2');
  });

  it('passes the path and the name through', () => {
    const view = pickerView(input({ path: '/work/payments/', name: 'Custom' }));
    expect(view.path).toBe('/work/payments/');
    expect(view.name).toBe('Custom');
  });
});
