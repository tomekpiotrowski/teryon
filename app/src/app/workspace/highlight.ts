import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import c from 'highlight.js/lib/languages/c';
import cpp from 'highlight.js/lib/languages/cpp';
import csharp from 'highlight.js/lib/languages/csharp';
import css from 'highlight.js/lib/languages/css';
import dockerfile from 'highlight.js/lib/languages/dockerfile';
import go from 'highlight.js/lib/languages/go';
import ini from 'highlight.js/lib/languages/ini';
import java from 'highlight.js/lib/languages/java';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import kotlin from 'highlight.js/lib/languages/kotlin';
import makefile from 'highlight.js/lib/languages/makefile';
import markdown from 'highlight.js/lib/languages/markdown';
import php from 'highlight.js/lib/languages/php';
import python from 'highlight.js/lib/languages/python';
import ruby from 'highlight.js/lib/languages/ruby';
import rust from 'highlight.js/lib/languages/rust';
import scss from 'highlight.js/lib/languages/scss';
import sql from 'highlight.js/lib/languages/sql';
import swift from 'highlight.js/lib/languages/swift';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';
import type { FileLine, LanguageName } from './files.model';

/**
 * What a piece of code is, as far as its colour goes.
 *
 * Our own short list rather than the many names highlight.js has, so the
 * styles stay few and do not depend on the library.
 */
export type TokenKind =
  | 'attribute'
  | 'comment'
  | 'constant'
  | 'emphasis'
  | 'keyword'
  | 'meta'
  | 'name'
  | 'string'
  | 'strong'
  | 'type';

/** A run of text within one line that is all shown the same way. */
export interface CodeToken {
  readonly text: string;
  /** Nothing for text that keeps the plain colour. */
  readonly kind: TokenKind | null;
}

/** A line of a file, cut into the pieces that are coloured differently. */
export interface CodeLine extends FileLine {
  /** Put together they are exactly `text`. An empty line has none. */
  readonly tokens: readonly CodeToken[];
}

type Grammar = Parameters<typeof hljs.registerLanguage>[1];

/**
 * The colouring rules of each language, under the name highlight.js knows them by.
 *
 * The names matter: rules for one language reach for another by name, as
 * HTML does for the scripts and styles inside it.
 */
const GRAMMARS: Readonly<Record<Exclude<LanguageName, 'Plain text'>, readonly [string, Grammar]>> = {
  C: ['c', c],
  'C++': ['cpp', cpp],
  'C#': ['csharp', csharp],
  CSS: ['css', css],
  Dockerfile: ['dockerfile', dockerfile],
  Go: ['go', go],
  HTML: ['xml', xml],
  Java: ['java', java],
  JavaScript: ['javascript', javascript],
  JSON: ['json', json],
  Kotlin: ['kotlin', kotlin],
  Makefile: ['makefile', makefile],
  Markdown: ['markdown', markdown],
  PHP: ['php', php],
  Python: ['python', python],
  Ruby: ['ruby', ruby],
  Rust: ['rust', rust],
  SCSS: ['scss', scss],
  Shell: ['bash', bash],
  SQL: ['sql', sql],
  SVG: ['xml', xml],
  Swift: ['swift', swift],
  TOML: ['ini', ini],
  TypeScript: ['typescript', typescript],
  XML: ['xml', xml],
  YAML: ['yaml', yaml],
};

for (const [name, grammar] of Object.values(GRAMMARS)) hljs.registerLanguage(name, grammar);

/**
 * How each highlight.js name is shown. Nothing means plain text.
 *
 * A name that is missing here is shown like whatever it sits inside, which is
 * how the escaped characters of a string stay the colour of the string.
 */
const KINDS: Readonly<Record<string, TokenKind | null>> = {
  attr: 'attribute',
  attribute: 'attribute',
  built_in: 'type',
  bullet: 'constant',
  code: 'string',
  comment: 'comment',
  emphasis: 'emphasis',
  keyword: 'keyword',
  link: 'constant',
  literal: 'constant',
  meta: 'meta',
  name: 'name',
  number: 'constant',
  params: null,
  quote: 'comment',
  regexp: 'string',
  section: 'name',
  'selector-attr': 'attribute',
  'selector-class': 'name',
  'selector-id': 'name',
  'selector-pseudo': 'attribute',
  'selector-tag': 'keyword',
  string: 'string',
  strong: 'strong',
  subst: null,
  symbol: 'constant',
  'template-tag': 'keyword',
  'template-variable': 'attribute',
  title: 'name',
  type: 'type',
  variable: 'attribute',
};

/**
 * The longest text that is coloured, in characters.
 *
 * Colouring runs where the screen is drawn, so a very long file would freeze
 * it for a moment: about half a second for each million characters on a
 * desktop, several times that on a phone. Files past this are shown plain.
 */
export const LARGEST_COLOURED = 200_000;

/** One piece of what highlight.js writes: the start of a marked run, its end, or text. */
const PIECE = /<span class="([^"]*)">|<\/span>|([^<]+)/g;

const ESCAPED = /&(amp|lt|gt|quot|#x27);/g;

const WRITTEN: Readonly<Record<string, string>> = { amp: '&', lt: '<', gt: '>', quot: '"', '#x27': "'" };

/**
 * The lines of a file with their colouring worked out.
 *
 * Exists so that code in the file browser reads like code in an editor.
 *
 * A language is coloured as a whole, not line by line, because a comment or
 * a string can run over many lines. The lines of a changed file hold two
 * texts at once, the file as it was and as it is, so each is coloured on its
 * own: a removed line is shown as it was coloured before the change, every
 * other line as it is coloured now.
 *
 * Colouring must never change what a file says. A line whose pieces do not
 * add up to its text is shown plain instead, as is everything when the
 * language is not known or the text is longer than `LARGEST_COLOURED`.
 */
export function colouredLines(lines: readonly FileLine[], language: LanguageName | ''): CodeLine[] {
  const grammar = language && language !== 'Plain text' ? GRAMMARS[language][0] : null;
  const coloured = new Map<FileLine, readonly CodeToken[]>();
  if (grammar) {
    if (lines.some((line) => line.change === 'removed')) {
      colourVersion(
        lines.filter((line) => line.change !== 'added'),
        grammar,
        coloured,
      );
    }
    // Second, so that a line found in both texts is coloured as it is now.
    colourVersion(
      lines.filter((line) => line.change !== 'removed'),
      grammar,
      coloured,
    );
  }
  return lines.map((line) => ({ ...line, tokens: coloured.get(line) ?? plain(line.text) }));
}

/** Colours one of the two texts of a file, and notes the pieces of every line that came out whole. */
function colourVersion(lines: readonly FileLine[], grammar: string, into: Map<FileLine, readonly CodeToken[]>): void {
  const text = lines.map((line) => line.text).join('\n');
  if (text.length > LARGEST_COLOURED) return;
  const pieces = tokenLines(hljs.highlight(text, { language: grammar, ignoreIllegals: true }).value);
  if (pieces.length !== lines.length) return;
  lines.forEach((line, index) => {
    if (pieces[index].map((token) => token.text).join('') === line.text) into.set(line, pieces[index]);
  });
}

/**
 * Reads what highlight.js wrote back into pieces, line by line.
 *
 * highlight.js only hands out HTML: runs of text inside `<span>`s, one inside
 * another, some of them over several lines. Pieces are safer to show than
 * HTML, because they go to the screen as text, and they can be cut at the
 * end of each line.
 */
function tokenLines(html: string): CodeToken[][] {
  const lines: CodeToken[][] = [[]];
  /** The kinds of the runs the text is inside at the moment, the innermost last. */
  const inside: (TokenKind | null)[] = [];
  for (const [, marked, text] of html.matchAll(PIECE)) {
    if (marked !== undefined) {
      inside.push(kindOf(marked, inside.at(-1) ?? null));
    } else if (text === undefined) {
      inside.pop();
    } else {
      const kind = inside.at(-1) ?? null;
      unescaped(text)
        .split('\n')
        .forEach((part, index) => {
          if (index > 0) lines.push([]);
          if (part) add(lines[lines.length - 1], part, kind);
        });
    }
  }
  return lines;
}

/** How a marked run is shown, given the classes highlight.js put on it and what it sits inside. */
function kindOf(classes: string, around: TokenKind | null): TokenKind | null {
  // Another language inside this one, such as a script in HTML, starts afresh.
  if (classes.startsWith('language-')) return null;
  // Everything inside a comment is part of the comment.
  if (around === 'comment') return around;
  // `hljs-title function_` is a title; what kind of title does not change its colour.
  const name = classes.split(' ')[0].replace(/^hljs-/, '');
  const kind = KINDS[name];
  return kind === undefined ? around : kind;
}

/** Adds text to a line, joining it to the piece before it when both are shown the same way. */
function add(line: CodeToken[], text: string, kind: TokenKind | null): void {
  const last = line.at(-1);
  if (last && last.kind === kind) line[line.length - 1] = { text: last.text + text, kind };
  else line.push({ text, kind });
}

function unescaped(text: string): string {
  return text.replace(ESCAPED, (_, name: string) => WRITTEN[name]);
}

function plain(text: string): CodeToken[] {
  return text ? [{ text, kind: null }] : [];
}
