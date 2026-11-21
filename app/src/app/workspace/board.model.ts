/**
 * The board: every thread as a card, in a column for its stage.
 *
 * With several agents at work, the thread list says which are running but
 * not where each piece of work stands. The board answers "what should I look
 * at next?" in one screen. It is for looking only: stages move as a result
 * of work, never by moving a card.
 *
 * Free of Angular, like `workspace.model.ts`, so it can be tested on its own.
 */

import {
  choiceLabel,
  type Project,
  stageLabel,
  stageOf,
  STAGES,
  type Stage,
  type Thread,
  threadChoice,
  threadLabel,
  defaultWorkflow,
  workflowOf,
  type Workflows,
} from './workspace.model';

/** How many finished threads the Done column shows, newest first. */
export const DONE_SHOWN = 10;

/** One thread on the board. */
export interface BoardCard {
  readonly thread: Thread;
  readonly title: string;
  /** The project's name, on the board of every project. Empty otherwise. */
  readonly project: string;
  /** Who works on it: the reviewer while a review runs, otherwise the thread's agent, as "Claude · Opus". */
  readonly choice: string;
  readonly live: boolean;
  /** Whether its agent waits for the person's answers. */
  readonly asking: boolean;
  /** The change's name and how many files it changed, such as `fix-login · 3 files`. Empty without a change. */
  readonly change: string;
  /** What the last review found, while the author hasn't worked on it since. Empty otherwise. */
  readonly findings: string;
  /** How many agents Teryon started by itself since the person last wrote, when any. Empty otherwise. */
  readonly rounds: string;
  /** Whether the thread has a plan. */
  readonly planned: boolean;
}

export interface BoardColumn {
  readonly stage: Stage;
  readonly label: string;
  readonly cards: readonly BoardCard[];
  /** How many threads the column holds but doesn't show: older finished ones. */
  readonly hidden: number;
}

/** The card the keys have reached: a column, and a row within it. */
export interface BoardCursor {
  readonly column: number;
  readonly row: number;
}

export const BOARD_START: BoardCursor = { column: 0, row: 0 };

/**
 * The columns of the board for one project, or for every project when `projectId` is null.
 *
 * Only the stages the visible threads' workflows use are columns, in the
 * fixed order, so a board of small changes has no review column. Within a
 * column the thread that moved there last comes first, so Done keeps what
 * was finished most recently, whenever it was started.
 */
export function boardColumns(
  threads: readonly Thread[],
  projects: readonly Project[],
  workflows: Workflows,
  projectId: string | null,
): BoardColumn[] {
  const visible = threads.filter((thread) => projectId === null || thread.project_id === projectId);
  const used = new Set<Stage>(
    visible.length ? visible.flatMap((thread) => workflowOf(workflows, thread).stages) : defaultWorkflow(workflows).stages,
  );
  for (const thread of visible) used.add(stageOf(thread));
  const names = new Map(projects.map((project) => [project.id, project.name]));
  return STAGES.filter((stage) => used.has(stage)).map((stage) => {
    const cards = visible
      .filter((thread) => stageOf(thread) === stage)
      .sort((a, b) => sinceOf(b).localeCompare(sinceOf(a)))
      .map((thread) => boardCard(thread, projectId === null ? (names.get(thread.project_id) ?? '') : ''));
    const shown = stage === 'done' ? cards.slice(0, DONE_SHOWN) : cards;
    return { stage, label: stageLabel(workflows, stage), cards: shown, hidden: cards.length - shown.length };
  });
}

/** When a thread moved to its stage; its creation for threads from before Teryon recorded it. */
function sinceOf(thread: Thread): string {
  return thread.stage_since ?? thread.created_at;
}

export function boardCard(thread: Thread, project: string): BoardCard {
  const reviewing = thread.status === 'running' && thread.reviewer?.reviewing === true;
  const choice = threadChoice(thread, reviewing ? 'reviewer' : 'author') ?? { agent: thread.agent, model: null };
  const change = thread.change;
  const files = change?.state?.files_changed;
  const verdict = thread.reviewer?.verdict;
  const rounds = thread.rounds ?? 0;
  return {
    thread,
    title: threadLabel(thread),
    project,
    choice: choiceLabel(choice),
    live: thread.status === 'running',
    asking: thread.status === 'idle' && thread.asking === true,
    change: change ? [change.name, files === undefined ? '' : files === 1 ? '1 file' : `${files} files`].filter(Boolean).join(' · ') : '',
    findings:
      verdict && !verdict.answered && !verdict.cut && verdict.findings !== null
        ? verdict.findings === 1
          ? '1 finding'
          : `${verdict.findings} findings`
        : '',
    rounds: rounds === 0 ? '' : rounds === 1 ? '1 round without you' : `${rounds} rounds without you`,
    planned: Boolean(thread.plan),
  };
}

/**
 * Where the keys go: `h` and `l` between columns, `j` and `k` within one.
 *
 * Moving to another column keeps the row when it can, and stops at the last
 * card of a shorter column. The ends don't go round: the board is a grid,
 * and going round would lose the person's sense of where they are.
 */
export function movedCursor(columns: readonly BoardColumn[], cursor: BoardCursor, key: 'h' | 'j' | 'k' | 'l'): BoardCursor {
  if (!columns.length) return BOARD_START;
  const at = settledCursor(columns, cursor);
  const rows = (column: number) => columns[column].cards.length;
  switch (key) {
    case 'h':
    case 'l': {
      const column = Math.min(Math.max(at.column + (key === 'l' ? 1 : -1), 0), columns.length - 1);
      return { column, row: Math.min(at.row, Math.max(rows(column) - 1, 0)) };
    }
    case 'j':
    case 'k':
      return { column: at.column, row: Math.min(Math.max(at.row + (key === 'j' ? 1 : -1), 0), Math.max(rows(at.column) - 1, 0)) };
  }
}

/** The cursor kept inside the board, after its threads changed under it. */
export function settledCursor(columns: readonly BoardColumn[], cursor: BoardCursor): BoardCursor {
  if (!columns.length) return BOARD_START;
  const column = Math.min(Math.max(cursor.column, 0), columns.length - 1);
  return { column, row: Math.min(Math.max(cursor.row, 0), Math.max(columns[column].cards.length - 1, 0)) };
}

/** The thread under the cursor, or nothing in an empty column. */
export function cardAt(columns: readonly BoardColumn[], cursor: BoardCursor): Thread | null {
  const at = settledCursor(columns, cursor);
  return columns[at.column]?.cards[at.row]?.thread ?? null;
}

/** The cursor that starts on a thread, such as the one open before the board, or at the start when it isn't there. */
export function cursorOn(columns: readonly BoardColumn[], threadId: string | null): BoardCursor {
  for (const [column, candidate] of columns.entries()) {
    const row = candidate.cards.findIndex((card) => card.thread.id === threadId);
    if (row >= 0) return { column, row };
  }
  return BOARD_START;
}
