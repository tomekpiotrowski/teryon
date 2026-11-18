/**
 * Finds git's words in text meant for the person.
 *
 * Teryon uses git underneath, but a person never needs to know it: the
 * screens speak of changes, copies and applying them. Specs run every text
 * a person reads through this, so a git word slipping onto the screen fails.
 */

/** Each git word with the forms it takes in a sentence. */
const GIT_WORDS: Readonly<Record<string, string>> = {
  branch: 'branch(es)?',
  commit: 'commit(s|ted|ting)?',
  merge: 'merg(e|es|ed|ing)',
  rebase: 'rebas(e|es|ed|ing)',
  worktree: 'worktrees?',
  push: 'push(es|ed|ing)?',
  'pull request': 'pull requests?',
  head: 'head',
  checkout: 'checkouts?',
  stash: 'stash(es|ed|ing)?',
};

/** The git words in `text`, whole words only, whatever their case. */
export function gitWordsIn(text: string): string[] {
  return Object.entries(GIT_WORDS)
    .filter(([, forms]) => new RegExp(`\\b${forms}\\b`, 'i').test(text))
    .map(([word]) => word);
}
