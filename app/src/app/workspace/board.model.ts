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
}

/** A finish the board rendered during this visit, and when its card was last shown. */
export interface SeenFinish {
  readonly id: string;
  readonly at: string;
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
 * Only the stages the threads' workflows use are columns, in the fixed
 * order, including threads that are off the board, so putting everything
 * away does not collapse the columns. A put-away thread has no card, and
 * neither does a finish already shown. Done is left out when it has no
 * cards. Within a column the thread that moved there last comes first.
 */
export function boardColumns(
  threads: readonly Thread[],
  projects: readonly Project[],
  workflows: Workflows,
  projectId: string | null,
): BoardColumn[] {
  const inScope = threads.filter((thread) => projectId === null || thread.project_id === projectId);
  const used = new Set<Stage>(
    inScope.length ? inScope.flatMap((thread) => workflowOf(workflows, thread).stages) : defaultWorkflow(workflows).stages,
  );
  for (const thread of inScope) used.add(stageOf(thread));
  const names = new Map(projects.map((project) => [project.id, project.name]));
  return STAGES.filter((stage) => used.has(stage))
    .map((stage) => {
      const cards = inScope
        .filter((thread) => onBoard(thread) && stageOf(thread) === stage)
        .sort((a, b) => sinceOf(b).localeCompare(sinceOf(a)))
        .map((thread) => boardCard(thread, projectId === null ? (names.get(thread.project_id) ?? '') : ''));
      return { stage, label: stageLabel(workflows, stage), cards };
    })
    .filter((column) => column.stage !== 'done' || column.cards.length > 0);
}

/** Whether a thread still has a card. A put-away thread, and a finish already shown, do not. */
function onBoard(thread: Thread): boolean {
  return !thread.put_away && !(stageOf(thread) === 'done' && thread.done_seen === true);
}

/**
 * The finishes to remember after this view of the board.
 *
 * A card in the Done column is stored with `now`. One that left Done is
 * forgotten. One still Done but off this screen keeps the earlier time,
 * unless it finished again after that time, in which case it waits until
 * its card is rendered again. Instants are compared as times, because the
 * service and the browser do not write the same text for one moment.
 */
export function rememberedFinishes(
  remembered: readonly SeenFinish[],
  threads: readonly Thread[],
  columns: readonly BoardColumn[],
  now: string,
): SeenFinish[] {
  const shown = new Set(
    columns.filter((column) => column.stage === 'done').flatMap((column) => column.cards.map((card) => card.thread.id)),
  );
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  const next = new Map<string, string>();
  for (const finish of remembered) {
    const thread = byId.get(finish.id);
    if (!thread || stageOf(thread) !== 'done' || shown.has(finish.id)) continue;
    const entered = Date.parse(thread.stage_since ?? thread.created_at);
    const seen = Date.parse(finish.at);
    if (Number.isNaN(entered) || Number.isNaN(seen) || entered > seen) continue;
    next.set(finish.id, finish.at);
  }
  for (const id of shown) next.set(id, now);
  return [...next].map(([id, at]) => ({ id, at }));
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
