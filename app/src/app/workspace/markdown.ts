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
 * - Only web and email addresses can be followed, and they open in a new tab.
 *   Agents often link to files in the project; such an address means nothing
 *   to the browser, so the link is shown as text that names its address when
 *   pointed at.
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
export function renderMarkdown(text: string): string {
  return markdown.render(text);
}

/** The addresses a person can follow from a thread. */
const FOLLOWABLE = /^(https?:\/\/|mailto:)/i;

const ALIGNMENT = /^text-align:(left|center|right)$/;

const markdown = new MarkdownIt({ breaks: true, linkify: true });
markdown.linkify.set({ fuzzyLink: false, fuzzyEmail: false, fuzzyIP: false });
markdown.core.ruler.push('teryon', adapt);

/** Applies the rules described on `renderMarkdown` to what markdown-it parsed. */
function adapt(state: StateCore): void {
  for (const block of state.tokens) {
    alignCell(block);
    if (block.children) block.children = withSafeLinks(block.children, state);
  }
}

function alignCell(token: Token): void {
  const alignment = ALIGNMENT.exec(String(token.attrGet('style') ?? ''));
  if (alignment) token.attrs = [['class', `align-${alignment[1]}`]];
}

/** The same run of text with pictures turned into links and every link settled. */
function withSafeLinks(tokens: readonly Token[], state: StateCore): Token[] {
  const shown: Token[] = [];
  let open: Token | null = null;

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
        settle(link, end);
        shown.push(link, label, end);
      }
      continue;
    }

    if (token.type === 'link_open') open = token;
    if (token.type === 'link_close' && open) {
      settle(open, token);
      open = null;
    }
    shown.push(token);
  }
  return shown;
}

/** Makes a link either one that opens in a new tab, or text that names its address. */
function settle(open: Token, close: Token): void {
  const address = String(open.attrGet('href') ?? '');
  if (FOLLOWABLE.test(address)) {
    open.attrSet('target', '_blank');
    open.attrSet('rel', 'noopener noreferrer');
    return;
  }
  open.tag = close.tag = 'span';
  // An address that holds the data itself can be enormous and tells a person nothing.
  open.attrs = address.startsWith('data:') ? null : [['title', address]];
}
