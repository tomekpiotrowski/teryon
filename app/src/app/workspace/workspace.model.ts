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

/** A project with its threads, as the side list shows it. */
export interface ProjectGroup {
  readonly project: Project;
  readonly threads: readonly Thread[];
  /** Whether an agent is working in any of the project's threads. */
  readonly running: boolean;
}

/** What the person has picked in the side list. */
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
