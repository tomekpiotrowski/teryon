/**
 * The shapes the history screen works with, and the plain logic around them.
 *
 * Kept free of Angular and Ionic for the same reason as `workspace.model.ts`:
 * the screen itself cannot be unit tested until Vitest can load Ionic, so
 * everything worth testing lives here.
 */

import type { FileChange, FileLine } from './files.model';
import { agentLabel, isBareLetter, type KeyPress, type Thread, threadLabel } from './workspace.model';

/** One commit of a branch, as the server lists it. */
export interface Commit {
  /** The commit's full name. */
  readonly hash: string;
  /** The commits this one was made on top of, the one it continues first. */
  readonly parents: readonly string[];
  /** The first line of the message. */
  readonly subject: string;
  /** The name of the person the commit is recorded under. */
  readonly author: string;
  readonly email: string;
  /** When the commit came to be, in seconds since 1970. */
  readonly time: number;
  /** Whether the main branch has this commit too. */
  readonly on_base: boolean;
  /** The thread whose agent was working when the commit was made. Nothing for a commit made by hand. */
  readonly thread: string | null;
  /** The files the commit changed, each from the top of the repository. */
  readonly files: readonly string[];
}

/** The commits of a branch, a branch's own first and newest first within that. */
export interface History {
  readonly branches: readonly string[];
  /** The branch shown. Nothing when the project's directory is not on a branch. */
  readonly branch: string | null;
  /** The main branch, when the branch shown is another one. */
  readonly base: string | null;
  /** How many commits the branch shown has that the main branch has not. */
  readonly ahead: number;
  /** Whether the branch has older commits than the ones listed. */
  readonly more: boolean;
  readonly commits: readonly Commit[];
}

/** One file a commit changed. */
export interface ChangedFile {
  readonly path: string;
  readonly change: FileChange;
  /** How many lines were added and removed. Nothing for a file that is not text. */
  readonly added: number | null;
  readonly removed: number | null;
}

/** What a commit changed, or what changed between two commits. */
export interface Changes {
  readonly commit: string;
  /** The commit it is compared with. Nothing for a commit without a parent. */
  readonly from: string | null;
  /** The commit's message after its first line. */
  readonly body: string;
  readonly files: readonly ChangedFile[];
}

/** Whose commits are listed. */
export type Maker = 'all' | 'agents' | 'people';

/** The two ways a change is laid out: old and new lines in one column, or side by side. */
export type DiffMode = 'unified' | 'split';

/** Who made a commit, as the screen names them. */
export interface Who {
  readonly kind: 'agent' | 'person';
  readonly name: string;
  /** The thread an agent's commit came from. Nothing for a person, and for a thread this browser does not hold. */
  readonly thread: Thread | null;
}

/** The two lines of the picture beside the commits: the branch's own commits, and the main branch's. */
export type Lane = 'own' | 'base';

/** How much of a lane's line crosses a row: none, from the dot down, all of it, or down to the dot. */
export type LaneLine = 'none' | 'below' | 'through' | 'above';

/** What is drawn beside one commit. */
export interface GraphRow {
  /** The lane the commit's dot sits on. */
  readonly lane: Lane;
  readonly own: LaneLine;
  readonly base: LaneLine;
  /** A line between the lanes: where the main branch was merged in, or where the branch left it. */
  readonly joint: 'none' | 'merge' | 'fork';
}

/** One row of the list of changed files. */
export type ChangeRow =
  | { readonly kind: 'folder'; readonly path: string; readonly open: boolean }
  | {
      readonly kind: 'file';
      readonly path: string;
      readonly name: string;
      /** Whether the file sits under a folder row. */
      readonly nested: boolean;
      readonly change: FileChange;
      readonly counts: string;
    };

/** One row of a change laid out in one column. */
export type DiffRow<Line extends FileLine> =
  | { readonly kind: 'hunk'; readonly text: string }
  | {
      readonly kind: 'line';
      readonly line: Line;
      /** Where the line was before the change. An added line had no place. */
      readonly before: number | null;
      /** Where the line is after the change. A removed line has no place. */
      readonly after: number | null;
    };

/** One side of a row of a change laid out side by side. */
export interface SplitSide<Line extends FileLine> {
  readonly line: Line;
  readonly number: number | null;
}

/** One row of a change laid out side by side: the old line on the left, the new one on the right. */
export type SplitRow<Line extends FileLine> =
  | { readonly kind: 'hunk'; readonly text: string }
  | { readonly kind: 'pair'; readonly before: SplitSide<Line> | null; readonly after: SplitSide<Line> | null };

/** How many unchanged lines are shown around a change. */
export const CONTEXT = 3;

/** How many digits of a commit's name are shown. */
const SHORT = 7;

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** The start of a commit's name, which is how people refer to a commit. */
export function shortHash(hash: string): string {
  return hash.slice(0, SHORT);
}

/**
 * Who made a commit.
 *
 * A commit the server tied to a thread is that thread's agent's. Any other
 * is a person's: `you` when it is recorded under the address of whoever is
 * signed in, and the recorded name otherwise.
 */
export function whoMade(commit: Commit, threads: readonly Thread[], ownEmail: string | null): Who {
  if (commit.thread !== null) {
    const thread = threads.find((candidate) => candidate.id === commit.thread) ?? null;
    return { kind: 'agent', name: thread ? agentLabel(thread.agent) : 'Agent', thread };
  }
  const address = commit.email.trim().toLowerCase();
  const own = address !== '' && address === (ownEmail ?? '').trim().toLowerCase();
  return { kind: 'person', name: own ? 'you' : commit.author || commit.email || 'unknown', thread: null };
}

/**
 * Whether a commit stays in the list.
 *
 * The typed text is looked for in the message's first line, in who made the
 * commit, in the title of the thread it came from and in the files it
 * changed. A commit's name only counts from its start, because its digits
 * would otherwise match short words such as `add` by chance.
 */
export function matchesFilter(commit: Commit, who: Who, filter: string, maker: Maker): boolean {
  if (maker === 'agents' && who.kind !== 'agent') return false;
  if (maker === 'people' && who.kind !== 'person') return false;
  const wanted = filter.trim().toLowerCase();
  if (!wanted) return true;
  if (commit.hash.toLowerCase().startsWith(wanted)) return true;
  const texts = [commit.subject, commit.author, who.name, who.thread ? threadLabel(who.thread) : '', ...commit.files];
  return texts.some((text) => text.toLowerCase().includes(wanted));
}

/**
 * The picture beside the commits: two lanes, dots, and the lines between them.
 *
 * The left lane holds the branch's own commits and the right lane the ones
 * the main branch has too. The server lists a branch's own commits first, so
 * the left lane runs from the top down to where the branch left the main
 * branch, and the right lane from the first place the two meet to the bottom.
 * A real history can have any number of lines side by side; two are enough
 * to tell a branch's work from what it started from and what it merged in.
 *
 * `more` says that older commits follow the listed ones, in which case a
 * lane that has not ended runs off the bottom.
 */
export function graphRows(commits: readonly Commit[], more: boolean): GraphRow[] {
  const rowOf = new Map(commits.map((commit, row) => [commit.hash, row]));
  const baseParent = (hash: string): number => {
    const row = rowOf.get(hash);
    return row !== undefined && commits[row].on_base ? row : -1;
  };

  const last = commits.length - 1;
  let lastOwn = -1;
  let firstBase = -1;
  let firstMerge = -1;
  let fork = -1;
  const merges = new Set<number>();
  commits.forEach((commit, row) => {
    if (commit.on_base) {
      if (firstBase < 0) firstBase = row;
      return;
    }
    lastOwn = row;
    commit.parents.forEach((parent, nth) => {
      const at = baseParent(parent);
      if (at < 0) return;
      if (nth === 0) {
        fork = Math.max(fork, at);
      } else {
        merges.add(row);
        if (firstMerge < 0) firstMerge = row;
      }
    });
  });

  // Where each lane's line starts and ends. A lane that runs off the bottom ends after the last row.
  const ownStart = lastOwn < 0 ? -1 : 0;
  const ownEnd = fork >= 0 ? fork : more ? last + 1 : lastOwn;
  const baseStart = firstMerge >= 0 ? firstMerge : firstBase;
  const baseEnd = more ? last + 1 : last;

  return commits.map((commit, row) => ({
    lane: commit.on_base ? 'base' : 'own',
    own: laneLine(row, ownStart, ownEnd),
    base: laneLine(row, baseStart, baseEnd),
    joint: row === fork ? 'fork' : merges.has(row) ? 'merge' : 'none',
  }));
}

/** The picture beside commits picked out by a filter: dots only, because their neighbours are missing. */
export function looseRows(commits: readonly Commit[]): GraphRow[] {
  return commits.map((commit) => ({ lane: commit.on_base ? 'base' : 'own', own: 'none', base: 'none', joint: 'none' }));
}

/** For example `7 commits`, `2 of 7 commits`, or `newest 200 commits` when there are older ones. */
export function commitCountText(shown: number, listed: number, more: boolean): string {
  const noun = listed === 1 ? 'commit' : 'commits';
  if (shown !== listed) return `${shown} of ${more ? 'newest ' : ''}${listed} ${noun}`;
  return more ? `newest ${listed} ${noun}` : `${listed} ${noun}`;
}

/** For example `4 ahead of main`. Nothing when there is no main branch to be ahead of. */
export function aheadText(base: string | null, ahead: number): string {
  return base ? `${ahead} ahead of ${base}` : '';
}

/**
 * When a commit was made, as short as the list needs it.
 *
 * The hour for today, `yesterday`, the day for this year and the day with
 * its year before that. Both moments are in seconds since 1970.
 */
export function timeText(time: number, now: number): string {
  const made = new Date(time * 1000);
  const today = new Date(now * 1000);
  const days = Math.round((startOfDay(today) - startOfDay(made)) / 86_400_000);
  if (days === 0) return clockText(made);
  if (days === 1) return 'yesterday';
  const day = `${made.getDate()} ${MONTHS[made.getMonth()]}`;
  return made.getFullYear() === today.getFullYear() ? day : `${day} ${made.getFullYear()}`;
}

/** When a commit was made, in full: `today 09:22`, `yesterday 08:31`, `3 Nov 08:31`, `3 Nov 2025 08:31`. */
export function momentText(time: number, now: number): string {
  const short = timeText(time, now);
  const made = new Date(time * 1000);
  const clock = clockText(made);
  return short === clock ? `today ${clock}` : `${short} ${clock}`;
}

/**
 * The commit shown and the one it is compared with.
 *
 * A choice can outlive its commit, because a branch can be rebased or the
 * filter can hide it, so the first listed commit stands in. Two commits are
 * compared from the older to the newer, whichever was picked first. A commit
 * is not compared with itself.
 */
export function chosenCommits(
  shown: readonly Commit[],
  chosen: string | null,
  compared: string | null,
): { readonly commit: Commit | null; readonly from: Commit | null } {
  const chosenAt = shown.findIndex((commit) => commit.hash === chosen);
  const at = chosenAt < 0 ? 0 : chosenAt;
  const commit = shown[at] ?? null;
  const otherAt = chosenAt < 0 ? -1 : shown.findIndex((candidate) => candidate.hash === compared);
  if (!commit || otherAt < 0 || otherAt === at) return { commit, from: null };
  // The list runs from new to old, so the later row is the older commit.
  return at < otherAt ? { commit, from: shown[otherAt] } : { commit: shown[otherAt], from: commit };
}

/** The commit a step up or down the list leads to. Stays on the first or last one at the ends. */
export function neighbour(shown: readonly Commit[], current: string | null, step: number): string | null {
  if (!shown.length) return null;
  const at = shown.findIndex((commit) => commit.hash === current);
  const next = at < 0 ? 0 : Math.min(shown.length - 1, Math.max(0, at + step));
  return shown[next].hash;
}

/** How far a key press moves through the commits: `J` one down, `K` one up, any other key not at all. */
export function moveStep(press: KeyPress): number {
  if (isBareLetter(press, 'j')) return 1;
  if (isBareLetter(press, 'k')) return -1;
  return 0;
}

/**
 * The rows of the list of changed files: each folder once, with its files under it.
 *
 * A folder is named by its whole path on one row, because a commit touches
 * few files in few places and a tree level by level would mostly be folders.
 * Files at the top of the repository come first, without a folder row.
 */
export function changeRows(files: readonly ChangedFile[], closed: ReadonlySet<string>): ChangeRow[] {
  const folders = new Map<string, ChangedFile[]>();
  for (const file of files) {
    const folder = folderOf(file.path);
    folders.set(folder, [...(folders.get(folder) ?? []), file]);
  }

  const rows: ChangeRow[] = [];
  for (const folder of [...folders.keys()].sort()) {
    const open = !closed.has(folder);
    if (folder) rows.push({ kind: 'folder', path: folder, open });
    if (folder && !open) continue;
    for (const file of folders.get(folder) ?? []) {
      rows.push({
        kind: 'file',
        path: file.path,
        name: file.path.slice(folder ? folder.length + 1 : 0),
        nested: folder !== '',
        change: file.change,
        counts: countsText(file),
      });
    }
  }
  return rows;
}

/** The changed file to show: the one the person chose, or else the first one. */
export function selectedChange(files: readonly ChangedFile[], chosen: string | null): ChangedFile | null {
  return files.find((file) => file.path === chosen) ?? files[0] ?? null;
}

/** For example `+1 −1`, or `+3` when nothing was removed. Nothing for a file that is not text. */
export function countsText(file: ChangedFile): string {
  const parts: string[] = [];
  if (file.added) parts.push(`+${file.added}`);
  if (file.removed) parts.push(`−${file.removed}`);
  return parts.join(' ');
}

/**
 * A changed file cut down to its changes, each with a few unchanged lines around it.
 *
 * The server sends the whole file with its changes marked. Showing all of it
 * would bury a small change in a long file, so only the lines within
 * `context` of a change are kept, and each stretch starts with the row git
 * would print for it: `@@ -7,4 +7,4 @@`, the place and length before and
 * after the change. A file without a changed line has no rows.
 */
export function diffRows<Line extends FileLine>(lines: readonly Line[], context = CONTEXT): DiffRow<Line>[] {
  let before = 0;
  const numbered = lines.map((line) => {
    if (line.change !== 'added') before += 1;
    return { line, before: line.change === 'added' ? null : before, after: line.number };
  });

  const stretches: { start: number; end: number }[] = [];
  numbered.forEach(({ line }, at) => {
    if (line.change === null) return;
    const start = Math.max(0, at - context);
    const end = Math.min(numbered.length - 1, at + context);
    const previous = stretches.at(-1);
    if (previous && start <= previous.end + 1) previous.end = end;
    else stretches.push({ start, end });
  });

  const rows: DiffRow<Line>[] = [];
  for (const { start, end } of stretches) {
    const stretch = numbered.slice(start, end + 1);
    const earlier = numbered.slice(0, start);
    const old = stretch.filter((row) => row.before !== null);
    const current = stretch.filter((row) => row.after !== null);
    // An empty side is placed after the last line that came before it, as git does.
    const oldStart = old[0]?.before ?? earlier.filter((row) => row.before !== null).length;
    const currentStart = current[0]?.after ?? earlier.filter((row) => row.after !== null).length;
    rows.push({ kind: 'hunk', text: `@@ -${oldStart},${old.length} +${currentStart},${current.length} @@` });
    for (const row of stretch) rows.push({ kind: 'line', ...row });
  }
  return rows;
}

/**
 * A change laid out side by side.
 *
 * Removed lines go on the left and the added lines that follow them on the
 * right, row by row, so a rewritten line faces what it replaced. An unchanged
 * line is on both sides.
 */
export function splitRows<Line extends FileLine>(rows: readonly DiffRow<Line>[]): SplitRow<Line>[] {
  const split: SplitRow<Line>[] = [];
  let removed: SplitSide<Line>[] = [];
  let added: SplitSide<Line>[] = [];
  const flush = (): void => {
    for (let at = 0; at < Math.max(removed.length, added.length); at += 1) {
      split.push({ kind: 'pair', before: removed[at] ?? null, after: added[at] ?? null });
    }
    removed = [];
    added = [];
  };

  for (const row of rows) {
    if (row.kind === 'hunk') {
      flush();
      split.push(row);
    } else if (row.line.change === 'removed') {
      if (added.length) flush();
      removed.push({ line: row.line, number: row.before });
    } else if (row.line.change === 'added') {
      added.push({ line: row.line, number: row.after });
    } else {
      flush();
      split.push({
        kind: 'pair',
        before: { line: row.line, number: row.before },
        after: { line: row.line, number: row.after },
      });
    }
  }
  flush();
  return split;
}

/** What "Ask to revert" puts in the message box, after anything already typed there. */
export function revertDraft(draft: string, commit: Commit): string {
  const typed = draft.trim();
  const request = `Revert commit \`${shortHash(commit.hash)}\` (${commit.subject}). `;
  return typed ? `${typed} ${request}` : request;
}

function laneLine(row: number, start: number, end: number): LaneLine {
  if (start < 0 || start >= end || row < start || row > end) return 'none';
  if (row === start) return 'below';
  return row === end ? 'above' : 'through';
}

function folderOf(path: string): string {
  const slash = path.lastIndexOf('/');
  return slash < 0 ? '' : path.slice(0, slash);
}

function startOfDay(moment: Date): number {
  return new Date(moment.getFullYear(), moment.getMonth(), moment.getDate()).getTime();
}

function clockText(moment: Date): string {
  return `${String(moment.getHours()).padStart(2, '0')}:${String(moment.getMinutes()).padStart(2, '0')}`;
}
