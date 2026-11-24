import { citationFromHref, renderMarkdown } from './markdown';

/** The rendered HTML on one line, which is easier to compare. */
function html(text: string, roots: readonly string[] = []): string {
  return renderMarkdown(text, roots).replace(/\n/g, '');
}

const PROJECT = '/work/app';
const COPY = '/work/app/.teryon/worktrees/fix-login';

/** The link a citation renders as. `label` keeps the words the agent wrote. */
function anchor(path: string, options: { start?: number; end?: number; label?: string } = {}): string {
  const canonical = options.start === undefined ? path : options.end !== undefined && options.end !== options.start ? `${path}:${options.start}-${options.end}` : `${path}:${options.start}`;
  const text = options.label ?? canonical;
  const query = options.start === undefined ? `path=${encodeURIComponent(path)}` : `path=${encodeURIComponent(path)}&amp;start=${options.start}&amp;end=${options.end ?? options.start}`;
  const cls = text === canonical ? 'file-citation path' : 'file-citation';
  return `<a class="${cls}" href="#open?${query}" title="${canonical}">${text}</a>`;
}

const NEW_TAB = 'target="_blank" rel="noopener noreferrer"';

describe('renderMarkdown', () => {
  describe('text', () => {
    it('renders nothing for no text', () => {
      expect(renderMarkdown('')).toBe('');
    });

    it('renders nothing for blank text', () => {
      expect(renderMarkdown(' \n\n ')).toBe('');
    });

    it('renders a line as a paragraph', () => {
      expect(html('ready')).toBe('<p>ready</p>');
    });

    it('starts a new paragraph after an empty line', () => {
      expect(html('one\n\ntwo')).toBe('<p>one</p><p>two</p>');
    });

    it('keeps a single line break', () => {
      expect(html('one\ntwo')).toBe('<p>one<br>two</p>');
    });

    it.each([
      ['**bold**', '<p><strong>bold</strong></p>'],
      ['__bold__', '<p><strong>bold</strong></p>'],
      ['*slanted*', '<p><em>slanted</em></p>'],
      ['_slanted_', '<p><em>slanted</em></p>'],
      ['***both***', '<p><em><strong>both</strong></em></p>'],
      ['~~gone~~', '<p><s>gone</s></p>'],
      ['`note.txt`', '<p><code>note.txt</code></p>'],
      ['snake_case_name', '<p>snake_case_name</p>'],
      ['2 * 3 * 4', '<p>2 * 3 * 4</p>'],
      ['\\*not slanted\\*', '<p>*not slanted*</p>'],
    ])('renders %s', (text, expected) => {
      expect(html(text)).toBe(expected);
    });

    it('leaves markdown inside inline code alone', () => {
      expect(html('`**not bold**`')).toBe('<p><code>**not bold**</code></p>');
    });
  });

  describe('structure', () => {
    it.each([1, 2, 3, 4, 5, 6])('renders a level %i heading', (level) => {
      expect(html(`${'#'.repeat(level)} Title`)).toBe(`<h${level}>Title</h${level}>`);
    });

    it('does not treat a seventh level as a heading', () => {
      expect(html('####### Title')).toBe('<p>####### Title</p>');
    });

    it('renders a list', () => {
      expect(html('- one\n- two')).toBe('<ul><li>one</li><li>two</li></ul>');
    });

    it('renders a numbered list', () => {
      expect(html('1. one\n2. two')).toBe('<ol><li>one</li><li>two</li></ol>');
    });

    it('keeps the number a numbered list starts at', () => {
      expect(html('3. three\n4. four')).toBe('<ol start="3"><li>three</li><li>four</li></ol>');
    });

    it('renders a list inside a list', () => {
      expect(html('- one\n  - inner\n- two')).toBe('<ul><li>one<ul><li>inner</li></ul></li><li>two</li></ul>');
    });

    it('renders formatting inside a list item', () => {
      expect(html('- **File write:** denied')).toBe('<ul><li><strong>File write:</strong> denied</li></ul>');
    });

    it('renders a quotation', () => {
      expect(html('> said')).toBe('<blockquote><p>said</p></blockquote>');
    });

    it('renders a dividing line', () => {
      expect(html('one\n\n---\n\ntwo')).toBe('<p>one</p><hr><p>two</p>');
    });
  });

  describe('code blocks', () => {
    it('renders a block and names its language', () => {
      expect(renderMarkdown('```rust\nfn main() {}\n```')).toBe(
        '<pre><code class="language-rust">fn main() {}\n</code></pre>\n',
      );
    });

    it('renders a block without a language', () => {
      expect(renderMarkdown('```\nplain\n```')).toBe('<pre><code>plain\n</code></pre>\n');
    });

    it('renders a block fenced with tildes', () => {
      expect(renderMarkdown('~~~\nplain\n~~~')).toBe('<pre><code>plain\n</code></pre>\n');
    });

    it('renders an indented block', () => {
      expect(renderMarkdown('before\n\n    indented\n')).toBe('<p>before</p>\n<pre><code>indented\n</code></pre>\n');
    });

    it('renders a block that was never closed, as happens when an agent is cut off', () => {
      expect(renderMarkdown('```\nhalf')).toBe('<pre><code>half</code></pre>\n');
    });

    it('keeps line breaks and spacing inside a block', () => {
      expect(renderMarkdown('```\na\n\n  b\n```')).toBe('<pre><code>a\n\n  b\n</code></pre>\n');
    });

    it('leaves markdown inside a block alone', () => {
      expect(html('```\n**not bold** [not a link](https://example.com)\n```')).toBe(
        '<pre><code>**not bold** [not a link](https://example.com)</code></pre>',
      );
    });

    it('shows HTML inside a block as text', () => {
      expect(html('```html\n<script>alert(1)</script>\n```')).toBe(
        '<pre><code class="language-html">&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>',
      );
    });

    it('cannot break out of the language name', () => {
      expect(html('```"><script>alert(1)</script>\ncode\n```')).not.toContain('<script');
    });
  });

  describe('tables', () => {
    it('renders a table', () => {
      expect(html('| a | b |\n|---|---|\n| 1 | 2 |')).toBe(
        '<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>',
      );
    });

    it('marks how each column is aligned with a class', () => {
      expect(html('| a | b | c | d |\n|:--|:-:|--:|---|\n| 1 | 2 | 3 | 4 |')).toBe(
        '<table><thead><tr>' +
          '<th class="align-left">a</th><th class="align-center">b</th><th class="align-right">c</th><th>d</th>' +
          '</tr></thead><tbody><tr>' +
          '<td class="align-left">1</td><td class="align-center">2</td><td class="align-right">3</td><td>4</td>' +
          '</tr></tbody></table>',
      );
    });

    it('never writes an inline style, which Angular would remove', () => {
      expect(html('| a |\n|--:|\n| 1 |')).not.toContain('style=');
    });

    it('renders formatting inside a cell', () => {
      expect(html('| a |\n|---|\n| `x` |')).toContain('<td><code>x</code></td>');
    });
  });

  describe('HTML in the text', () => {
    it.each([
      ['<script>alert(1)</script>', '<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>'],
      ['a <b>bold</b> word', '<p>a &lt;b&gt;bold&lt;/b&gt; word</p>'],
      ['<img src=x onerror=alert(1)>', '<p>&lt;img src=x onerror=alert(1)&gt;</p>'],
      ['<div>\n\nblock\n\n</div>', '<p>&lt;div&gt;</p><p>block</p><p>&lt;/div&gt;</p>'],
      ['<!-- hidden -->', '<p>&lt;!-- hidden --&gt;</p>'],
      ['<a href="page.html" onclick="alert(1)">x</a>', '<p>&lt;a href=&quot;page.html&quot; onclick=&quot;alert(1)&quot;&gt;x&lt;/a&gt;</p>'],
      ['5 < 6 && 7 > 6', '<p>5 &lt; 6 &amp;&amp; 7 &gt; 6</p>'],
      ['&copy; &amp; &lt;b&gt;', '<p>© &amp; &lt;b&gt;</p>'],
    ])('shows %s as text', (text, expected) => {
      expect(html(text)).toBe(expected);
    });
  });

  describe('links', () => {
    it.each([
      ['[site](https://example.com/a?b=1&c=2)', `<p><a href="https://example.com/a?b=1&amp;c=2" ${NEW_TAB}>site</a></p>`],
      ['[site](http://example.com)', `<p><a href="http://example.com" ${NEW_TAB}>site</a></p>`],
      ['[site](HTTPS://EXAMPLE.COM)', `<p><a href="HTTPS://EXAMPLE.COM" ${NEW_TAB}>site</a></p>`],
      ['[write](mailto:person@example.com)', `<p><a href="mailto:person@example.com" ${NEW_TAB}>write</a></p>`],
      ['[site](https://example.com "About")', `<p><a href="https://example.com" title="About" ${NEW_TAB}>site</a></p>`],
      ['[**site**](https://example.com)', `<p><a href="https://example.com" ${NEW_TAB}><strong>site</strong></a></p>`],
      ['[site][1]\n\n[1]: https://example.com', `<p><a href="https://example.com" ${NEW_TAB}>site</a></p>`],
      ['<https://example.com>', `<p><a href="https://example.com" ${NEW_TAB}>https://example.com</a></p>`],
    ])('opens %s in a new tab', (text, expected) => {
      expect(html(text)).toBe(expected);
    });

    it.each([
      ['[root](/etc/hosts)', '<p><span title="/etc/hosts">root</span></p>'],
      ['[up](../other)', '<p><span title="../other">up</span></p>'],
      ['[part](#heading)', '<p><span title="#heading">part</span></p>'],
      ['[host](//example.com/x)', '<p><span title="//example.com/x">host</span></p>'],
      ['[files](ftp://example.com/x)', '<p><span title="ftp://example.com/x">files</span></p>'],
      ['[call](tel:+48123456789)', '<p><span title="tel:+48123456789">call</span></p>'],
      ['[nowhere]()', '<p><span title="">nowhere</span></p>'],
      ['[dot](./README.md)', '<p><span title="./README.md">dot</span></p>'],
      ['[inside](.git/config)', '<p><span title=".git/config">inside</span></p>'],
      ['[slashed](src\\a.rs)', '<p><span title="src%5Ca.rs">slashed</span></p>'],
    ])('shows %s as text that names its address', (text, expected) => {
      expect(html(text)).toBe(expected);
    });

    it.each([
      ['[x](javascript:alert(1))'],
      ['[x](JaVaScRiPt:alert(1))'],
      ['[x](vbscript:msgbox(1))'],
      ['[x](file:///etc/passwd)'],
      ['[x](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)'],
    ])('leaves %s as it was written', (text) => {
      expect(html(text)).toBe(`<p>${text}</p>`);
    });

    it('cannot break out of an address', () => {
      expect(html('[x](https://example.com/"onmouseover="alert(1))')).toBe(
        `<p><a href="https://example.com/%22onmouseover=%22alert(1)" ${NEW_TAB}>x</a></p>`,
      );
    });

    it('cannot break out of a title', () => {
      expect(html('[x](https://example.com "a\\" onmouseover=\\"alert(1)")')).toBe(
        `<p><a href="https://example.com" title="a&quot; onmouseover=&quot;alert(1)" ${NEW_TAB}>x</a></p>`,
      );
    });

    it('settles each link on a line by itself', () => {
      expect(html('[a](a.ts) [b](https://example.com) [c](c.ts)')).toBe(
        `<p>${anchor('a.ts', { label: 'a' })} <a href="https://example.com" ${NEW_TAB}>b</a> ${anchor('c.ts', { label: 'c' })}</p>`,
      );
    });
  });

  describe('citations', () => {
    const roots = [PROJECT, COPY];

    it.each([
      ['[core/src/workspace/stages.rs]', anchor('core/src/workspace/stages.rs')],
      ['[core/src/a.rs:12]', anchor('core/src/a.rs', { start: 12 })],
      ['[core/src/a.rs:12-40]', anchor('core/src/a.rs', { start: 12, end: 40 })],
      ['[core/src/a.rs#L12-L40]', anchor('core/src/a.rs', { start: 12, end: 40 })],
      ['[notes](readme)', anchor('readme', { label: 'notes' })],
      ['[the parser](src/parser.rs)', anchor('src/parser.rs', { label: 'the parser' })],
      ['[the store](app/src/store.ts)', anchor('app/src/store.ts', { label: 'the store' })],
      ['[the store](app/src/store.ts#L12)', anchor('app/src/store.ts', { start: 12, label: 'the store' })],
      ['[src/a.rs](src/a.rs:12-40)', anchor('src/a.rs', { start: 12, end: 40, label: 'src/a.rs' })],
      ['[src/a.rs:12](src/a.rs:12)', anchor('src/a.rs', { start: 12 })],
    ])('shows %s as a citation', (text, link) => {
      expect(html(text)).toBe(`<p>${link}</p>`);
    });

    it('keeps the formatting of words the agent wrote', () => {
      expect(html('[**the store**](store.ts) next')).toBe(`<p>${anchor('store.ts', { label: '<strong>the store</strong>' })} next</p>`);
    });

    it('keeps a web link whose words happen to look like a path', () => {
      expect(html('[core/src/a.rs](https://example.com)')).toBe(`<p><a href="https://example.com" ${NEW_TAB}>core/src/a.rs</a></p>`);
    });

    it.each([
      [`(file://${PROJECT}/core/src/workspace/mod.rs#L748-L826)`, anchor('core/src/workspace/mod.rs', { start: 748, end: 826 })],
      [`file://${PROJECT}/src/a.rs#L12`, anchor('src/a.rs', { start: 12 })],
      [`file://localhost${PROJECT}/src/a.rs#L12`, anchor('src/a.rs', { start: 12 })],
      [`file://${PROJECT}/src/a.rs, next`, `${anchor('src/a.rs')}, next`],
      [`(file://${PROJECT}/src/a.rs).`, `${anchor('src/a.rs')}.`],
      [`See (file://${PROJECT}/src/a.rs#L1-L2) and [src/b.rs:3].`, `See ${anchor('src/a.rs', { start: 1, end: 2 })} and ${anchor('src/b.rs', { start: 3 })}.`],
    ])('shows %s as a citation of a file in the project', (text, body) => {
      expect(html(text, roots)).toBe(`<p>${body}</p>`);
    });

    it('uses the longer root when the file is in the copy', () => {
      expect(html(`file://${COPY}/src/a.rs`, roots)).toBe(`<p>${anchor('src/a.rs')}</p>`);
    });

    it('uses the project when the file is not in the copy', () => {
      expect(html(`file://${PROJECT}/src/b.rs`, roots)).toBe(`<p>${anchor('src/b.rs')}</p>`);
    });

    it('keeps the words of a file address written as a markdown link', () => {
      expect(html(`[the parser](file://${PROJECT}/src/parser.rs#L4-L9)`, roots)).toBe(
        `<p>${anchor('src/parser.rs', { start: 4, end: 9, label: 'the parser' })}</p>`,
      );
    });

    it.each([
      ['file:///etc/passwd'],
      [`file:///work/application/src/a.rs`],
      ['[x](file:///etc/passwd)'],
      ['[x](javascript:alert(1))'],
      ['[1]'],
      ['[the parser]'],
    ])('leaves %s as it was written', (text) => {
      expect(html(text, roots)).toBe(`<p>${text}</p>`);
    });

    it('leaves a file address as written when no directories were given', () => {
      expect(html(`file://${PROJECT}/src/a.rs`)).toBe(`<p>file://${PROJECT}/src/a.rs</p>`);
    });

    it('leaves a citation inside a code span as text', () => {
      expect(html('`[core/src/a.rs]` and `file:///etc/passwd`', roots)).toBe(
        '<p><code>[core/src/a.rs]</code> and <code>file:///etc/passwd</code></p>',
      );
    });

    it('leaves a citation inside a code block as text', () => {
      expect(renderMarkdown('```\nfile:///etc/passwd\n[src/a.rs](src/a.rs)\n```', roots)).toBe(
        '<pre><code>file:///etc/passwd\n[src/a.rs](src/a.rs)\n</code></pre>\n',
      );
    });

    it('round-trips the address a citation renders', () => {
      const rendered = html('[core/src/a.rs:12-40]');
      const href = /href="([^"]*)"/.exec(rendered)?.[1]?.replace(/&amp;/g, '&') ?? '';
      expect(citationFromHref(href)).toEqual({ path: 'core/src/a.rs', start: 12, end: 40 });
    });

    it.each([
      '#open?path=src%2Fparser.rs',
      '#open?path=src%2Fparser.rs&start=748&end=826',
      'http://localhost/#open?path=src%2Fparser.rs&start=4&end=4',
    ])('reads %s', (href) => {
      const citation = citationFromHref(href);
      expect(citation?.path).toBe('src/parser.rs');
    });

    it.each(['#open?path=..%2Fsecret', '#open?path=.git%2Fconfig', '#open?path=.GIT%2Fconfig', '#open?path=%2Fetc%2Fpasswd', '#open?path=src%5Ca.rs', '#open?path=.%2FREADME.md', '#open?path=src%2F..%2Fsecret', '#open?path=', '#heading', '#open?path=src%2Fparser.rs&start=0&end=1', '#open?path=src%2Fparser.rs&start=9&end=2'])(
      'reads nothing from %s',
      (href) => {
        expect(citationFromHref(href)).toBeNull();
      },
    );
  });

  describe('bare addresses', () => {
    it('links an address that says how to reach it', () => {
      expect(html('See https://example.com/docs.')).toBe(
        `<p>See <a href="https://example.com/docs" ${NEW_TAB}>https://example.com/docs</a>.</p>`,
      );
    });

    it('links an email address written as one', () => {
      expect(html('mailto:person@example.com')).toBe(
        `<p><a href="mailto:person@example.com" ${NEW_TAB}>mailto:person@example.com</a></p>`,
      );
    });

    it.each([['README.md'], ['main.rs'], ['setup.py'], ['www.example.com'], ['example.com'], ['person@example.com'], ['192.168.0.1']])(
      'does not guess that %s is a link',
      (text) => {
        expect(html(`Open ${text} now`)).toBe(`<p>Open ${text} now</p>`);
      },
    );

    it('does not link an address inside code', () => {
      expect(html('`https://example.com`')).toBe('<p><code>https://example.com</code></p>');
    });
  });

  describe('pictures', () => {
    it('shows a picture as a link named by its description', () => {
      expect(html('![A chart](https://example.com/chart.png)')).toBe(
        `<p><a href="https://example.com/chart.png" ${NEW_TAB}>A chart</a></p>`,
      );
    });

    it('names a picture without a description by its address', () => {
      expect(html('![](https://example.com/chart.png)')).toBe(
        `<p><a href="https://example.com/chart.png" ${NEW_TAB}>https://example.com/chart.png</a></p>`,
      );
    });

    it('shows a picture from the project as text that names its address', () => {
      expect(html('![Logo](assets/logo.png)')).toBe('<p><span title="assets/logo.png">Logo</span></p>');
    });

    it('shows a picture that holds its own data as text alone', () => {
      expect(html('![Dot](data:image/png;base64,iVBORw0KGgo=)')).toBe('<p><span>Dot</span></p>');
    });

    it('shows a picture inside a link as that link', () => {
      expect(html('[![Build](https://example.com/badge.svg)](https://example.com/builds)')).toBe(
        `<p><a href="https://example.com/builds" ${NEW_TAB}>Build</a></p>`,
      );
    });

    it('shows text around a picture', () => {
      expect(html('before ![A chart](https://example.com/c.png) after')).toBe(
        `<p>before <a href="https://example.com/c.png" ${NEW_TAB}>A chart</a> after</p>`,
      );
    });

    it('shows HTML in a description as text', () => {
      expect(html('![<b>x</b>](https://example.com/c.png)')).toBe(
        `<p><a href="https://example.com/c.png" ${NEW_TAB}>&lt;b&gt;x&lt;/b&gt;</a></p>`,
      );
    });

    it.each([
      ['![x](https://example.com/?secret=1)'],
      ['![x](x.png)'],
      ['[![x](https://example.com/a.png)](https://example.com)'],
      ['![x][1]\n\n[1]: https://example.com/a.png'],
      ['<img src="https://example.com/a.png">'],
    ])('never asks the browser to fetch %s', (text) => {
      expect(html(text)).not.toContain('<img');
    });
  });

  it('renders an answer the way an agent writes one', () => {
    const answer = [
      'I could not create `x.txt` — both attempts were blocked.',
      '',
      '- **File write:** denied with "you haven\'t granted it yet".',
      '- **Shell redirect:** denied as well.',
      '',
      'No file exists yet.',
    ].join('\n');

    expect(html(answer)).toBe(
      '<p>I could not create <code>x.txt</code> — both attempts were blocked.</p>' +
        '<ul><li><strong>File write:</strong> denied with &quot;you haven\'t granted it yet&quot;.</li>' +
        '<li><strong>Shell redirect:</strong> denied as well.</li></ul>' +
        '<p>No file exists yet.</p>',
    );
  });
});
