/**
 * What Home shows: threads to decide, answer or read, and threads still at work.
 *
 * Home is where a thread is started and where its latest turn is handled, so
 * a person can reply, answer questions and apply a change without opening the
 * thread on its own screen. Free of Angular so it can be tested on its own.
 */

import {
  AGENTS,
  type AgentName,
  changeCard,
  type ChangeCard,
  type Choice,
  type EntryLine,
  firstStageChooses,
  type ModelLists,
  NO_WORKFLOWS,
  type Project,
  type ProjectGroup,
  resolveChoice,
  stageText,
  type Starting,
  type Thread,
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

/** How many running threads Home shows before pointing to the board. */
const PROGRESS_LIMIT = 7;

/** Which group a waiting thread belongs to. */
const SECTION_KIND: Readonly<Record<HomeWord, HomeSectionKind>> = {
  apply: 'decide',
  failed: 'decide',
  reply: 'answer',
  new: 'read',
};

/** The groups, in the order Home shows them. */
const SECTION_ORDER: readonly HomeSectionKind[] = ['decide', 'answer', 'read', 'progress'];

const SECTION_TITLE: Readonly<Record<HomeSectionKind, string>> = {
  decide: 'Decide',
  answer: 'Answer',
  read: 'Read',
  progress: 'In progress',
};

/** A group of threads on Home. */
export type HomeSectionKind = 'decide' | 'answer' | 'read' | 'progress';

/**
 * One thread on Home.
 *
 * `projectName` is blank on a waiting card when every waiting thread is in
 * the same project. A thread at work always names its project.
 */
export interface HomeCard {
  readonly thread: Thread;
  readonly project: Project;
  readonly section: HomeSectionKind;
  readonly word: HomeWord | 'running';
  readonly projectName: string;
  readonly subtitle: string;
}

/** One group Home draws, heading included. Empty groups are left out. */
export interface HomeSection {
  readonly kind: HomeSectionKind;
  readonly title: string;
  readonly cards: readonly HomeCard[];
}

/** Everything under the prompt: the groups, in order, and how many threads were left off. */
export interface HomeQueue {
  readonly sections: readonly HomeSection[];
  /** Every visible card, in the order the keys move through them. */
  readonly cards: readonly HomeCard[];
  /** Every thread that needs the person, the same count as `homeList`. */
  readonly waiting: number;
  readonly moreWaiting: number;
  readonly moreRunning: number;
}

/**
 * The threads Home shows, grouped into Decide, Answer, Read and In progress.
 *
 * Waiting threads keep the order and the cap of `homeList`, so the count in
 * the bar and the `+N more` link stay the ones Home already had. Running
 * threads follow, newest first, up to their own cap. A group with nothing
 * in it is left out, and the names of projects are decided the same way as
 * the old list: only when the waiting threads span more than one.
 */
export function homeQueue(groups: readonly ProjectGroup[], workflows: Workflows = NO_WORKFLOWS): HomeQueue {
  const listed = homeList(groups);
  const projects = new Map(groups.map((group) => [group.project.id, group.project]));
  const waiting = listed.rows.flatMap((row) => {
    const project = projects.get(row.thread.project_id);
    return project ? [cardOf(row.thread, project, SECTION_KIND[row.word], row.word, row.project, workflows)] : [];
  });
  const running = groups
    .flatMap((group) => group.threads.filter((thread) => thread.status === 'running').map((thread) => ({ thread, project: group.project })))
    .sort((a, b) => moved(b.thread).localeCompare(moved(a.thread)));
  const progress = running
    .slice(0, PROGRESS_LIMIT)
    .map(({ thread, project }) => cardOf(thread, project, 'progress', 'running', project.name, workflows));
  const cards = [...waiting, ...progress];
  const sections = SECTION_ORDER.flatMap((kind) => {
    const inSection = cards.filter((card) => card.section === kind);
    return inSection.length ? [{ kind, title: SECTION_TITLE[kind], cards: inSection }] : [];
  });
  return {
    sections,
    cards,
    waiting: listed.total,
    moreWaiting: listed.more,
    moreRunning: Math.max(0, running.length - PROGRESS_LIMIT),
  };
}

function cardOf(
  thread: Thread,
  project: Project,
  section: HomeSectionKind,
  word: HomeWord | 'running',
  projectName: string,
  workflows: Workflows,
): HomeCard {
  return {
    thread,
    project,
    section,
    word,
    projectName,
    subtitle: cardSubtitle(thread, project, changeCard(thread, project), stageText(workflows, thread)),
  };
}

/**
 * The one line under a thread's title.
 *
 * Decide reads the change (`Ready to apply · 2 files changed · fix-login`).
 * A failed run that never made a change says so in a sentence. Answer and
 * Read use the preview the service stored, then the plan's title, then the
 * stage, so a thread from before previews still says something. A thread at
 * work says what it is doing.
 */
export function cardSubtitle(thread: Thread, project: Project | null, change: ChangeCard | null, stage: string): string {
  if (thread.status === 'running') {
    if (!thread.change || !change) return 'working';
    const doing = change.title.toLowerCase();
    return change.note ? `${doing} · ${change.note}` : doing;
  }
  const word = homeWord(thread, project);
  if (word === 'apply' || word === 'failed') {
    if (change && thread.change) return [change.title, change.note, thread.change.name].filter(Boolean).join(' · ');
    if (thread.ended === 'limited') return 'Hit a usage limit';
    if (thread.ended === 'failed') return 'The run failed';
    if (change) return [change.title, change.note].filter(Boolean).join(' · ');
    return stage;
  }
  const preview = thread.preview?.trim();
  if (preview) return preview;
  const plan = thread.plan?.title?.trim();
  if (plan) return plan;
  return stage;
}

/**
 * Splits a conversation into what just happened and everything before it.
 *
 * The latest turn starts at the newest thing the person sent, or at the
 * newest review, since that is the work the card is about. With neither,
 * the whole conversation is the latest turn.
 */
export function splitLatestTurn(lines: readonly EntryLine[]): { earlier: readonly EntryLine[]; latest: readonly EntryLine[] } {
  let start = -1;
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    if (lines[index].kind === 'user' || lines[index].kind === 'review') {
      start = index;
      break;
    }
  }
  if (start < 0) return { earlier: [], latest: lines };
  return { earlier: lines.slice(0, start), latest: lines.slice(start) };
}

/** What Enter does on an open card. */
export type CardAction = 'apply' | 'catchUp' | 'questions' | 'reply' | 'markRead' | 'open';

/**
 * The action Enter takes on a card.
 *
 * A thread at work opens. A change ready to apply asks to apply it, and one
 * that overlaps newer work asks to catch up, when that command is on offer.
 * An empty command list does not withhold those: the list is a filter only
 * once it names something. Questions are focused when the agent is waiting
 * on them, and a finished result is marked read. Anything else goes to the reply.
 */
export function primaryCardAction(card: HomeCard, commands: readonly { id: string }[] = []): CardAction {
  const offered = (id: string) => commands.length === 0 || commands.some((command) => command.id === id);
  if (card.section === 'progress') return 'open';
  if (card.word === 'apply' && offered('change:apply')) return 'apply';
  if (card.word === 'failed' && card.thread.change?.state?.fit === 'overlaps' && offered('change:catchUp')) return 'catchUp';
  if (card.section === 'answer') return card.thread.asking === true ? 'questions' : 'reply';
  if (card.section === 'read') return 'markRead';
  return 'reply';
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
