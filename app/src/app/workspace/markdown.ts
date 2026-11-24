import MarkdownIt, { type StateCore, type Token } from 'markdown-it';

/**
 * Turns the markdown an agent writes into HTML the thread can show.
 *
 * Agents answer in markdown: headings, lists, code, tables. Shown as plain
 * text that is a wall of asterisks and backticks.
 *
 * What an agent writes is not to be trusted, because it can repeat anything it
 * read in a file or on a web page. The rules below follow from that and from
 * how agents write:
 *
 * - HTML in the text is shown as text. It is never handed to the browser.
 * - A picture becomes a link. The browser would fetch a picture the moment it
 *   is shown, and the address it fetches can carry whatever the agent was
 *   tricked into putting there.
 * - A web or email address opens in a new tab. A citation of a file in the
 *   project, or in the open thread's copy, becomes a link that names the
 *   path and opens that file in the code view. `roots` are those directories.
 *   Without them a `file://` address is left as written, and a relative
 *   citation still works.
 * - A bare address becomes a link only when it says how to reach it
 *   (`https://…`). Guessing would turn `README.md` and `main.rs` into links,
 *   because `md` and `rs` are also country codes.
 * - A line break stays a line break: agents do not wrap their lines.
 * - A table column's alignment is a class, not an inline style, because
 *   Angular removes inline styles from HTML it is given.
 *
 * Free of Angular so it can be tested on its own; `MarkdownPipe` is the thin
 * piece that hands the result to the template.
 */
export function renderMarkdown(text: string, roots: readonly string[] = []): string {
  return markdown.render(text, { roots });
}

/**
 * The file a citation link points at.
 *
 * The lines are the file's own, counted from one, so the code view can mark
 * the place the agent named. Nothing for a line means the link names the
 * file and not a place in it.
 */
export interface Citation {
  readonly path: string;
  readonly start: number | null;
  readonly end: number | null;
}

/**
 * Reads a citation link's address back into the file and lines it names.
 *
 * The page handles the click, and the address is the only thing Angular keeps
 * of the link. A path the file API would refuse is nothing, so the click
 * does not open the code on it.
 */
export function citationFromHref(href: string): Citation | null {
  const hash = href.indexOf('#');
  const fragment = (hash >= 0 ? href.slice(hash + 1) : href).trim();
  const query = fragment.indexOf('?');
  if (query < 0 || fragment.slice(0, query) !== 'open') return null;
  const params = new URLSearchParams(fragment.slice(query + 1));
  const path = params.get('path');
  if (path === null || !acceptablePath(path)) return null;
  const start = readLine(params, 'start');
  const end = readLine(params, 'end');
  if (start === undefined || end === undefined) return null;
  if (start === null && end === null) return { path, start: null, end: null };
  if (start === null || end === null || end < start) return null;
  return { path, start, end };
}

/** The addresses a person can follow out of Teryon. */
const FOLLOWABLE = /^(https?:\/\/|mailto:)/i;

const ALIGNMENT = /^text-align:(left|center|right)$/;

/** A `file://` address, with the line mark agents append. The period after a sentence is kept out. */
const FILE_URL = /file:\/\/(?:localhost)?\/[^\s#"),]+(?:#L[1-9]\d*(?:-L[1-9]\d*)?)?/gi;

/** A markdown link whose address is a `file://` URL. markdown-it leaves these as text. */
const FILE_LINK = /\[([^\]\n]*)\]\((file:\/\/[^)\s]+)\)/gi;

/** A path written in brackets. A `]` followed by `(` is a markdown link, not this. */
const BRACKET_PATH = /\[([^\]\n]+)\](?!\()/g;

/** The line mark on the end of a path: `:12`, `:12-40`, `#L12` or `#L12-L40`. */
const LINE_MARK = /(?:#L([1-9]\d*)(?:-L([1-9]\d*))?|:([1-9]\d*)(?:-([1-9]\d*))?)$/;

const markdown = new MarkdownIt({ breaks: true, linkify: true });
markdown.linkify.set({ fuzzyLink: false, fuzzyEmail: false, fuzzyIP: false });
markdown.core.ruler.push('teryon', adapt);

/** Applies the rules described on `renderMarkdown` to what markdown-it parsed. */
function adapt(state: StateCore): void {
  const roots = rootsOf(state.env);
  for (const block of state.tokens) {
    alignCell(block);
    if (block.children) block.children = withSafeLinks(block.children, state, roots);
  }
}

function rootsOf(env: unknown): readonly string[] {
  if (!env || typeof env !== 'object' || !('roots' in env)) return [];
  const roots = (env as { roots?: unknown }).roots;
  return Array.isArray(roots) ? roots.filter((root): root is string => typeof root === 'string') : [];
}

function alignCell(token: Token): void {
  const alignment = ALIGNMENT.exec(String(token.attrGet('style') ?? ''));
  if (alignment) token.attrs = [['class', `align-${alignment[1]}`]];
}

/** The same run of text with pictures turned into links and every link settled. */
function withSafeLinks(tokens: readonly Token[], state: StateCore, roots: readonly string[]): Token[] {
  const shown: Token[] = [];
  let open: Token | null = null;
  let openAt = 0;

  for (const token of tokens) {
    if (token.type === 'image') {
      const address = String(token.attrGet('src') ?? '');
      const label = new state.Token('text', '', 0);
      label.content = token.content || address;
      if (open) {
        // Already inside a link, and a link cannot hold another.
        shown.push(label);
      } else {
        const link = new state.Token('link_open', 'a', 1);
        const end = new state.Token('link_close', 'a', -1);
        link.attrSet('href', address);
        settlePlain(link, end, address);
        shown.push(link, label, end);
      }
      continue;
    }

    // A code span is its own token, so it is never scanned. Text inside a
    // link is the link's words, not a citation of its own.
    if (token.type === 'text' && !open) {
      shown.push(...citeText(token, state, roots));
      continue;
    }

    if (token.type === 'link_open') {
      open = token;
      openAt = shown.length;
    }
    if (token.type === 'link_close' && open) {
      settleLink(open, token, shown, openAt, state);
      open = null;
    }
    shown.push(token);
  }
  return shown;
}

/** Makes a link either one that opens in a new tab, or text that names its address. */
function settlePlain(open: Token, close: Token, address: string): void {
  if (FOLLOWABLE.test(address)) {
    open.attrSet('target', '_blank');
    open.attrSet('rel', 'noopener noreferrer');
    return;
  }
  open.tag = close.tag = 'span';
  // An address that holds the data itself can be enormous and tells a person nothing.
  open.attrs = address.startsWith('data:') ? null : [['title', address]];
}

/**
 * Makes a markdown link a citation when its address is a file in the repository.
 *
 * The words the agent wrote stay when they are not the address itself, and
 * the path is then the tooltip. Otherwise the link reads as the path.
 */
function settleLink(open: Token, close: Token, shown: Token[], openAt: number, state: StateCore): void {
  const address = String(open.attrGet('href') ?? '');
  if (FOLLOWABLE.test(address)) {
    open.attrSet('target', '_blank');
    open.attrSet('rel', 'noopener noreferrer');
    return;
  }
  const citation = citationFromRelative(address);
  if (citation) {
    const label = shown
      .slice(openAt + 1)
      .map((token) => token.content)
      .join('');
    applyCitation(open, shown, openAt, state, citation, label, address);
    return;
  }
  settlePlain(open, close, address);
}

/** A citation found in a run of text, and the characters it replaces. */
interface FoundCitation {
  readonly start: number;
  readonly end: number;
  readonly citation: Citation;
  /** The words of a markdown link. Nothing when the link should read as the path. */
  readonly label: string | null;
  /** The address as written, so it can be told apart from the agent's own words. */
  readonly address: string;
}

/** Replaces the citations in a run of text with links, leaving the rest as text. */
function citeText(token: Token, state: StateCore, roots: readonly string[]): Token[] {
  const found = scanCitations(token.content, roots);
  if (found.length === 0) return [token];
  const pieces: Token[] = [];
  let cursor = 0;
  for (const item of found) {
    if (item.start > cursor) pieces.push(plainText(state, token.content.slice(cursor, item.start)));
    pieces.push(...citationTokens(state, item));
    cursor = item.end;
  }
  if (cursor < token.content.length) pieces.push(plainText(state, token.content.slice(cursor)));
  return pieces;
}

/**
 * The citations in `text`, in order.
 *
 * A `file://` address is one only when it sits inside a root, on a path
 * boundary, so an address outside the project stays as it was written. A
 * path in brackets is one only when it could be a file in the repository.
 */
function scanCitations(text: string, roots: readonly string[]): FoundCitation[] {
  const found: FoundCitation[] = [];
  let index = 0;
  while (index < text.length) {
    const linked = findAt(FILE_LINK, text, index);
    const file = findAt(FILE_URL, text, index);
    const bracket = findAt(BRACKET_PATH, text, index);
    const choice = earliest(linked, file, bracket);
    if (!choice || choice.start < index) break;

    if (choice === linked && linked) {
      const address = linked.groups[1] ?? '';
      const citation = citationFromFile(address, roots);
      if (citation) found.push({ start: linked.start, end: linked.end, citation, label: linked.groups[0] ?? '', address });
      index = linked.end;
      continue;
    }

    if (choice === file && file) {
      const taken = withoutSentencePeriod(file.match[0], file.end);
      const citation = citationFromFile(taken.address, roots);
      if (citation && boundaryBefore(text, file.start)) {
        let start = file.start;
        let end = taken.end;
        // Parentheses that wrap only the address are part of the citation.
        if (text[start - 1] === '(' && text[end] === ')') {
          start -= 1;
          end += 1;
        }
        found.push({ start, end, citation, label: null, address: taken.address });
        index = end;
      } else {
        index = Math.max(file.end, index + 1);
      }
      continue;
    }

    if (choice === bracket && bracket) {
      const inside = bracket.groups[0] ?? '';
      const citation = citationFromBracket(inside);
      if (citation) found.push({ start: bracket.start, end: bracket.end, citation, label: null, address: inside });
      index = Math.max(bracket.end, index + 1);
      continue;
    }

    index += 1;
  }
  return found;
}

/** A match and the text its groups captured. */
interface MatchAt {
  readonly start: number;
  readonly end: number;
  readonly match: RegExpExecArray;
  readonly groups: readonly string[];
}

function findAt(pattern: RegExp, text: string, from: number): MatchAt | null {
  const expression = new RegExp(pattern.source, pattern.flags);
  expression.lastIndex = from;
  const match = expression.exec(text);
  if (!match || match.index < from) return null;
  return { start: match.index, end: match.index + match[0].length, match, groups: match.slice(1) };
}

function earliest(...matches: Array<MatchAt | null>): MatchAt | null {
  let best: MatchAt | null = null;
  for (const match of matches) {
    if (!match) continue;
    if (!best || match.start < best.start) best = match;
  }
  return best;
}

/** Whether `file://` starts a new address and is not the tail of a longer token. */
function boundaryBefore(text: string, index: number): boolean {
  const previous = text[index - 1] ?? '';
  return previous === '' || !/[A-Za-z0-9+.-]/.test(previous);
}

function citationTokens(state: StateCore, found: FoundCitation): Token[] {
  const open = new state.Token('link_open', 'a', 1);
  const close = new state.Token('link_close', 'a', -1);
  const canonical = citationText(found.citation);
  const label = found.label ?? '';
  const ownWords = label !== '' && label !== found.address && label !== canonical;
  open.attrs = [
    ['class', ownWords ? 'file-citation' : 'file-citation path'],
    ['href', citationHref(found.citation)],
    ['title', canonical],
  ];
  const text = new state.Token('text', '', 0);
  text.content = ownWords ? label : canonical;
  return [open, text, close];
}

/** Drops sentence periods the address pattern swallowed, so they stay after the link. */
function withoutSentencePeriod(address: string, end: number): { address: string; end: number } {
  let trimmed = address;
  let cursor = end;
  while (trimmed.endsWith('.')) {
    trimmed = trimmed.slice(0, -1);
    cursor -= 1;
  }
  return { address: trimmed, end: cursor };
}

/**
 * Points a link at a citation.
 *
 * `label` is what the agent wrote. When that is the address, or empty, the
 * link text becomes the path, in the code font. The tooltip is always the path.
 */
function applyCitation(open: Token, shown: Token[], openAt: number, state: StateCore, citation: Citation, label: string, address: string): void {
  const canonical = citationText(citation);
  const ownWords = label !== '' && label !== address && label !== canonical;
  open.tag = 'a';
  open.attrs = [
    ['class', ownWords ? 'file-citation' : 'file-citation path'],
    ['href', citationHref(citation)],
    ['title', canonical],
  ];
  if (!ownWords && shown.length > openAt) {
    const text = new state.Token('text', '', 0);
    text.content = canonical;
    shown.splice(openAt + 1, shown.length - openAt - 1, text);
  }
}

function citationText(citation: Citation): string {
  if (citation.start === null) return citation.path;
  const end = citation.end ?? citation.start;
  return end === citation.start ? `${citation.path}:${citation.start}` : `${citation.path}:${citation.start}-${end}`;
}

/** The in-page address of a citation. Angular keeps `href`, and the page reads it back. */
function citationHref(citation: Citation): string {
  const path = `path=${encodeURIComponent(citation.path)}`;
  if (citation.start === null) return `#open?${path}`;
  const end = citation.end ?? citation.start;
  return `#open?${path}&start=${citation.start}&end=${end}`;
}

/**
 * The repository path a `file://` address names, when it sits inside a root.
 *
 * The longest root wins, and only on a path boundary, so a directory that
 * merely shares a prefix does not count. The copy is a root as well as the
 * project because an agent writes whichever one it has open.
 */
function citationFromFile(address: string, roots: readonly string[]): Citation | null {
  const parsed = parseFileAddress(address);
  if (!parsed) return null;
  const path = pathInside(parsed.absolute, roots);
  if (path === null || !acceptablePath(path)) return null;
  return { path, start: parsed.start, end: parsed.end };
}

function parseFileAddress(address: string): { absolute: string; start: number | null; end: number | null } | null {
  const match = /^file:\/\/(?:localhost)?(\/[^#]*?)(?:#(L[1-9]\d*(?:-L[1-9]\d*)?))?$/i.exec(address);
  if (!match) return null;
  let pathname = match[1];
  if (pathname.endsWith('.') || pathname.endsWith(',')) pathname = pathname.slice(0, -1);
  let absolute: string;
  try {
    absolute = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (!absolute.startsWith('/')) return null;
  const fragment = match[2] ?? '';
  if (fragment === '') return { absolute, start: null, end: null };
  const lines = /^L([1-9]\d*)(?:-L([1-9]\d*))?$/i.exec(fragment);
  if (!lines) return null;
  const start = Number(lines[1]);
  const end = lines[2] === undefined ? start : Number(lines[2]);
  if (end < start) return null;
  return { absolute, start, end };
}

function pathInside(absolute: string, roots: readonly string[]): string | null {
  const sorted = [...roots].filter((root) => root !== '').sort((a, b) => b.length - a.length);
  for (const root of sorted) {
    const base = root.endsWith('/') ? root.slice(0, -1) : root;
    if (absolute.startsWith(`${base}/`)) return absolute.slice(base.length + 1);
  }
  return null;
}

/**
 * A relative repository path used as a markdown link's address, with its line mark.
 *
 * The address is decoded first. markdown-it encodes a backslash, and a path
 * is judged after that, the way the file API reads it.
 */
function citationFromRelative(address: string): Citation | null {
  if (address === '' || address.startsWith('#') || address.startsWith('/') || address.startsWith('//')) return null;
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(address)) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(address);
  } catch {
    return null;
  }
  const marked = splitLineMark(decoded);
  if (!marked || !acceptablePath(marked.path)) return null;
  return marked;
}

/**
 * A path written in brackets, such as `[src/a.rs:12]`.
 *
 * Prose in brackets is left alone: a citation here has a `/` or a `.` in it,
 * which a footnote number and an ordinary phrase do not.
 */
function citationFromBracket(inside: string): Citation | null {
  const marked = splitLineMark(inside);
  if (!marked || !acceptablePath(marked.path) || !looksLikeFilePath(marked.path)) return null;
  return marked;
}

function looksLikeFilePath(path: string): boolean {
  return path.includes('/') || path.includes('.');
}

function splitLineMark(address: string): Citation | null {
  const match = LINE_MARK.exec(address);
  if (!match) return { path: address, start: null, end: null };
  const path = address.slice(0, match.index);
  const start = Number(match[1] ?? match[3]);
  const end = Number(match[2] ?? match[4] ?? start);
  if (path === '' || end < start) return null;
  return { path, start, end };
}

/**
 * Whether the file API would read this path.
 *
 * The same refusals as `clean_path` on the server: empty, absolute, a `.` or
 * `..` segment, a backslash, and anything inside the repository's own `.git`.
 */
function acceptablePath(path: string): boolean {
  if (path === '' || path.includes('\\') || path.includes('\0') || path.startsWith('/') || path.endsWith('/')) return false;
  const parts = path.split('/');
  if (parts.some((part) => part === '' || part === '.' || part === '..')) return false;
  return parts[0].toLowerCase() !== '.git';
}

/** A line number from a citation address: null when absent, nothing when present but not a line number. */
function readLine(params: URLSearchParams, name: string): number | null | undefined {
  if (!params.has(name)) return null;
  const raw = params.get(name) ?? '';
  if (!/^[1-9]\d*$/.test(raw)) return undefined;
  const value = Number(raw);
  return Number.isSafeInteger(value) ? value : undefined;
}

function plainText(state: StateCore, content: string): Token {
  const token = new state.Token('text', '', 0);
  token.content = content;
  return token;
}
