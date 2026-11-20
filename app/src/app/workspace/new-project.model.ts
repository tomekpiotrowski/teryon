/**
 * Choosing a directory for a new project, as plain data.
 *
 * The screen lists the directories the service found and narrows them by
 * whatever was typed after the directory that exists. The name follows that
 * directory until the person types one of their own.
 *
 * Free of Angular, like `workspace.model.ts`, so the combinations can be
 * tested without a browser.
 */

/** The directories inside one directory, as the service lists them. */
export interface DirectoryListing {
  /** The real directory whose children these are. */
  readonly directory: string;
  /** That directory's parent. Nothing at the top of the filesystem. */
  readonly parent: string | null;
  /** Child directories, by name. Names starting with `.` are included. */
  readonly children: readonly string[];
  /** What was typed after `directory`. Empty when the typed path is this directory. */
  readonly remainder: string;
}

/** One row of the list: a child directory, or `..` for the parent. */
export interface PickerRow {
  readonly label: string;
  readonly path: string;
}

/** What to show after a listing arrives for the path in the field. */
export interface Adopted {
  /** The real directory, when the field should show it instead of what was typed. Nothing when the field already says it. */
  readonly path: string | null;
  readonly name: string;
  /** Whether the name is one the person typed, rather than the directory's own. */
  readonly customName: boolean;
  readonly directory: string;
}

/** Everything the new-project form draws at one moment. */
export interface PickerView {
  readonly path: string;
  readonly name: string;
  readonly rows: readonly PickerRow[];
  readonly marked: number;
  readonly notice: string;
  /** What to say in the list when there are no rows. */
  readonly empty: string;
  /** Whether what was typed is the start of a name, so Enter steps into the marked row. */
  readonly filtering: boolean;
  /** The marked row's id, for the field to point at. Nothing when the list is empty. */
  readonly active: string | null;
}

/** What the form knows when it works out [`pickerView`]. */
export interface PickerInput {
  readonly path: string;
  readonly name: string;
  readonly marked: number;
  /** Whether the mark follows what was typed, rather than the arrows or the pointer. */
  readonly followTyped: boolean;
  /** The path the listing was asked for. */
  readonly requested: string;
  readonly listing: DirectoryListing | null;
  readonly notice: string;
  /** Whether a listing, or a refusal, has arrived. */
  readonly settled: boolean;
}

/** The last part of a directory, which is the name a project starts with. */
export function directoryBaseName(directory: string): string {
  const parts = directory.split(/[\\/]/).filter((part) => part !== '');
  return parts[parts.length - 1] ?? directory;
}

/** `directory` with `name` inside it. The top of the filesystem gains no extra slash. */
export function childPath(directory: string, name: string): string {
  if (directory === '/' || directory === '\\') return `/${name}`;
  const separator = directory.includes('\\') && !directory.includes('/') ? '\\' : '/';
  const base = directory.endsWith(separator) ? directory.slice(0, -1) : directory;
  return `${base}${separator}${name}`;
}

/**
 * The rows for what was typed.
 *
 * The parent is offered when there is one and the typed ending does not
 * rule it out. A name starting with `.` stays hidden until the ending
 * starts with `.`.
 */
export function pickerRows(listing: DirectoryListing): PickerRow[] {
  const needle = filterText(listing.remainder).toLocaleLowerCase();
  const hidden = needle.startsWith('.');
  const children = listing.children
    .filter((name) => hidden || !name.startsWith('.'))
    .filter((name) => needle === '' || name.toLocaleLowerCase().startsWith(needle))
    .map((name) => ({ label: name, path: childPath(listing.directory, name) }));
  if (listing.parent === null || (needle !== '' && !'..'.startsWith(needle))) return children;
  return [{ label: '..', path: listing.parent }, ...children];
}

/**
 * The row Enter would step into.
 *
 * While the letters decide, an exact name is marked. The arrows and the
 * pointer leave a mark of their own, kept inside the list.
 */
export function markedIndex(marked: number, followTyped: boolean, rows: readonly PickerRow[], remainder: string): number {
  if (rows.length === 0) return 0;
  if (followTyped) {
    const needle = filterText(remainder).toLocaleLowerCase();
    const exact = needle === '' ? -1 : rows.findIndex((row) => row.label.toLocaleLowerCase() === needle);
    return exact >= 0 ? exact : 0;
  }
  return Math.min(Math.max(marked, 0), rows.length - 1);
}

/**
 * The real directory, when the field should show it instead of what was typed.
 *
 * Nothing while a name is still being typed, and nothing when the field
 * already says that directory apart from a trailing slash: rewriting it
 * would move the cursor.
 */
export function shownPath(typed: string, listing: DirectoryListing): string | null {
  if (listing.remainder !== '') return null;
  if (samePlace(typed.trim(), listing.directory)) return null;
  return listing.directory;
}

/**
 * The project name after the chosen directory changes.
 *
 * A name the person typed stays. A blank one, or one that still repeats
 * the directory it was taken from, follows the directory they are on now.
 */
export function keptName(name: string, custom: boolean, directory: string): { name: string; custom: boolean } {
  const automatic = directoryBaseName(directory);
  if (!custom || name.trim() === '') return { name: automatic, custom: false };
  return { name, custom: true };
}

/** A listing the screen already has, and the text it was asked with. */
export interface HeldListing {
  readonly requested: string;
  readonly listing: DirectoryListing;
}

/**
 * The listing to keep using when the typed path has not entered another directory.
 *
 * Nothing when the screen has to ask again. Letters at the end of a name reuse
 * the children already in hand. A slash, or another part, after a real child
 * directory means that directory was entered, so the old children are finished.
 */
export function carriedListing(path: string, held: HeldListing | null): DirectoryListing | null {
  if (!held) return null;
  if (samePlace(path.trim(), held.listing.directory)) return { ...held.listing, remainder: '' };
  const anchor = typedAnchor(held.requested, held.listing.remainder);
  if (anchor === null) return null;
  const rest = typedRest(path.trim(), anchor);
  if (!rest || entersDirectory(rest, held.listing)) return null;
  return { ...held.listing, remainder: rest.text };
}

/** What Enter does in the directory field. `submit` lets the form add the project. */
export function directoryEnter(view: PickerView): 'step' | 'stay' | 'submit' {
  if (!view.filtering) return 'submit';
  return view.rows[view.marked] ? 'step' : 'stay';
}

/**
 * The typed text that named `directory`, with the unfinished ending removed.
 *
 * Nothing when the ending is not the end of what was asked, so the two cannot be lined up.
 */
function typedAnchor(requested: string, remainder: string): string | null {
  const typed = requested.trim();
  if (remainder === '') return stripSlash(typed);
  const raw = typed.replace(/[\\/]+$/, '');
  if (!raw.endsWith(remainder)) return null;
  return stripSlash(raw.slice(0, raw.length - remainder.length));
}

/** The part of `path` after `anchor`, when `path` is still that directory. */
function typedRest(path: string, anchor: string): { text: string; parts: string[]; finished: boolean } | null {
  if (anchor === '') {
    if (path.startsWith('/') || path.startsWith('\\') || path === '~' || path.startsWith('~/') || path.startsWith('~\\')) return null;
    return splitRest(path);
  }
  if (anchor === '/' || anchor === '\\') {
    if (path === anchor) return splitRest('');
    if (!path.startsWith(anchor)) return null;
    return splitRest(path.slice(anchor.length));
  }
  const base = stripSlash(anchor);
  if (stripSlash(path) === base) return splitRest('');
  const separator = path.startsWith(`${base}/`) ? '/' : path.startsWith(`${base}\\`) ? '\\' : '';
  if (separator === '') return null;
  return splitRest(path.slice(base.length + separator.length));
}

function splitRest(text: string): { text: string; parts: string[]; finished: boolean } {
  return {
    text,
    parts: text.split(/[\\/]/).filter((part) => part !== ''),
    finished: /[\\/]$/.test(text),
  };
}

/**
 * Whether the ending has gone into a child, `.`, or `..`.
 *
 * The exact name of a child, with nothing after it, is still only a filter.
 * A slash after it, or another part, has stepped inside.
 */
function entersDirectory(rest: { parts: string[]; finished: boolean }, listing: DirectoryListing): boolean {
  const [first, ...more] = rest.parts;
  if (!first) return false;
  const known = first === '..' || first === '.' || listing.children.includes(first);
  return known && (rest.finished || more.length > 0);
}

/**
 * What to do with a listing that just arrived.
 *
 * Nothing when it is for a path the field has since left, or when the
 * field still has the start of a name: the list narrows, and the name
 * stays until a real directory is named.
 */
export function adopted(
  requested: string,
  listing: DirectoryListing | null,
  path: string,
  name: string,
  customName: boolean,
): Adopted | null {
  if (path.trim() !== requested.trim() || !listing || listing.remainder !== '') return null;
  const kept = keptName(name, customName, listing.directory);
  return { path: shownPath(requested, listing), name: kept.name, customName: kept.custom, directory: listing.directory };
}

/**
 * Whether `listing` is the answer for what the field holds now.
 *
 * A late answer for some other directory must not flash its children
 * under a path that has moved on.
 */
export function listingApplies(path: string, requested: string, listing: DirectoryListing | null): boolean {
  if (!listing) return false;
  const typed = path.trim();
  if (typed === requested.trim()) return true;
  return isInside(stripSlash(typed), stripSlash(listing.directory));
}

/** The form's view for one moment of typing, listing and marking. */
export function pickerView(input: PickerInput): PickerView {
  const listing = input.settled && listingApplies(input.path, input.requested, input.listing) ? input.listing : null;
  const rows = listing ? pickerRows(listing) : [];
  const filtering = listing !== null && filterText(listing.remainder) !== '';
  const marked = markedIndex(input.marked, input.followTyped, rows, listing?.remainder ?? '');
  const children = rows.some((row) => row.label !== '..');
  const showEmpty = rows.length === 0 || (!children && !filtering);
  return {
    path: input.path,
    name: input.name,
    rows,
    marked,
    notice: input.settled ? input.notice : '',
    empty: showEmpty ? emptyText(input.notice, listing, filtering) : '',
    filtering,
    active: rows.length ? `directory-option-${marked}` : null,
  };
}

/** The first part of what was typed after the directory, which is what narrows the list. */
function filterText(remainder: string): string {
  return remainder.split(/[\\/]/).find((part) => part !== '') ?? '';
}

/** What the list says when it has no rows. Nothing when a refusal is already on screen. */
function emptyText(notice: string, listing: DirectoryListing | null, filtering: boolean): string {
  if (notice) return '';
  if (!listing) return 'Looking…';
  if (filtering) return 'No directory matches.';
  return 'This directory is empty.';
}

/** Whether `place` is `directory` or something inside it. */
function isInside(place: string, directory: string): boolean {
  if (place === directory) return true;
  const prefix = directory === '/' || directory === '\\' ? directory : `${directory}/`;
  if (place.startsWith(prefix)) return true;
  return directory !== '/' && directory !== '\\' && place.startsWith(`${directory}\\`);
}

/** The path without a trailing slash, except the filesystem's own top. */
function stripSlash(path: string): string {
  if (path === '/' || path === '\\') return path;
  return path.replace(/[\\/]+$/, '');
}

/** The two paths name one directory, apart from a trailing slash. */
function samePlace(typed: string, directory: string): boolean {
  return stripSlash(typed) === stripSlash(directory);
}
