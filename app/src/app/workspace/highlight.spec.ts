import type { FileLine, LanguageName } from './files.model';
import { type CodeLine, colouredLines, LARGEST_COLOURED } from './highlight';

/** The lines of an unchanged file holding this text. */
function fileOf(text: string): FileLine[] {
  return text.split('\n').map((line, index) => ({ change: null, number: index + 1, text: line }));
}

function line(change: FileLine['change'], number: number | null, text: string): FileLine {
  return { change, number, text };
}

/** Each line as its pieces, a coloured piece written as `kind:text`. */
function drawn(lines: readonly CodeLine[]): string[][] {
  return lines.map((coloured) => coloured.tokens.map((token) => (token.kind ? `${token.kind}:${token.text}` : token.text)));
}

/** A little of every language, with at least one thing in it worth colouring. */
const SAMPLES: Readonly<Record<Exclude<LanguageName, 'Plain text'>, string>> = {
  C: '#include <stdio.h>\nint main(void) {\n\treturn 0; /* done */\n}',
  'C++': 'template <typename T>\nclass Box { public: T value = nullptr; };',
  'C#': 'namespace App {\n  public class Entry { string Name = "a"; }\n}',
  CSS: 'a:hover > .row, #top {\n  color: #fff;\n  margin: 0 auto;\n}',
  Dockerfile: 'FROM rust:1 AS build\nRUN cargo build --release\nCOPY . .',
  Go: 'package main\n\nfunc main() {\n\tprintln("hi", 1)\n}',
  HTML: '<!doctype html>\n<p class="a">1 &amp; 2</p>\n<script>const a = "<b>";</script>\n<style>p { color: red; }</style>',
  Java: 'public final class Entry {\n  private final int id = 1; // id\n}',
  JavaScript: 'export const add = (a, b) => `${a + b}`;\nif (a < b && b > 1) throw new Error("no");',
  JSON: '{\n  "name": "teryon",\n  "private": true,\n  "count": 1\n}',
  Kotlin: 'fun main() {\n  val name: String = "a"\n  println("$name")\n}',
  Makefile: 'build: main.c\n\tcc -o $@ $<\n.PHONY: build',
  Markdown: '# Title\n\n*one* **two** `three` [four](https://example.com)\n\n```\nfive\n```\n> six',
  PHP: '<?php\nfunction add(int $a): int {\n  return $a + 1;\n}',
  Python: '@cache\ndef add(a: int) -> int:\n    """Adds."""\n    return a + 1  # one',
  Ruby: 'class Entry\n  def name = @name || :none\nend',
  Rust: '#[derive(Debug)]\npub struct Entry<\'a> {\n    name: &\'a str, // the name\n}\nfn main() { let x = 1_u8; }',
  SCSS: '$gap: 4px;\n.row {\n  &:hover { margin: $gap; }\n}',
  Shell: '#!/bin/sh\nfor f in *.txt; do\n  echo "$f" > /dev/null\ndone',
  SQL: "SELECT id, name FROM entries WHERE name = 'a' AND id > 1; -- all",
  SVG: '<svg viewBox="0 0 1 1"><!-- dot --><circle r="1"/></svg>',
  Swift: 'struct Entry {\n  let name: String = "a"\n  func show() -> Int { return 1 }\n}',
  TOML: '[package]\nname = "teryon"\nedition = 2024\n# done',
  TypeScript: 'export class Adapter<T> {\n  async write(entry: T): Promise<void> {\n    return this.ledger.append(entry as "x");\n  }\n}',
  XML: '<?xml version="1.0"?>\n<a b="c"><![CDATA[ x < y ]]></a>',
  YAML: 'name: ci\non: [push]\njobs:\n  build: { runs-on: "ubuntu" } # one',
};

describe('colouredLines', () => {
  it('colours the words, strings, numbers and comments of a language', () => {
    const lines = colouredLines(fileOf('const count = 42; // how many\nreturn "text";'), 'TypeScript');

    expect(drawn(lines)).toEqual([
      ['keyword:const', ' count = ', 'constant:42', '; ', 'comment:// how many'],
      ['keyword:return', ' ', 'string:"text"', ';'],
    ]);
  });

  it('keeps everything about a line but adds its pieces', () => {
    const [first] = colouredLines([line('added', 7, 'let a;')], 'JavaScript');

    expect(first).toEqual({
      change: 'added',
      number: 7,
      text: 'let a;',
      tokens: [
        { text: 'let', kind: 'keyword' },
        { text: ' a;', kind: null },
      ],
    });
  });

  it('carries a comment over the lines it runs across', () => {
    const lines = colouredLines(fileOf('/* one\n   two */ let a;'), 'JavaScript');

    expect(drawn(lines)).toEqual([['comment:/* one'], ['comment:   two */', ' ', 'keyword:let', ' a;']]);
  });

  it('carries a string over the lines it runs across, an empty line among them', () => {
    const lines = colouredLines(fileOf('a = `one\n\ntwo`;'), 'JavaScript');

    expect(drawn(lines)).toEqual([['a = ', 'string:`one'], [], ['string:two`', ';']]);
  });

  it('shows everything inside a comment as the comment', () => {
    const lines = colouredLines(fileOf('/** @param count how many */'), 'TypeScript');

    expect(drawn(lines)).toEqual([['comment:/** @param count how many */']]);
  });

  it('shows what a string works out from code as code, not as the string', () => {
    const lines = colouredLines(fileOf('`a${b + 1}c`'), 'JavaScript');

    expect(drawn(lines)).toEqual([['string:`a', '${b + ', 'constant:1', '}', 'string:c`']]);
  });

  it('colours the name of a function, whatever kind of name it is, and leaves what it takes plain', () => {
    const lines = colouredLines(fileOf('function add(left, right) {}'), 'JavaScript');

    expect(drawn(lines)).toEqual([['keyword:function', ' ', 'name:add', '(left, right) {}']]);
  });

  it('colours a script inside HTML as a script', () => {
    const lines = colouredLines(fileOf('<script>\nconst a = 1;\n</script>'), 'HTML');

    expect(drawn(lines)[1]).toEqual(['keyword:const', ' a = ', 'constant:1', ';']);
  });

  it('gives back the characters that HTML would have to escape as they were written', () => {
    const text = `if (a < b && c > d) say("it's", '"&amp;"');`;

    const [only] = colouredLines(fileOf(text), 'JavaScript');

    expect(only.tokens.map((token) => token.text).join('')).toBe(text);
    expect(only.tokens).toContainEqual({ text: `"it's"`, kind: 'string' });
    expect(only.tokens).toContainEqual({ text: `'"&amp;"'`, kind: 'string' });
  });

  it('keeps tabs, spaces at either end and the carriage return of a Windows line ending', () => {
    const lines = colouredLines([line(null, 1, '\tlet a;  \r'), line(null, 2, '  \r')], 'JavaScript');

    expect(drawn(lines)).toEqual([['\t', 'keyword:let', ' a;  \r'], ['  \r']]);
  });

  it('joins neighbouring pieces that are shown the same way', () => {
    const [only] = colouredLines(fileOf('a.b.c = d;'), 'JavaScript');

    expect(only.tokens).toEqual([{ text: 'a.b.c = d;', kind: null }]);
  });

  it.each(['', 'Plain text'] as const)('leaves the lines plain when the language is "%s"', (language) => {
    const lines = colouredLines(fileOf('const a = 1;\n\n<b>&amp;</b>'), language);

    expect(drawn(lines)).toEqual([['const a = 1;'], [], ['<b>&amp;</b>']]);
  });

  it('leaves the lines plain when the coloured text does not come out with as many lines', () => {
    const lines = colouredLines([line(null, 1, 'let a;\nlet b;'), line(null, 2, 'let c;')], 'JavaScript');

    expect(drawn(lines)).toEqual([['let a;\nlet b;'], ['let c;']]);
  });

  it('gives no lines for a file that has none', () => {
    expect(colouredLines([], 'Rust')).toEqual([]);
  });

  it('gives an empty line no pieces', () => {
    expect(drawn(colouredLines(fileOf(''), 'Rust'))).toEqual([[]]);
  });

  describe('for a changed file', () => {
    it('colours a removed line as the file was and every other line as the file is', () => {
      const lines = colouredLines(
        [
          line('removed', null, 'const a = `one'),
          line('removed', null, 'two`;'),
          line('added', 1, 'const a = 1;'),
          line(null, 2, 'two();'),
        ],
        'JavaScript',
      );

      expect(drawn(lines)).toEqual([
        ['keyword:const', ' a = ', 'string:`one'],
        // Inside a string in the file as it was.
        ['string:two`', ';'],
        ['keyword:const', ' a = ', 'constant:1', ';'],
        // The same words are code in the file as it is.
        ['name:two', '();'],
      ]);
    });

    it('colours a line found in both texts as it is now', () => {
      const lines = colouredLines(
        [line('removed', null, '/*'), line('added', 1, '//'), line(null, 2, 'let a;'), line('removed', null, '*/')],
        'JavaScript',
      );

      expect(drawn(lines)).toEqual([['comment:/*'], ['comment://'], ['keyword:let', ' a;'], ['comment:*/']]);
    });

    it('colours a file with nothing but removed lines', () => {
      const lines = colouredLines([line('removed', null, 'let a;'), line('removed', null, '// gone')], 'JavaScript');

      expect(drawn(lines)).toEqual([['keyword:let', ' a;'], ['comment:// gone']]);
    });

    it('colours a file with nothing but added lines', () => {
      const lines = colouredLines([line('added', 1, 'let a;'), line('added', 2, '// new')], 'JavaScript');

      expect(drawn(lines)).toEqual([['keyword:let', ' a;'], ['comment:// new']]);
    });

    it('leaves the text that is too long plain and still colours the other', () => {
      const long = `// ${'x'.repeat(LARGEST_COLOURED)}`;

      const lines = colouredLines([line('removed', null, long), line('added', 1, 'let a;')], 'JavaScript');

      expect(drawn(lines)).toEqual([[long], ['keyword:let', ' a;']]);
    });
  });

  describe('for a long file', () => {
    /** A file of comment lines that is this many characters long, line breaks included. */
    function commentsOf(length: number): FileLine[] {
      const lines = fileOf('// 345678\n'.repeat(length / 10).slice(0, -1));
      expect(lines.map((each) => each.text).join('\n')).toHaveLength(length - 1);
      return lines;
    }

    it('colours a text of exactly the longest length', () => {
      const lines = commentsOf(LARGEST_COLOURED);
      lines.push(line(null, lines.length + 1, ''));

      expect(drawn(colouredLines(lines, 'JavaScript'))[0]).toEqual(['comment:// 345678']);
    });

    it('leaves a text one character longer plain', () => {
      const lines = commentsOf(LARGEST_COLOURED);
      lines.push(line(null, lines.length + 1, '/'));

      expect(drawn(colouredLines(lines, 'JavaScript'))[0]).toEqual(['// 345678']);
    });
  });

  describe.each(Object.entries(SAMPLES) as [LanguageName, string][])('for %s', (language, sample) => {
    const lines = colouredLines(fileOf(sample), language);

    it('colours something', () => {
      expect(lines.some((coloured) => coloured.tokens.some((token) => token.kind !== null))).toBe(true);
    });

    it('never changes what a line says', () => {
      expect(lines.map((coloured) => coloured.tokens.map((token) => token.text).join(''))).toEqual(sample.split('\n'));
    });

    it('has no empty pieces', () => {
      expect(lines.flatMap((coloured) => coloured.tokens).every((token) => token.text !== '')).toBe(true);
    });
  });
});
