import { firstValueFrom } from 'rxjs';
import { FilesMemory } from './files.memory';
import { NOTHING_REMEMBERED, type Remembered } from './files.model';

function now(memory: FilesMemory, place: string): Promise<Remembered> {
  return firstValueFrom(memory.remembered(place));
}

describe('FilesMemory', () => {
  let memory: FilesMemory;

  beforeEach(() => {
    memory = new FilesMemory();
  });

  it('remembers nothing of a place it has not seen', async () => {
    expect(await now(memory, 'main')).toBe(NOTHING_REMEMBERED);
  });

  it('remembers the file opened', async () => {
    memory.choose('main', 'src/a.ts');
    expect(await now(memory, 'main')).toEqual({ chosen: 'src/a.ts', toggled: new Map() });
  });

  it('remembers only the last file opened', async () => {
    memory.choose('main', 'src/a.ts');
    memory.choose('main', 'src/b.ts');
    expect((await now(memory, 'main')).chosen).toBe('src/b.ts');
  });

  it('remembers folders opened and closed, the last time counting', async () => {
    memory.toggle('main', 'src', true);
    memory.toggle('main', 'docs', true);
    memory.toggle('main', 'docs', false);
    expect((await now(memory, 'main')).toggled).toEqual(
      new Map([
        ['src', true],
        ['docs', false],
      ]),
    );
  });

  it('keeps the file when a folder is toggled, and the folders when a file is opened', async () => {
    memory.choose('main', 'src/a.ts');
    memory.toggle('main', 'src', true);
    memory.choose('main', 'src/b.ts');
    expect(await now(memory, 'main')).toEqual({ chosen: 'src/b.ts', toggled: new Map([['src', true]]) });
  });

  it('keeps each place apart', async () => {
    memory.choose('main', 'src/a.ts');
    memory.toggle('fix-login', 'docs', true);
    expect(await now(memory, 'main')).toEqual({ chosen: 'src/a.ts', toggled: new Map() });
    expect(await now(memory, 'fix-login')).toEqual({ chosen: null, toggled: new Map([['docs', true]]) });
  });

  it('tells those watching a place of its changes, and not of changes to other places', () => {
    const seen: Remembered[] = [];
    const watching = memory.remembered('main').subscribe((remembered) => seen.push(remembered));
    memory.toggle('fix-login', 'docs', true);
    memory.choose('main', 'src/a.ts');
    watching.unsubscribe();
    expect(seen).toEqual([NOTHING_REMEMBERED, { chosen: 'src/a.ts', toggled: new Map() }]);
  });

  it('does not change what was remembered before', async () => {
    memory.toggle('main', 'src', true);
    const before = await now(memory, 'main');
    memory.toggle('main', 'docs', true);
    expect(before.toggled).toEqual(new Map([['src', true]]));
  });
});
