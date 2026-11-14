/**
 * The shapes the file browser works with, and the plain logic around them.
 *
 * Kept free of Angular and Ionic for the same reason as `workspace.model.ts`:
 * the screen itself cannot be unit tested until Vitest can load Ionic, so
 * everything worth testing lives here.
 */

export type FileChange = 'added' | 'modified' | 'deleted';

export interface FileEntry {
  /** Where the file is, from the top of the repository, with `/` between folders. */
  readonly path: string;
  readonly change: FileChange | null;
}

/** Every file of a branch or a commit, as the server lists them. */
export interface Listing {
  readonly branches: readonly string[];
  /** The branch shown. Nothing when the project's directory is not on a branch, and when a commit is shown. */
  readonly branch: string | null;
  /** The commit shown, by its full name. Nothing unless a commit was asked for. */
  readonly commit: string | null;
  /** The branch changes are counted from. Nothing when only uncommitted work counts. */
  readonly base: string | null;
  readonly files: readonly FileEntry[];
}

export interface FileLine {
  readonly change: 'added' | 'removed' | null;
  /** Where the line is in the file as it is now. A removed line has no place. */
  readonly number: number | null;
  readonly text: string;
}

export type FileContent =
  | { readonly kind: 'text'; readonly lines: readonly FileLine[] }
  | { readonly kind: 'binary' }
  | { readonly kind: 'too_large' };

export interface FileView {
  readonly path: string;
  readonly content: FileContent;
}

/**
 * Every language the file browser can tell by a file's name.
 *
 * Spelled out rather than left as any text so that the compiler can check
 * that each of them has colouring rules in `highlight.ts`.
 */
export type LanguageName =
  | 'C'
  | 'C++'
  | 'C#'
  | 'CSS'
  | 'Dockerfile'
  | 'Go'
  | 'HTML'
  | 'Java'
  | 'JavaScript'
  | 'JSON'
  | 'Kotlin'
  | 'Makefile'
  | 'Markdown'
  | 'PHP'
  | 'Plain text'
  | 'Python'
  | 'Ruby'
  | 'Rust'
  | 'SCSS'
  | 'Shell'
  | 'SQL'
  | 'SVG'
  | 'Swift'
  | 'TOML'
  | 'TypeScript'
  | 'XML'
  | 'YAML';

/** The two ways a file is shown: as it is, or with its changes. */
export type ViewMode = 'file' | 'diff';

/** One row of the file tree, ready to show. */
export interface TreeRow {
  readonly kind: 'folder' | 'file';
  readonly path: string;
  readonly name: string;
  /** How many folders the row sits inside. */
  readonly depth: number;
  /** Whether a folder shows what it holds. Always false for a file. */
  readonly open: boolean;
  /** What happened to a file. Always nothing for a folder. */
  readonly change: FileChange | null;
  /** Whether a folder holds a changed file. Always false for a file. */
  readonly changed: boolean;
}

/** A folder while the tree is being put together. */
interface Folder {
  readonly folders: Map<string, Folder>;
  readonly files: { readonly name: string; readonly entry: FileEntry }[];
  changed: boolean;
}

/** How far each level of the tree is moved to the right, in pixels. */
const INDENT = 14;
/** The space before a row at the top level, in pixels. */
const FIRST_INDENT = 8;

const LANGUAGES: Readonly<Record<string, LanguageName>> = {
  c: 'C',
  cc: 'C++',
  cpp: 'C++',
  cs: 'C#',
  css: 'CSS',
  go: 'Go',
  h: 'C',
  html: 'HTML',
  java: 'Java',
  js: 'JavaScript',
  json: 'JSON',
  jsx: 'JavaScript',
  kt: 'Kotlin',
  md: 'Markdown',
  mjs: 'JavaScript',
  php: 'PHP',
  py: 'Python',
  rb: 'Ruby',
  rs: 'Rust',
  scss: 'SCSS',
  sh: 'Shell',
  sql: 'SQL',
  svg: 'SVG',
  swift: 'Swift',
  toml: 'TOML',
  ts: 'TypeScript',
  tsx: 'TypeScript',
  txt: 'Plain text',
  xml: 'XML',
  yaml: 'YAML',
  yml: 'YAML',
};

/**
 * The rows of the file tree: folders before files, each level by name.
 *
 * A folder starts out open when it holds a change or the selected file, so
 * what an agent touched is in sight without any clicking, and everything
 * else stays out of the way. While a filter is typed every folder that still
 * holds something is open. A folder the person opened or closed themselves
 * stays as they left it.
 */
export function treeRows(
  files: readonly FileEntry[],
  filter: string,
  toggled: ReadonlyMap<string, boolean>,
  selected: string | null,
): TreeRow[] {
  const wanted = filter.trim().toLowerCase();
  const top: Folder = { folders: new Map(), files: [], changed: false };

  for (const entry of files) {
    if (wanted && !entry.path.toLowerCase().includes(wanted)) continue;
    const parts = entry.path.split('/');
    const name = parts.pop() ?? entry.path;
    let folder = top;
    for (const part of parts) {
      let next = folder.folders.get(part);
      if (!next) {
        next = { folders: new Map(), files: [], changed: false };
        folder.folders.set(part, next);
      }
      next.changed ||= entry.change !== null;
      folder = next;
    }
    folder.files.push({ name, entry });
  }

  const rows: TreeRow[] = [];
  const walk = (folder: Folder, prefix: string, depth: number): void => {
    for (const [name, inner] of [...folder.folders].sort(([a], [b]) => byName(a, b))) {
      const path = prefix + name;
      const holdsSelected = selected?.startsWith(`${path}/`) ?? false;
      const open = toggled.get(path) ?? (wanted !== '' || inner.changed || holdsSelected);
      rows.push({ kind: 'folder', path, name, depth, open, change: null, changed: inner.changed });
      if (open) walk(inner, `${path}/`, depth + 1);
    }
    for (const { name, entry } of [...folder.files].sort((a, b) => byName(a.name, b.name))) {
      rows.push({ kind: 'file', path: entry.path, name, depth, open: false, change: entry.change, changed: false });
    }
  };
  walk(top, '', 0);
  return rows;
}

/** How far a row starts from the left, in pixels. A file lines up with the names of the folders beside it. */
export function rowIndent(row: TreeRow): number {
  return FIRST_INDENT + INDENT * row.depth + (row.kind === 'file' ? INDENT : 0);
}

/**
 * The file to show: the one the person chose, or else the first changed one.
 *
 * A choice can outlive its file, because an agent can remove it or the branch
 * can change. Falling back to the first change means the screen opens on what
 * an agent did.
 */
export function selectedEntry(files: readonly FileEntry[], chosen: string | null): FileEntry | null {
  return files.find((entry) => entry.path === chosen) ?? files.find((entry) => entry.change !== null) ?? null;
}

/** The folders of a path as `src / ledger /`, and the file's own name. */
export function splitPath(path: string): { readonly folders: string; readonly name: string } {
  const parts = path.split('/');
  const name = parts.pop() ?? path;
  return { folders: parts.map((part) => `${part} /`).join(' '), name };
}

/** The letter shown beside a changed file. */
export function changeLetter(change: FileChange | null): string {
  return change ? change[0].toUpperCase() : '';
}

/** For example `modified since main`, or `added, not committed` when only uncommitted work counts. */
export function changeTag(change: FileChange | null, base: string | null): string {
  if (!change) return '';
  return base ? `${change} since ${base}` : `${change}, not committed`;
}

/**
 * How a file is shown, given what the person asked for.
 *
 * An unchanged file has no changes to show and a deleted file has nothing
 * else, so the choice only counts in between. A changed file opens on its
 * changes.
 */
export function effectiveMode(chosen: ViewMode | null, change: FileChange | null): ViewMode {
  if (change === null) return 'file';
  if (change === 'deleted') return 'diff';
  return chosen ?? 'diff';
}

/** Whether the person can switch to a way of showing the file. */
export function modeAvailable(mode: ViewMode, change: FileChange | null): boolean {
  return mode === 'file' ? change !== 'deleted' : change !== null;
}

/** The lines to show: all of them with their marks, or the file as it is, unmarked. */
export function shownLines<Line extends FileLine>(lines: readonly Line[], mode: ViewMode): Line[] {
  if (mode === 'diff') return [...lines];
  return lines.filter((line) => line.change !== 'removed').map((line) => ({ ...line, change: null }));
}

/** For example `17 lines`, counting the file as it is now. */
export function lineCountText(lines: readonly FileLine[]): string {
  const count = lines.filter((line) => line.change !== 'removed').length;
  return count === 1 ? '1 line' : `${count} lines`;
}

/** The language a file is written in, judged by its name. Nothing when the name does not say. */
export function languageOf(path: string): LanguageName | '' {
  const name = (path.split('/').pop() ?? '').toLowerCase();
  if (name === 'dockerfile') return 'Dockerfile';
  if (name === 'makefile') return 'Makefile';
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return '';
  return LANGUAGES[name.slice(dot + 1)] ?? '';
}

/** For example `2 agents live`. Nothing when no agent is running. */
export function liveText(running: number): string {
  if (running <= 0) return '';
  return running === 1 ? '1 agent live' : `${running} agents live`;
}

/** The last part of a directory, which is how a repository is usually known. */
export function directoryName(directory: string): string {
  return directory.split('/').filter(Boolean).pop() ?? directory;
}

/** What "Ask about this file" puts in the message box, after anything already typed there. */
export function askDraft(draft: string, path: string): string {
  const typed = draft.trim();
  return typed ? `${typed} \`${path}\` ` : `About \`${path}\`: `;
}

/** Whether two answers from the server say the same, so the screen need not redraw. */
export function sameAnswer(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function byName(a: string, b: string): number {
  return a.localeCompare(b, 'en', { sensitivity: 'base' }) || a.localeCompare(b, 'en');
}
