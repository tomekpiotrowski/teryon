/**
 * What Home shows: the threads that need the person, one word each, and what is running.
 *
 * Home answers two questions: do I start something, and which thread do I go
 * to next. So every item is one line and its only action is to open the
 * thread. Free of Angular so it can be tested on its own.
 */

import {
  AGENTS, type AgentName, changeCard, type Choice, firstStageChooses, type ModelLists, resolveChoice, type Project, type ProjectGroup, type Starting, type Thread,
  type Workflows,
} from './workspace.model';

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
 * The default choice of every installed agent on Home.
 *
 * Only installed agents are offered, even when discovery has models for other agents.
 */
export function homeChoices(installed: readonly string[], models: ModelLists): Choice[] {
  return AGENTS.filter((agent) => installed.includes(agent)).map((agent) => resolveChoice({
    agent, model: null, effort: null,
  }, models));
}

/** How Home's agent chooser looks, and what a new thread started from Home asks for. */
export interface HomeAgentState {
  /** Whether the chooser offers Automatic: the default workflow's first stage has a rule. */
  readonly allowAutomatic: boolean;
  /** Whether the first stage chooses the next thread's agent, rather than the person. */
  readonly automatic: boolean;
  /** The agent and model the next thread starts with, when known. */
  readonly choice: Choice | null;
  /** Why no agent can work under the first stage's rule. Nothing while one can. */
  readonly unavailableReason: string | null;
  /** What a thread started now sends: nothing lets the service use its held automatic pick. */
  readonly sendChoice: Choice | null;
}

/**
 * The agent Home displays and starts the next thread with.
 *
 * The button and the request are worked out together, so Home never shows
 * one mode while sending the other.
 *
 * A pick of an installed agent lasts until the person chooses Automatic.
 * While the first stage has a rule and nobody picked, or the picked agent is
 * no longer installed, the service's held pick is shown and nothing is sent,
 * so the service starts the thread on that pick and chooses afresh for the
 * next one. Without a rule, the pick, else Claude, else the first installed
 * agent, is both shown and sent.
 */
export function homeAgentState(
  installed: readonly string[],
  models: ModelLists,
  workflows: Workflows,
  picked: Choice | null,
  starting: Starting | null,
): HomeAgentState {
  const choices = homeChoices(installed, models);
  const allowAutomatic = firstStageChooses(workflows);
  const manual = picked && choices.some((choice) => choice.agent === picked.agent) ? resolveChoice(picked, models) : null;
  if (allowAutomatic && !manual) {
    const choice = starting?.type === 'provider'
      ? resolveChoice({ agent: starting.agent as AgentName, model: starting.model, effort: starting.effort }, models)
      : null;
    const unavailableReason = starting?.type === 'unavailable' ? starting.problem : null;
    return { allowAutomatic, automatic: true, choice, unavailableReason, sendChoice: null };
  }
  const choice = manual ?? choices.find((choice) => choice.agent === 'claude') ?? choices[0] ?? null;
  return { allowAutomatic, automatic: false, choice, unavailableReason: null, sendChoice: choice };
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
