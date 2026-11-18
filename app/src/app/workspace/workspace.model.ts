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
  readonly created_at: string;
}

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

/**
 * The reason the server gave for refusing a request, or a fallback.
 *
 * The server answers a refused request with `{ "error": "…" }`.
 */
export function refusalText(failure: unknown, fallback: string): string {
  const body = (failure as { error?: unknown } | null)?.error;
  const reason = (body as { error?: unknown } | null)?.error;
  return typeof reason === 'string' && reason.trim() ? reason : fallback;
}

function textOf(value: unknown): string {
  return typeof value === 'string' ? value : '';
}
