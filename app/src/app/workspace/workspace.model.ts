/**
 * The shapes the workspace screen works with, and the plain logic around them.
 *
 * Everything here is free of Angular and Ionic so it can be tested on its own:
 * the screen itself cannot be unit tested until Vitest can load Ionic.
 */

export type AgentName = 'claude' | 'grok';

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
  readonly status: 'idle' | 'running';
  /** Whether an agent stopped working here since the person last looked. */
  readonly unread: boolean;
  /** The names of the slash commands the agent program last said it takes. Empty until it has run here. */
  readonly commands: readonly string[];
  /** The change the thread's agent is working on, in its own copy of the project. Nothing until it starts one. */
  readonly change: Change | null;
  /** How the thread's last change ended. Nothing while one is under way, or before the first. */
  readonly finished: Finish | null;
  /** The second agent, reviewing the change. Nothing until a review is asked for. */
  readonly reviewer: Reviewer | null;
  readonly created_at: string;
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

/** The agent that reviews a thread's change: the one already reviewing, or the other program. */
export function reviewerOf(thread: Thread): AgentName {
  return thread.reviewer?.agent ?? otherAgent(thread.agent);
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

export type EntryKind = 'user_message' | 'agent_message' | 'activity' | 'notice' | 'error' | 'turn_summary';

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
  /** `review` marks where a review starts. */
  readonly kind: 'user' | 'agent' | 'activity' | 'notice' | 'review' | 'error' | 'summary';
  readonly text: string;
  /** Whether the reviewer wrote it, or the person wrote it to the reviewer. */
  readonly reviewer: boolean;
}

export const NOTHING_SELECTED: Selection = { projectId: null, threadId: null };

/** What fills the screen: a project's threads, its files or its history. */
export type Screen = 'thread' | 'files' | 'history';

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
  /** Nothing means whatever the project's directory is on. */
  readonly branch: string | null;
  /** A commit shown in place of a branch, in the file browser only. */
  readonly commit: string | null;
}

export const NO_PLACE: Place = { projectId: null, branch: null, commit: null };

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
  return place.projectId === projectId ? place : { projectId, branch: null, commit: null };
}

/** What the bar at the top names: the selected project, and the open thread after it. */
export function barTitle(project: Project | null, thread: Thread | null): string {
  if (!project) return '';
  return thread ? `${project.name} · ${threadLabel(thread)}` : project.name;
}

/** How many agents are working, across every project. For example `2 running`; nothing when none is. */
export function runningText(groups: readonly ProjectGroup[]): string {
  const count = groups.flatMap((group) => group.threads).filter((thread) => thread.status === 'running').length;
  return count ? `${count} running` : '';
}

/** The threads an agent stopped working in since the person last looked, newest first. */
export function waitingThreads(groups: readonly ProjectGroup[]): Thread[] {
  return groups
    .flatMap((group) => group.threads)
    .filter((thread) => thread.unread && thread.status === 'idle')
    .sort((a, b) => b.created_at.localeCompare(a.created_at));
}

/**
 * What the bar says about threads waiting to be looked at.
 *
 * Up to three are named, for example `waiting: Fix the parser, New thread`,
 * so that a glance is enough to know which. More are only counted.
 */
export function waitingText(groups: readonly ProjectGroup[]): string {
  const waiting = waitingThreads(groups);
  if (!waiting.length) return '';
  return waiting.length > 3 ? `${waiting.length} waiting` : `waiting: ${waiting.map(threadLabel).join(', ')}`;
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

export function threadLabel(thread: Thread): string {
  return thread.title || 'New thread';
}

export function agentLabel(agent: AgentName): string {
  return agent === 'claude' ? 'Claude' : 'Grok';
}

export function otherAgent(agent: AgentName): AgentName {
  return agent === 'claude' ? 'grok' : 'claude';
}

/** Turns a stored entry into the line the thread shows. */
export function entryLine(entry: ThreadEntry): EntryLine {
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
    case 'turn_summary':
      return { kind: 'summary', text: summaryText(entry.body), reviewer };
  }
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

/** For example `Done in 1m 12s · $0.49`. Parts the agent did not report are left out. */
export function summaryText(body: Readonly<Record<string, unknown>>): string {
  const duration = body['duration_ms'];
  const cost = body['cost_usd'];
  const parts = [typeof duration === 'number' ? `Done in ${durationText(duration)}` : 'Done'];
  if (typeof cost === 'number') parts.push(`$${cost.toFixed(2)}`);
  return parts.join(' · ');
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
