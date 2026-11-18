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
  readonly created_at: string;
}

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
  readonly kind: 'user' | 'agent' | 'activity' | 'notice' | 'error' | 'summary';
  readonly text: string;
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
 * What the status line says about a change, after its name, one part each.
 *
 * In the person's words, never git's: files changed rather than commits,
 * "ready to apply" rather than merging cleanly, newer work in the project
 * rather than conflicts with a branch. How far the project has moved on is
 * left out, since it only matters when it overlaps, and that is said.
 */
export function changeStatus(change: Change): string[] {
  const state = change.state;
  if (!state) return [];
  if (!state.copy_present) return ['its copy was removed outside Teryon'];
  if (state.unfinished) return ['the agent left this change unfinished'];
  const files = state.files_changed;
  const parts = [files === 0 ? 'no files changed yet' : files === 1 ? '1 file changed' : `${files} files changed`];
  if (!state.base_present) return [...parts, 'the version of the project it started from is gone'];
  if (files > 0 && state.fit === 'applies') parts.push('ready to apply');
  if (state.fit === 'overlaps') parts.push('overlaps with newer work in the project');
  return parts;
}

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
  switch (entry.kind) {
    case 'user_message':
      return { kind: 'user', text };
    case 'agent_message':
      return { kind: 'agent', text };
    case 'activity':
      return { kind: 'activity', text: [textOf(entry.body['tool']), textOf(entry.body['summary'])].filter(Boolean).join('  ') };
    case 'notice':
      return { kind: 'notice', text };
    case 'error':
      return { kind: 'error', text };
    case 'turn_summary':
      return { kind: 'summary', text: summaryText(entry.body) };
  }
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
 */
export type ConnectionState = 'connecting' | 'connected' | 'no_address';

/** What the empty screen says while the workspace cannot be shown. */
export function connectionText(state: ConnectionState): string {
  switch (state) {
    case 'connecting':
      return 'Connecting to Teryon…';
    case 'no_address':
      return 'This window does not know where Teryon is running. Open Teryon from the desktop app, or from the address `teryon service --print-address` prints.';
    case 'connected':
      return '';
  }
}
