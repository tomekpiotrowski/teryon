/**
 * The keys shown in the shortcuts dialog, grouped by where they work.
 *
 * Commands supply navigation and thread actions. Screen controls that
 * handle their own keys are listed here with the conditions they need.
 */
import { CHANGE_LEADER, type Command, rankedMatches, SHORTCUTS_KEY } from './commands.model';
import type { Screen } from './workspace.model';

export interface ShortcutRow {
  readonly keys: readonly string[];
  readonly text: string;
}

export interface ShortcutSection {
  readonly heading: string;
  readonly rows: readonly ShortcutRow[];
}

type ShortcutPlace = 'home' | Screen;

interface ScreenKeys {
  readonly waitingThreads?: boolean;
  readonly usageAgents?: number;
}

const PLACE_NAMES: Readonly<Record<ShortcutPlace, string>> = {
  home: 'Home',
  thread: 'Thread',
  board: 'Board',
  history: 'History',
  files: 'Code',
  usage: 'Usage',
  settings: 'Settings',
  plan: 'Plan',
};

const BACK: ShortcutRow = { keys: ['esc'], text: 'Back' };
const HOME_LIST: readonly ShortcutRow[] = [
  { keys: ['tab'], text: 'Switch between prompt and list' },
  { keys: ['j', 'k'], text: 'Move through the list' },
];
const BOARD: readonly ShortcutRow[] = [
  { keys: ['h', 'j', 'k', 'l'], text: 'Move between cards' },
  { keys: ['↵'], text: 'Open the selected thread' },
];
const HISTORY: readonly ShortcutRow[] = [
  { keys: ['j', 'k'], text: 'Move between commits' },
  BACK,
];

const EVERYWHERE: readonly ShortcutRow[] = [
  { keys: ['/', 'Ctrl+K'], text: 'Open the menu to search, jump or start a thread' },
  { keys: ['esc'], text: 'Back or close' },
  { keys: [SHORTCUTS_KEY], text: 'Show shortcuts' },
];

export const SHARED_KEYS: readonly ShortcutRow[] = [
  { keys: ['/'], text: 'menu' },
  { keys: ['esc'], text: 'back or close' },
  { keys: [CHANGE_LEADER], text: 'this thread’s changes' },
  { keys: ['p'], text: 'plan' },
  { keys: [SHORTCUTS_KEY], text: 'this list' },
];

export function shortcutSections(
  commands: readonly Command[],
  place: ShortcutPlace,
  badge: string,
  screenKeys: ScreenKeys = {},
): ShortcutSection[] {
  const here = hereRows(commands, place, screenKeys);
  const goTo = commands.flatMap((command) => {
    const action = command.action;
    if (action.kind !== 'showHome' && action.kind !== 'showBoard' && action.kind !== 'showScreen' && action.kind !== 'openProject') return [];
    const row = commandRow(command);
    if (!row) return [];
    // On the board, lowercase h belongs to the cards. Shift+H still opens History.
    if (place === 'board' && command.leader === null && command.key === 'h') return [{ ...row, keys: ['H'] }];
    if (action.kind === 'showScreen' && action.screen === 'files') return [{ ...row, text: 'Code' }];
    return [row];
  });
  const name = (place === 'thread' && badge) || PLACE_NAMES[place];
  return [
    ...(here.length ? [{ heading: `Here · ${name}`, rows: here }] : []),
    { heading: 'Everywhere', rows: EVERYWHERE },
    ...(goTo.length ? [{ heading: 'Go to', rows: goTo }] : []),
  ];
}

function hereRows(commands: readonly Command[], place: ShortcutPlace, screenKeys: ScreenKeys): readonly ShortcutRow[] {
  switch (place) {
    case 'thread':
      return [
        ...commands.flatMap((command) => {
          if (command.group !== 'change' && command.action.kind !== 'answerQuestions' &&
              !(command.action.kind === 'showScreen' && command.action.screen === 'plan')) return [];
          const row = commandRow(command);
          return row ? [row] : [];
        }),
        { keys: ['esc'], text: 'Back to Home' },
      ];
    case 'home': {
      if (!commands.some((command) => command.action.kind === 'openProject' && command.key !== null)) return [];
      return [
        ...(screenKeys.waitingThreads ? HOME_LIST : []),
        { keys: ['↵'], text: screenKeys.waitingThreads ? 'Open or run' : 'Run' },
        { keys: ['@'], text: 'Continue a thread' },
      ];
    }
    case 'board':
      return [
        ...BOARD,
        ...(commands.some((command) => command.id === 'action:boardEvery')
          ? [{ keys: ['a'], text: 'Switch between this project and every project' }]
          : []),
        BACK,
      ];
    case 'history':
      return HISTORY;
    case 'usage': {
      const keys = Array.from({ length: Math.min(screenKeys.usageAgents ?? 0, 9) }, (_, index) => String(index + 1));
      return keys.length ? [{ keys, text: 'Pick the agent shown next to the number' }, BACK] : [BACK];
    }
    default:
      return [BACK];
  }
}

function commandRow(command: Command): ShortcutRow | null {
  if (command.key === null) return null;
  return {
    keys: command.leader === null ? [command.key] : [command.leader, command.key],
    text: command.title,
  };
}

/** Filters each section in place, with the descriptions ranked ahead of keys. */
export function filteredSections(sections: readonly ShortcutSection[], typed: string): ShortcutSection[] {
  return sections.flatMap((section) => {
    const rows = rankedMatches(section.rows, typed, (row) => ({ title: row.text, detail: row.keys.join(' ') }));
    return rows.length ? [{ ...section, rows }] : [];
  });
}
