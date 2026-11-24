/**
 * The shapes the workspace screen works with, and the plain logic around them.
 *
 * Everything here is free of Angular and Ionic so it can be tested on its own:
 * the screen itself cannot be unit tested until Vitest can load Ionic.
 */

import type { PlanInfo, Question, RunEnd, Stage, Starting, Workflow, Workflows } from '../service/protocol';

export type { PlanInfo, Stage, Starting, Workflow, Workflows };
import type { Asked } from './questions.model';

export type AgentName = 'claude' | 'codex' | 'grok' | 'gemini';

/** Every agent, in the order Teryon offers them. */
export const AGENTS: readonly AgentName[] = ['claude', 'codex', 'grok', 'gemini'];

export interface Project {
  readonly id: string;
  readonly name: string;
  readonly directory: string;
  readonly created_at: string;
  /** Whether it can be worked in: its directory may have gone, or another Teryon may have it open. */
  readonly state?: 'ready' | 'missing' | 'in_use';
  /** Whether it lives on GitHub, so its changes can be sent for review there. */
  readonly on_github?: boolean;
}

export interface Thread {
  readonly id: string;
  readonly project_id: string;
  readonly title: string;
  readonly agent: AgentName;
  /** The model the agent runs on. Nothing leaves it to the agent program's own settings. */
  readonly model?: string | null;
  readonly status: 'idle' | 'running';
  /** Whether an agent stopped working here since the person last looked. */
  readonly unread: boolean;
  /** Whether the thread's agent asked questions the person hasn't answered yet. Any message from them clears it. */
  readonly asking?: boolean;
  /** The names of the slash commands the agent program last said it takes. Empty until it has run here. */
  readonly commands: readonly string[];
  /** The change the thread's agent is working on, in its own copy of the project. Nothing until it starts one. */
  readonly change: Change | null;
  /** How the thread's last change ended. Nothing while one is under way, or before the first. */
  readonly finished: Finish | null;
  /** The second agent, reviewing the change. Nothing until a review is asked for. */
  readonly reviewer: Reviewer | null;
  /** The workflow the thread follows, by name. Empty means the default. */
  readonly workflow?: string;
  /** Where the thread's work stands. Missing on threads from before stages, which count as talking. */
  readonly stage?: Stage;
  /** When the thread moved to its stage. Missing on threads from before Teryon recorded it. */
  readonly stage_since?: string | null;
  /** What Teryon last saw of the thread's plan. Nothing until its agent writes one. */
  readonly plan?: PlanInfo | null;
  /** How the thread's last run ended. Nothing before one ends, and while a new one runs. */
  readonly ended?: RunEnd | null;
  /** How many agents Teryon started by itself since the person last wrote. */
  readonly rounds?: number;
  /** Whether the person stopped the thread, so Teryon starts nothing by itself until they write again. */
  readonly held?: boolean;
  /** Whether the person put the thread away, off the board and off Home until they pick it up. */
  readonly put_away?: boolean;
  /** Whether this finish has been shown on the board. A seen finish stays off the board and off Home. */
  readonly done_seen?: boolean;
  readonly created_at: string;
}

/** The stages, in the order work goes through them. */
export const STAGES: readonly Stage[] = ['talking', 'planning', 'implementing', 'review', 'ready', 'done'];

/** What each stage is called, until the service says otherwise. */
const USUAL_STAGE_LABELS: Readonly<Record<Stage, string>> = {
  talking: 'Talking',
  planning: 'Planning',
  implementing: 'Implementing',
  review: 'In review',
  ready: 'Ready to apply',
  done: 'Done',
};

/** The usual word for a stage, before anyone renames it. */
export function usualStageLabel(stage: Stage): string {
  return USUAL_STAGE_LABELS[stage];
}

/** The workflows before the service has said which there are. */
export const NO_WORKFLOWS: Workflows = {
  default: 'default',
  workflows: [{ name: 'default', description: '', stages: [...STAGES] }],
  labels: STAGES.map((stage) => ({ stage, label: USUAL_STAGE_LABELS[stage], chooses: null })),
};

/** What a person reads for a stage, as `config.toml` may have relabelled it. */
export function stageLabel(workflows: Workflows, stage: Stage): string {
  return workflows.labels.find((label) => label.stage === stage)?.label ?? USUAL_STAGE_LABELS[stage];
}

/**
 * What the thread's stage button reads.
 *
 * A put-away thread is still in its stage. The button says so, and still
 * opens the board, where the thread is absent until the person writes in it.
 */
export function stageText(workflows: Workflows, thread: Thread): string {
  const label = stageLabel(workflows, stageOf(thread));
  return thread.put_away ? `${label} · put away` : label;
}

/**
 * Whether a thread is a quiet conversation that can leave the board.
 *
 * Quiet means it is still just being talked through: idle, not waiting for
 * answers, and holding no change or plan. One already put away is not quiet
 * again. Unread, and one the person stopped, still are.
 */
export function isQuiet(thread: Thread): boolean {
  return (
    stageOf(thread) === 'talking' &&
    thread.status === 'idle' &&
    thread.asking !== true &&
    !thread.change &&
    !thread.plan &&
    thread.put_away !== true
  );
}

/**
 * The confirmation for putting quiet threads away.
 *
 * It names how many, and where, and says they stay in the menu. Putting
 * away is reversible by writing, so it is not phrased as destruction.
 */
export function putAwayText(count: number, scope: string): string {
  const threads = count === 1 ? '1 thread' : `${count} threads`;
  return `Put away ${threads} in ${scope}? They leave the board and stay in the menu. Writing in one brings it back.`;
}

/** The stage a thread is in, counting threads from before stages as talking. */
export function stageOf(thread: Thread): Stage {
  return thread.stage ?? 'talking';
}

/** A thread's workflow; one no longer set up, or none, means the default. */
export function workflowOf(workflows: Workflows, thread: Thread): Workflow {
  return workflows.workflows.find((workflow) => workflow.name === thread.workflow) ?? defaultWorkflow(workflows);
}

/** The workflow new threads start on. */
export function defaultWorkflow(workflows: Workflows): Workflow {
  return workflows.workflows.find((workflow) => workflow.name === workflows.default) ?? workflows.workflows[0] ?? NO_WORKFLOWS.workflows[0];
}

/** Which of a thread's two agents something belongs to: the one making the change, or the one reviewing it. */
export type Voice = 'author' | 'reviewer';

/**
 * The agent reviewing a thread's change, in a conversation of its own.
 *
 * Its lines are in the same thread as the author's, set apart, so that all
 * the work on one change stays in one place.
 */
export interface Reviewer {
  readonly agent: AgentName;
  /** The model the reviewer runs on. Nothing leaves it to the agent program's own settings. */
  readonly model?: string | null;
  /** Whether the turn running in the thread is the reviewer's. */
  readonly reviewing: boolean;
  /** What the last review found. Nothing until one has ended. */
  readonly verdict: Verdict | null;
}

export interface Verdict {
  /** How many problems the reviewer counted. Nothing when it did not say. */
  readonly findings: number | null;
  /** Whether the author has worked on the change since, so the review may no longer fit it. */
  readonly answered: boolean;
  /** Whether the review ended before the reviewer finished it, so it may be missing findings. */
  readonly cut: boolean;
}

/** The agent that reviews a thread's change: the one already reviewing, or the first other agent. */
export function reviewerOf(thread: Thread): AgentName {
  return reviewersFor(thread)[0];
}

/**
 * The agents that may review a thread's change, the usual one first.
 *
 * Any agent but the thread's own, so a second model reads the work. The
 * one that reviewed before comes first: asking it again continues its
 * conversation, so it remembers what it found.
 */
export function reviewersFor(thread: Thread): AgentName[] {
  const others = AGENTS.filter((agent) => agent !== thread.agent);
  const previous = thread.reviewer?.agent;
  return previous && others.includes(previous) ? [previous, ...others.filter((agent) => agent !== previous)] : others;
}

/**
 * An agent and the model it runs on, as one choice: "Claude · Opus".
 *
 * A model of `null` is the agent's default: no model is passed, and the
 * agent program's own settings decide.
 */
export interface Choice {
  readonly agent: AgentName;
  readonly model: string | null;
}

/** The models each agent offers besides its default, as the service lists them. */
export type ModelLists = Readonly<Record<AgentName, readonly string[]>>;

export const NO_MODELS: ModelLists = { claude: [], codex: [], grok: [], gemini: [] };

/**
 * What new threads start with: the agent, and the model last chosen for each agent.
 *
 * Remembering a model per agent means switching from Claude · Opus to Codex
 * and back gives Opus again. A review uses the model remembered for the
 * reviewing agent.
 */
export interface NewThreads {
  readonly agent: AgentName;
  readonly models: Readonly<Record<AgentName, string | null>>;
  /**
   * Whether the person picked the agent.
   *
   * Until they do, the service lets the workflow's first stage choose, as
   * `config.toml` says, and Claude on its default model when it says nothing.
   */
  readonly picked: boolean;
}

export const FIRST_NEW_THREADS: NewThreads = {
  agent: 'claude',
  models: { claude: null, codex: null, grok: null, gemini: null },
  picked: false,
};

/** The agent and model to start a new thread with, or nothing to let the first stage choose. */
export function startingChoice(newThreads: NewThreads): Choice | null {
  return newThreads.picked ? chosen(newThreads) : null;
}

/** The agent and model a new thread would start with: the person's pick, otherwise what the service chose. */
export function pendingChoice(newThreads: NewThreads, starting: Starting | null): Choice {
  if (!newThreads.picked && starting?.type === 'provider') return { agent: starting.agent as AgentName, model: starting.model };
  return chosen(newThreads);
}

/** Whether the workflow's first stage chooses the agent of new threads. */
export function firstStageChooses(workflows: Workflows): boolean {
  const first = defaultWorkflow(workflows).stages[0] ?? 'talking';
  return Boolean(workflows.labels.find((label) => label.stage === first)?.chooses);
}

/** The agent and model a new thread starts with. */
export function chosen(newThreads: NewThreads): Choice {
  return { agent: newThreads.agent, model: newThreads.models[newThreads.agent] };
}

/** New threads after picking a choice: its agent, with its model remembered for that agent. */
export function choosing(newThreads: NewThreads, choice: Choice): NewThreads {
  return { agent: choice.agent, models: { ...newThreads.models, [choice.agent]: choice.model }, picked: true };
}

/** Reads what was remembered of the choice for new threads, ignoring anything this version does not write. */
export function readNewThreads(stored: string | null): NewThreads {
  if (!stored) return FIRST_NEW_THREADS;
  try {
    const value: unknown = JSON.parse(stored);
    if (typeof value !== 'object' || value === null) return FIRST_NEW_THREADS;
    const { agent, models, picked } = value as Record<string, unknown>;
    if (typeof agent !== 'string' || !(AGENTS as readonly string[]).includes(agent) || typeof picked !== 'boolean') return FIRST_NEW_THREADS;
    const remembered = typeof models === 'object' && models !== null ? (models as Record<string, unknown>) : {};
    const model = (name: AgentName): string | null => (typeof remembered[name] === 'string' ? (remembered[name] as string) : null);
    return {
      agent: agent as AgentName,
      picked,
      models: { claude: model('claude'), codex: model('codex'), grok: model('grok'), gemini: model('gemini') },
    };
  } catch {
    return FIRST_NEW_THREADS;
  }
}

/** Every choice an agent offers: its default first, then its models. */
export function choicesOf(agent: AgentName, models: ModelLists): Choice[] {
  return [null, ...models[agent]].map((model) => ({ agent, model }));
}

/** Every choice of every agent, in the order agents are offered. */
export function allChoices(models: ModelLists): Choice[] {
  return AGENTS.flatMap((agent) => choicesOf(agent, models));
}

export function sameChoice(first: Choice, second: Choice): boolean {
  return first.agent === second.agent && first.model === second.model;
}

/** The choice a thread's agent, or its reviewer, runs on. Nothing when the thread has no reviewer. */
export function threadChoice(thread: Thread, voice: Voice): Choice | null {
  if (voice === 'author') return { agent: thread.agent, model: thread.model ?? null };
  return thread.reviewer ? { agent: thread.reviewer.agent, model: thread.reviewer.model ?? null } : null;
}

/** For example "Claude · Opus" or "Codex · default". */
export function choiceLabel(choice: Choice): string {
  return `${agentLabel(choice.agent)} · ${choice.model === null ? 'default' : modelLabel(choice.model)}`;
}

/**
 * A model's name as people say it: `claude-opus-5-5` is "Opus 5.5".
 *
 * A rule rather than a table, so that a model released after this Teryon
 * still reads well: drop a leading `claude-`, join numbers that follow each
 * other with a dot, turn the remaining dashes into spaces, capitalise the
 * first word, and write `gpt` as `GPT`. Several models, as a turn that used
 * more than one reports them, are each read that way.
 */
export function modelLabel(name: string): string {
  return name
    .split(' + ')
    .map((single) => {
      const words = single
        .trim()
        .replace(/^claude-/, '')
        .replace(/(\d)-(?=\d)/g, '$1.')
        .split('-')
        .filter(Boolean)
        .map((word) => (word.toLowerCase() === 'gpt' ? 'GPT' : word));
      return words.map((word, index) => (index === 0 ? word.charAt(0).toUpperCase() + word.slice(1) : word)).join(' ');
    })
    .join(' + ');
}

/** How a thread's change ended, once Teryon let go of it and its copy. */
export type Finish = 'applied' | 'accepted' | 'discarded' | 'removed';

/**
 * The work a thread's agent is doing in its own copy of the project.
 *
 * Underneath it is a git worktree on a branch, but nothing shown on screen
 * says so: a person sees the change, its name, and where it stands.
 */
export interface Change {
  readonly name: string;
  /** Where the thread's copy is on this machine. */
  readonly path: string;
  /** The branch the change started from. Nothing when the project was not on one. */
  readonly base: string | null;
  readonly start: string;
  /** The branch checked out in the copy now. Usually the change's name. */
  readonly branch: string | null;
  /** Nothing until Teryon has looked. */
  readonly state: ChangeState | null;
  /** The review it was sent for on GitHub. Nothing until it is sent. */
  readonly review: Review | null;
}

/** A change sent for review on GitHub. */
export interface Review {
  readonly number: number;
  /** Where the review is on GitHub. */
  readonly url: string;
  readonly state: 'in_review' | 'accepted' | 'declined';
}

export interface ChangeState {
  readonly copy_present: boolean;
  readonly base_present: boolean;
  /** Files that differ from where the change started, saved by the agent or not. */
  readonly files_changed: number;
  readonly fit: 'applies' | 'overlaps' | 'unknown';
  /** The agent left the copy half way through something only it can sort out. */
  readonly unfinished: boolean;
}

/** Which openers that need setting up are set up on the machine. */
export interface Opens {
  readonly editor: boolean;
  readonly terminal: boolean;
}

export const NO_OPENS: Opens = { editor: false, terminal: false };

export type EntryKind = 'user_message' | 'agent_message' | 'activity' | 'notice' | 'error' | 'turn_summary' | 'questions' | 'stage' | 'plan';

export interface ThreadEntry {
  readonly id: string;
  readonly thread_id: string;
  readonly position: number;
  readonly kind: EntryKind;
  readonly body: Readonly<Record<string, unknown>>;
  readonly created_at: string;
}

/** A project with its threads. */
export interface ProjectGroup {
  readonly project: Project;
  readonly threads: readonly Thread[];
  /** Whether an agent is working in any of the project's threads. */
  readonly running: boolean;
}

/** The project and the thread the person has picked. */
export interface Selection {
  readonly projectId: string | null;
  readonly threadId: string | null;
}

/** One line of a thread, ready to show. */
export interface EntryLine {
  /** `review` marks where a review starts; `stage` where the thread moved to another stage; `plan` where its plan was written or changed. */
  readonly kind: 'user' | 'agent' | 'activity' | 'notice' | 'review' | 'error' | 'summary' | 'questions' | 'stage' | 'plan';
  readonly text: string;
  /** Whether the reviewer wrote it, or the person wrote it to the reviewer. */
  readonly reviewer: boolean;
  /**
   * The agent that reviewed, on a reviewer's line.
   *
   * A thread may be reviewed by different agents in turn, so the current
   * reviewer does not say who wrote an earlier review. Missing on lines
   * stored before Teryon recorded it.
   */
  readonly agent?: AgentName;
  /** More about the line, shown when the pointer rests on it: the full name of a summary's model. */
  readonly title?: string;
  /** The questions on a line of questions the agent asked. */
  readonly asked?: Asked;
}

export const NOTHING_SELECTED: Selection = { projectId: null, threadId: null };

/** What fills the screen: a project's threads, its files or its history. */
export type Screen = 'thread' | 'files' | 'history' | 'usage' | 'board' | 'plan' | 'settings';

/**
 * Whether a screen is drawn over the thread, which then waits hidden underneath.
 *
 * The thread is kept on the page while another screen is shown, so coming back
 * finds it where it was left, scrolled as it was, without drawing every line
 * again. The files and the history need a project; without one the thread
 * shows instead.
 */
export function coversThread(screen: Screen | null, project: Project | null): boolean {
  switch (screen) {
    case 'settings':
    case 'usage':
    case 'plan':
    case 'board':
      return true;
    case 'files':
    case 'history':
      return project !== null;
    default:
      return false;
  }
}

/** A pressed key, as far as the shortcuts care. */
export interface KeyPress {
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly altKey: boolean;
  /** The name of the element the key was pressed in, such as `TEXTAREA`. */
  readonly element: string;
  /** Whether that element is one whose text can be edited in place. */
  readonly editable: boolean;
  /** Whether that element is a message box: the reply box of a thread, or Home's prompt. */
  readonly messageBox?: boolean;
}

/**
 * The part of a project's repository the files and the history show.
 *
 * Kept in one place so that both screens stay on the same branch, and so
 * that the history can send a person to the files as a commit left them.
 */
export interface Place {
  /** The project the choice was made in. A choice does not carry over to another project. */
  readonly projectId: string | null;
  /**
   * The thread whose copy the files show, read from its directory whatever it is on.
   *
   * A copy is named by its thread rather than its branch, because while the
   * agent rebases the copy is on no branch, and the branch alone holds only
   * what was committed on it.
   */
  readonly threadId: string | null;
  /** Nothing means whatever the project's directory is on. For a thread's copy, the branch the copy is on, for the history. */
  readonly branch: string | null;
  /** A commit shown in place of a branch, in the file browser only. */
  readonly commit: string | null;
}

export const NO_PLACE: Place = { projectId: null, threadId: null, branch: null, commit: null };

/** Projects oldest first, each with its threads newest first. */
export function groupThreads(projects: readonly Project[], threads: readonly Thread[]): ProjectGroup[] {
  return [...projects]
    .sort((a, b) => a.created_at.localeCompare(b.created_at))
    .map((project) => {
      const own = threads
        .filter((thread) => thread.project_id === project.id)
        .sort((a, b) => b.created_at.localeCompare(a.created_at));
      return { project, threads: own, running: own.some((thread) => thread.status === 'running') };
    });
}

/**
 * Finds what the selection points at.
 *
 * A selection can outlive what it names, because a project or thread can be
 * removed elsewhere. With nothing valid selected the first project is shown,
 * so the screen is never empty while there is something to show.
 */
export function resolveSelection(
  groups: readonly ProjectGroup[],
  selection: Selection,
): { readonly project: Project | null; readonly thread: Thread | null } {
  const threads = groups.flatMap((group) => group.threads);
  const thread = threads.find((candidate) => candidate.id === selection.threadId) ?? null;
  const projectId = thread?.project_id ?? selection.projectId;
  const group = groups.find((candidate) => candidate.project.id === projectId) ?? groups[0];
  return { project: group?.project ?? null, thread };
}

/**
 * Whether a key press is the given letter on its own, meant as a shortcut.
 *
 * The shortcuts are bare letters, so they must stay out of the way wherever
 * a letter is typed as text or picks an entry from a list.
 */
export function isBareLetter(press: KeyPress, letter: string): boolean {
  if (press.key.toLowerCase() !== letter || press.ctrlKey || press.metaKey || press.altKey) return false;
  return !press.editable && !['INPUT', 'TEXTAREA', 'SELECT'].includes(press.element.toUpperCase());
}

/** A key pressed in the message box, as far as the message box cares. */
export interface ComposerPress {
  readonly key: string;
  readonly shiftKey: boolean;
  /** Whether the key is part of a character still being composed, as with an input method for Japanese. */
  readonly isComposing: boolean;
}

/**
 * What a key pressed in the message box does: send the message, leave the box, or type as usual.
 *
 * Every letter typed in the box is text, so the keys bound to letters only
 * work outside it. Escape leaves the box, as it leaves typing in vim, so
 * the keyboard alone can get from writing a message to those keys. Enter
 * sends and Shift+Enter starts a new line. While a character is still
 * being composed both keys belong to the input method.
 */
export function composerAction(press: ComposerPress): 'send' | 'leave' | null {
  if (press.isComposing) return null;
  if (press.key === 'Escape') return 'leave';
  if (press.key === 'Enter' && !press.shiftKey) return 'send';
  return null;
}

/**
 * How a change looks at a glance, which colours its label.
 *
 * - `working`: the agent is still at it, or has not changed anything yet
 * - `ready`: waiting for the person to apply it or send it for review
 * - `waiting`: in review, waiting for someone else
 * - `attention`: stuck until the person or the agent does something
 * - `done`: in the project
 * - `quiet`: thrown away, or its copy removed
 */
export type ChangeTone = 'working' | 'ready' | 'waiting' | 'attention' | 'done' | 'quiet';

/**
 * Where the open thread's change stands, for the line above the message box.
 *
 * A person never needs to know that a change is a copy of the project on a
 * branch of its own: they see that the thread changed files, where that work
 * stands, and the likely next thing to do. Everything else is in the menu.
 */
export interface ChangeCard {
  /** A word or two, such as `Ready to apply`. */
  readonly title: string;
  /**
   * Whether the line shows the title.
   *
   * A change ready to apply goes without: its buttons say so already, and a
   * label that is there most of the time stops being read.
   */
  readonly labelled: boolean;
  /** A few words more, such as `2 files changed`. Can be empty. */
  readonly note: string;
  readonly tone: ChangeTone;
  /** The commands offered on the line, as the likely next thing to do. */
  readonly next: readonly string[];
}

/**
 * The card for a thread's change, or nothing when the thread never changed anything.
 *
 * The state is read in order of what matters most: a removed copy is
 * fine, since copies get removed for all kinds of reasons, so it is shown
 * calmly, or as in review when GitHub still has the work, and Teryon lets
 * go of the change once nothing of it is left; a change left half way
 * through blocks everything else; an agent at work
 * means the change is not ready whatever git says; then whether there is
 * anything to apply, whether it overlaps with newer work, and how a review
 * on GitHub is going. A change finished earlier is remembered on the
 * thread, so that work that went into the project still shows it did.
 */
export function changeCard(thread: Thread, project: Project | null): ChangeCard | null {
  const card = (title: string, note: string, tone: ChangeTone, next: readonly string[] = [], labelled = tone !== 'ready'): ChangeCard => ({
    title,
    labelled,
    note,
    tone,
    next,
  });
  const change = thread.change;
  if (!change) {
    switch (thread.finished) {
      case 'applied':
        return card('Applied', 'in the project', 'done');
      case 'accepted':
        return card('Accepted', 'on GitHub, in the project', 'done');
      case 'discarded':
        return card('Discarded', '', 'quiet');
      case 'removed':
        return card('Copy removed', '', 'quiet');
      default:
        return null;
    }
  }

  const state = change.state;
  if (!state) return card('Changing', '', 'working');
  if (!state.copy_present) {
    const review = change.review?.state;
    if (review === 'in_review' || review === 'accepted') return card('In review', 'on GitHub', 'waiting', ['change:openReview']);
    return card('Copy removed', '', 'quiet');
  }
  if (state.unfinished) return card('Unfinished', 'the agent left it half way', 'attention');
  const files = filesChanged(state.files_changed);
  const reviewer = agentLabel(reviewerOf(thread));
  if (thread.status === 'running' && thread.reviewer?.reviewing) return card('Reviewing', `${reviewer} is reading the change`, 'waiting');
  if (thread.status === 'running') return card('Changing', state.files_changed ? `${files} so far` : '', 'working');
  if (state.files_changed === 0) return card('Changing', 'no files changed yet', 'working');
  if (!state.base_present) return card('Nowhere to apply', 'the version it started from is gone', 'attention');
  if (state.fit === 'overlaps') return card('Needs catching up', 'overlaps with newer work', 'attention', ['change:catchUp']);
  switch (change.review?.state) {
    case 'in_review':
    case 'accepted':
      return card('In review', `${files}, on GitHub`, 'waiting', ['change:openReview']);
    case 'declined':
      return card('Declined', `${files}, on GitHub`, 'attention', ['change:openReview']);
  }
  const finish = project?.on_github ? ['change:apply', 'change:sendForReview'] : ['change:apply'];
  const verdict = thread.reviewer?.verdict;
  if (verdict && !verdict.answered) {
    if (verdict.cut) return card('Review stopped', `${reviewer} stopped before finishing`, 'attention', ['change:askReview', ...finish]);
    if (verdict.findings === 0) return card('Reviewed', `${files}, ${reviewer} found nothing`, 'ready', finish, true);
    return card(findingsCount(verdict.findings), `from ${reviewer}`, 'attention', ['change:passFindings', ...finish]);
  }
  return card('Ready to apply', files, 'ready', ['change:askReview', ...finish]);
}

/**
 * A word or two about a thread's change, for lists of threads, or nothing when it has none.
 *
 * Shown next to a thread in the menu, so that threads holding work not yet
 * in the project stand out from plain conversations.
 */
export function changeBadge(thread: Thread): string {
  const card = changeCard(thread, null);
  return card && card.tone !== 'quiet' ? card.title.toLowerCase() : '';
}

function findingsCount(count: number | null): string {
  if (count === null) return 'Findings';
  return count === 1 ? '1 finding' : `${count} findings`;
}

function filesChanged(count: number): string {
  return count === 1 ? '1 file changed' : `${count} files changed`;
}

/**
 * What discarding a change will lose, for the person to confirm.
 *
 * Counting the files makes plain how much is at stake, without listing them.
 */
export function discardWarning(change: Change): string {
  const files = change.state?.copy_present ? change.state.files_changed : 0;
  const lost = files === 0 ? '' : files === 1 ? ': 1 changed file' : `: ${files} changed files`;
  return `All changes in this thread will be lost${lost}. This cannot be undone.`;
}

/**
 * What the agent is asked when its change overlaps with newer work in the project.
 *
 * Put in the message box rather than sent, like every request Teryon drafts:
 * the person decides whether the agent should spend a turn on it.
 */
export const CATCH_UP_REQUEST =
  'Bring this change up to date with the newer work in the project, and resolve where the two overlap.';

/** The screen to show once one is asked for. Asking for the screen already shown goes back to the thread. */
export function screenAfter(current: Screen, asked: Exclude<Screen, 'thread'>): Screen {
  return current === asked ? 'thread' : asked;
}

/** The place chosen in a project, or the project's directory as it is when the choice was made elsewhere. */
export function placeIn(place: Place, projectId: string): Place {
  return place.projectId === projectId ? place : { projectId, threadId: null, branch: null, commit: null };
}

/** The copy where a thread's agent works, as a place to show. */
export function copyPlace(thread: Thread, change: Change): Place {
  return { projectId: thread.project_id, threadId: thread.id, branch: change.branch, commit: null };
}

/**
 * What the code screen shows when asked for: the open thread's copy, or else the place chosen in the project.
 *
 * A person opening the code from a thread wants the work of its agent, not
 * the project as it was before. A copy Teryon has not looked at yet counts
 * as there, the way the thread's change commands treat it. Once the copy is
 * gone, or for a thread that never had one, the last choice stands.
 */
export function codePlace(place: Place, projectId: string, thread: Thread | null): Place {
  const change = thread?.project_id === projectId ? thread.change : null;
  if (!thread || !change || change.state?.copy_present === false) return placeIn(place, projectId);
  return copyPlace(thread, change);
}

/** What the bar at the top names: the selected project, and the open thread after it. */
export function barTitle(project: Project | null, thread: Thread | null): string {
  if (!project) return '';
  return thread ? `${project.name} · ${threadLabel(thread)}` : project.name;
}

/** How many agents are working, across every project. */
export function runningCount(groups: readonly ProjectGroup[]): number {
  return groups.flatMap((group) => group.threads).filter((thread) => thread.status === 'running').length;
}

/**
 * Whether the person is looking at a thread that is still marked as unread.
 *
 * A thread behind the files or the history, or in a browser tab that is
 * not showing, has not been looked at.
 */
export function seenNow(thread: Thread | null, screen: Screen, visible: boolean): boolean {
  return thread !== null && thread.unread && thread.status === 'idle' && screen === 'thread' && visible;
}

/**
 * Whether a thread waits for the person: its agent stopped since they last looked, or asked questions they haven't answered.
 *
 * Questions keep a thread waiting after it was looked at, since only an answer settles them.
 */
export function needsPerson(thread: Thread): boolean {
  return !thread.put_away && thread.status === 'idle' && (thread.unread || thread.asking === true);
}

export function threadLabel(thread: Thread): string {
  return thread.title || 'New thread';
}

const AGENT_LABELS: Readonly<Record<AgentName, string>> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', gemini: 'Gemini' };

export function agentLabel(agent: AgentName): string {
  return AGENT_LABELS[agent];
}

/**
 * The lines a thread shows, from its stored entries.
 *
 * An agent that asks a second time in the same turn has rethought its
 * questions, so only the newer ones of each turn are shown.
 */
export function threadLines(entries: readonly ThreadEntry[]): EntryLine[] {
  const lines = entries.map(entryLine);
  return lines.filter((line, index) => !line.asked || !lines.slice(index + 1).some((later) => later.asked?.turn === line.asked?.turn));
}

/**
 * The questions waiting for the person's answers: the newest the agent asked, while nothing was said after them.
 *
 * Any message from the person after the questions answers them, whether
 * picked or written. Nothing while an agent works, since nobody can send then.
 */
export function pendingQuestions(lines: readonly EntryLine[], running: boolean): Asked | null {
  if (running) return null;
  const last = lines.map((line) => line.asked !== undefined).lastIndexOf(true);
  if (last < 0 || lines.slice(last + 1).some((line) => line.kind === 'user')) return null;
  return lines[last].asked ?? null;
}

/** Turns a stored entry into the line the thread shows. */
export function entryLine(entry: ThreadEntry): EntryLine {
  const line = plainLine(entry);
  const agent = entry.body['agent'];
  return line.reviewer && AGENTS.includes(agent as AgentName) ? { ...line, agent: agent as AgentName } : line;
}

/** Who a reviewer's line is from or to: the agent it names, or else the thread's reviewer. */
export function reviewerLabel(line: EntryLine, thread: Thread): string {
  return agentLabel(line.agent ?? reviewerOf(thread));
}

function plainLine(entry: ThreadEntry): EntryLine {
  const text = textOf(entry.body['text']);
  const reviewer = entry.body['reviewer'] === true;
  switch (entry.kind) {
    case 'user_message':
      return { kind: 'user', text, reviewer };
    case 'agent_message':
      return { kind: 'agent', text, reviewer };
    case 'activity':
      return { kind: 'activity', text: [textOf(entry.body['tool']), textOf(entry.body['summary'])].filter(Boolean).join('  '), reviewer };
    case 'notice':
      return { kind: entry.body['starts_review'] === true ? 'review' : 'notice', text, reviewer };
    case 'error':
      return { kind: 'error', text, reviewer };
    case 'turn_summary': {
      const model = entry.body['model'];
      const title = typeof model === 'string' && model ? { title: model } : {};
      return { kind: 'summary', text: summaryText(entry.body), reviewer, ...title };
    }
    case 'stage':
      return { kind: 'stage', text, reviewer };
    case 'plan':
      return { kind: 'plan', text, reviewer };
    case 'questions': {
      const asked = askedIn(entry);
      return asked ? { kind: 'questions', text: '', reviewer, asked } : { kind: 'notice', text: 'The agent asked something Teryon couldn’t read.', reviewer };
    }
  }
}

/** The questions an entry holds, or nothing when they aren't in the shape Teryon stores. */
function askedIn(entry: ThreadEntry): Asked | null {
  const turn = entry.body['turn'];
  const questions = entry.body['questions'];
  if (typeof turn !== 'string' || !Array.isArray(questions) || !questions.length || !questions.every(isQuestion)) return null;
  return { id: entry.id, turn, questions };
}

function isQuestion(value: unknown): value is Question {
  if (typeof value !== 'object' || value === null) return false;
  const { question, answers, choosing } = value as Record<string, unknown>;
  if (typeof question !== 'string' || !Array.isArray(answers) || !answers.length) return false;
  if (!answers.every((answer) => typeof answer === 'object' && answer !== null && typeof (answer as Record<string, unknown>)['label'] === 'string')) return false;
  if (typeof choosing !== 'object' || choosing === null) return false;
  const { kind, recommended } = choosing as Record<string, unknown>;
  const inRange = (position: unknown) => Number.isInteger(position) && (position as number) >= 0 && (position as number) < answers.length;
  if (kind === 'one') return inRange(recommended);
  return kind === 'several' && Array.isArray(recommended) && recommended.every(inRange);
}

/** The last line of a review, which counts its findings for Teryon, not for the person. */
const COUNT_LINE = /\n*\s*[*`]*Findings:\s*\d+[*`]*\s*$/;

/**
 * What the author is asked once the person passes the latest review on.
 *
 * The review is the reviewer's message since the last review started that
 * ends with a count, so an answer to a question the person asked in between
 * is not taken for it. Without a count, its first message is the review.
 *
 * Put in the message box rather than sent, like every request Teryon
 * drafts: the person may drop findings they disagree with. Nothing when the
 * reviewer has not written anything.
 */
export function findingsDraft(lines: readonly EntryLine[]): string {
  const start = lines.map((line) => line.kind).lastIndexOf('review');
  const written = lines.slice(start + 1).filter((line) => line.reviewer && line.kind === 'agent');
  const review = [...written].reverse().find((line) => COUNT_LINE.test(line.text)) ?? written[0];
  if (!review) return '';
  const findings = review.text.replace(COUNT_LINE, '').trim();
  return `A reviewer looked at the change and found:\n\n${findings}\n\nFix what you agree with, and say why for anything you don't.`;
}

/**
 * For example `Done in 1m 12s · Opus 5.5 · 86k tokens read, 229 written`. Parts the agent did not report are left out.
 *
 * The model is the one that ran, as the agent program reported it, so the
 * thread says which model did each turn even when the choice changed.
 *
 * Tokens rather than money: the agents run on subscriptions, where a turn
 * uses up part of a limit instead of costing a price.
 */
export function summaryText(body: Readonly<Record<string, unknown>>): string {
  const duration = body['duration_ms'];
  const tokens = body['tokens'] as Readonly<Record<string, unknown>> | null | undefined;
  const parts = [typeof duration === 'number' ? `Done in ${durationText(duration)}` : 'Done'];
  const model = body['model'];
  if (typeof model === 'string' && model.trim()) parts.push(modelLabel(model));
  const read = tokens?.['read'];
  const written = tokens?.['written'];
  if (typeof read === 'number' && typeof written === 'number') {
    parts.push(`${countText(read)} tokens read, ${countText(written)} written`);
  }
  return parts.join(' · ');
}

/** A count short enough to take in at a glance: `229`, `86k`, `1.2M`. */
export function countText(count: number): string {
  if (count < 1000) return `${count}`;
  if (count < 999_500) return `${Math.round(count / 1000)}k`;
  return `${Number((count / 1_000_000).toFixed(1))}M`;
}

export function durationText(milliseconds: number): string {
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** The name a refusal from the service carries, see `ServiceRefusal`. */
export const REFUSAL = 'ServiceRefusal';

/**
 * The reason the service gave for refusing a request, or a fallback.
 *
 * A refusal arrives as an error named `ServiceRefusal` whose message is
 * meant for the person. Any other failure gets the fallback, because its
 * message is meant for programmers.
 */
export function refusalText(failure: unknown, fallback: string): string {
  const refusal = failure as { name?: unknown; message?: unknown } | null;
  const reason = refusal?.name === REFUSAL ? refusal.message : undefined;
  return typeof reason === 'string' && reason.trim() ? reason : fallback;
}

function textOf(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

/**
 * How the connection to the local service stands.
 *
 * - `connecting`: on the way, or waiting to try again
 * - `connected`: the token was accepted and changes are arriving
 * - `no_address`: this window does not know where the service is
 * - `unreachable`: the service could not be reached, for a reason meant for
 *   the person; `restartable` when restarting it would help, as with a
 *   service of another version left running after Teryon was updated
 * - `restarting`: the person asked to restart the service
 */
export type ConnectionState =
  | { readonly kind: 'connecting' }
  | { readonly kind: 'connected' }
  | { readonly kind: 'no_address' }
  | { readonly kind: 'unreachable'; readonly reason: string; readonly restartable: boolean }
  | { readonly kind: 'restarting' };

export const CONNECTING: ConnectionState = { kind: 'connecting' };
export const CONNECTED: ConnectionState = { kind: 'connected' };
export const NO_ADDRESS: ConnectionState = { kind: 'no_address' };
export const RESTARTING: ConnectionState = { kind: 'restarting' };

/** What a window says when the service could not be reached and gave no reason. */
export const NO_REASON = 'Could not reach Teryon. It will keep trying.';

/**
 * Why the service could not be reached, from whatever the attempt failed with.
 *
 * The desktop app fails with `{ reason, restartable }`. Anything else has a
 * message meant for programmers, so the person gets a plain fallback.
 */
export function unreachable(problem: unknown): ConnectionState {
  const given = problem as { reason?: unknown; restartable?: unknown } | null;
  const reason = typeof given?.reason === 'string' && given.reason.trim() ? given.reason : NO_REASON;
  return { kind: 'unreachable', reason, restartable: given?.restartable === true };
}

/** What the empty screen says while the workspace cannot be shown. */
export function connectionText(state: ConnectionState): string {
  switch (state.kind) {
    case 'connecting':
      return 'Connecting to Teryon…';
    case 'no_address':
      return 'This window does not know where Teryon is running. Open Teryon from the desktop app, or from the address `teryon service --print-address` prints.';
    case 'unreachable':
      return state.reason;
    case 'restarting':
      return 'Restarting Teryon…';
    case 'connected':
      return '';
  }
}

/** Whether the screen offers to restart the service. */
export function offersRestart(state: ConnectionState): boolean {
  return state.kind === 'unreachable' && state.restartable;
}
