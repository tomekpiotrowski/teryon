/**
 * What Home shows: the threads that need the person, one word each, and what is running.
 *
 * Home answers two questions: do I start something, and which thread do I go
 * to next. So every item is one line and its only action is to open the
 * thread. Free of Angular so it can be tested on its own.
 */

import { AGENTS, changeCard, type Choice, choicesOf, type ModelLists, type Project, type ProjectGroup, type Starting, type Thread } from './workspace.model';

/** The word at the right of a thread on Home, which is also the order of the list. */
export type HomeWord = 'apply' | 'failed' | 'reply' | 'new';

/** The words in the order the list shows them. */
export const HOME_WORDS: readonly HomeWord[] = ['apply', 'failed', 'reply', 'new'];

/** How many threads Home lists before pointing to the board. */
export const HOME_LIMIT = 7;

/** How many running threads Home names before counting the rest. */
export const RUNNING_LIMIT = 3;

/** The changes whose card asks for a decision Teryon cannot make, so the thread counts as failed. */
const BLOCKED = ['Needs catching up', 'Unfinished', 'Nowhere to apply'];

/** Stages in which a finished turn is still a conversation, so the agent is waiting for an answer. */
const CONVERSING = ['talking', 'planning'];

/**
 * The word for a thread, or nothing when it needs nothing from the person.
 *
 * - `failed`: the last run failed or hit a usage limit, or the change cannot
 *   be applied as it is (overlaps newer work, left half way, nothing to apply to)
 * - `apply`: the change is ready to apply
 * - `reply`: the agent asked questions, or ended its turn in a conversation the person has not looked at
 * - `new`: finished with results the person has not seen
 *
 * A thread at work is never listed; it has its own line. A thread the person
 * put away is left off, and stays reachable from the menu and from search.
 * A finished thread is listed while its result is unseen, until the board
 * has shown that finish.
 */
export function homeWord(thread: Thread, project: Project | null = null): HomeWord | null {
  if (thread.put_away) return null;
  if (thread.status === 'running') return null;
  const finished = (thread.stage ?? 'talking') === 'done';
  const card = changeCard(thread, project);
  if (!finished) {
    if (thread.ended === 'failed' || thread.ended === 'limited') return 'failed';
    if (card && BLOCKED.includes(card.title)) return 'failed';
    if (thread.stage === 'ready' || card?.tone === 'ready') return 'apply';
    if (thread.asking === true) return 'reply';
    if (card?.tone === 'attention' && thread.unread) return 'reply';
    if (thread.unread) return CONVERSING.includes(thread.stage ?? 'talking') ? 'reply' : 'new';
    return null;
  }
  if (thread.done_seen === true) return null;
  return thread.unread ? 'new' : null;
}

/** One line of the list. */
export interface HomeRow {
  readonly thread: Thread;
  readonly word: HomeWord;
  /** The project's name. Empty when the list spans one project and the name would only repeat. */
  readonly project: string;
}

/** What Home lists. */
export interface HomeList {
  /** The first rows, up to the limit. */
  readonly rows: readonly HomeRow[];
  /** How many more there are, for `+N more`. */
  readonly more: number;
  /** Every thread that needs the person, for the count in the bar. */
  readonly total: number;
}

/** When a thread last moved, the newest first order of its group. */
function moved(thread: Thread): string {
  return thread.stage_since ?? thread.created_at;
}

/**
 * The threads that need the person, apply first, then failed, reply and new, newest first within each.
 *
 * Project names are given only when the threads come from more than one project.
 */
export function homeList(groups: readonly ProjectGroup[], limit = HOME_LIMIT): HomeList {
  const found = groups.flatMap((group) =>
    group.threads.flatMap((thread) => {
      const word = homeWord(thread, group.project);
      return word ? [{ thread, word, project: group.project.name }] : [];
    }),
  );
  found.sort(
    (a, b) => HOME_WORDS.indexOf(a.word) - HOME_WORDS.indexOf(b.word) || moved(b.thread).localeCompare(moved(a.thread)),
  );
  const several = new Set(found.map((row) => row.thread.project_id)).size > 1;
  const rows = found.map((row) => ({ ...row, project: several ? row.project : '' }));
  return { rows: rows.slice(0, limit), more: Math.max(0, rows.length - limit), total: rows.length };
}

/** A thread at work, for the line under the list. */
export interface RunningLine {
  readonly thread: Thread;
  readonly project: string;
  /** What it is doing, in a word, such as `changing`. */
  readonly doing: string;
}

/** What the running lines say about everything running: the first few, and how many more. */
export interface RunningLines {
  readonly lines: readonly RunningLine[];
  readonly more: number;
}

/** The threads at work, newest first. */
export function runningLines(groups: readonly ProjectGroup[], limit = RUNNING_LIMIT): RunningLines {
  const running = groups
    .flatMap((group) =>
      group.threads
        .filter((thread) => thread.status === 'running')
        .map((thread) => ({ thread, project: group.project.name, doing: changeCard(thread, group.project)?.title.toLowerCase() ?? 'working' })),
    )
    .sort((a, b) => moved(b.thread).localeCompare(moved(a.thread)));
  return { lines: running.slice(0, limit), more: Math.max(0, running.length - limit) };
}

/** What a person has chosen on Home for the next thread, remembered between visits. The agent is the workspace's, see `NewThreads`. */
export interface HomeChoice {
  readonly projectId: string | null;
}

export const NO_HOME_CHOICE: HomeChoice = { projectId: null };

/** The project the next thread starts in: the remembered one while it still exists, else the open one, else the first. */
export function startProject(projects: readonly Project[], choice: HomeChoice, open: Project | null): Project | null {
  return projects.find((project) => project.id === choice.projectId) ?? open ?? projects[0] ?? null;
}

/**
 * The choices on Home, in agent order, with each agent's default before its models.
 *
 * Only installed agents are offered, even when discovery has models for other agents.
 */
export function homeChoices(installed: readonly string[], models: ModelLists): Choice[] {
  return AGENTS.filter((agent) => installed.includes(agent)).flatMap((agent) => choicesOf(agent, models));
}

/**
 * The agent and model displayed on Home for the next thread.
 *
 * The person's pick takes precedence, even when its model is missing from discovery.
 * Otherwise Home shows the service's choice for the default workflow's first
 * stage. A fallback label leaves the actual choice to the service.
 */
export function startChoice(
  choices: readonly Choice[],
  picked: Choice | null,
  starting: Starting | null,
): Choice | null {
  if (!choices.length) return null;
  if (picked) return picked;

  if (starting?.type === 'provider') {
    const provider = choices.find((choice) => choice.agent === starting.agent);
    if (provider) return { agent: provider.agent, model: starting.model };
  }

  return choices.find((choice) => choice.agent === 'claude') ?? choices[0];
}

/** Reads what was remembered, ignoring anything that is not what this version writes. */
export function readHomeChoice(stored: string | null): HomeChoice {
  if (!stored) return NO_HOME_CHOICE;
  try {
    const value: unknown = JSON.parse(stored);
    if (typeof value !== 'object' || value === null) return NO_HOME_CHOICE;
    const { projectId } = value as Record<string, unknown>;
    return {
      projectId: typeof projectId === 'string' ? projectId : null,
    };
  } catch {
    return NO_HOME_CHOICE;
  }
}
