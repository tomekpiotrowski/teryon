import type { Event } from './protocol';
import { type ModelLists, NO_MODELS, NO_OPENS, NO_WORKFLOWS, type Opens, type Project, type Thread, type ThreadEntry, type Workflows } from '../workspace/workspace.model';

/**
 * The projects and threads as the service last described them.
 *
 * Plain logic with no Angular, so it can be tested on its own.
 */
export interface WorkspaceLists {
  readonly projects: readonly Project[];
  readonly threads: readonly Thread[];
  /** What a thread's copy can be opened in on the service's machine, besides the file manager. */
  readonly opens: Opens;
  /** The models each agent offers besides its default. */
  readonly models: ModelLists;
  /** The workflows threads can follow, and what each stage is called. */
  readonly workflows: Workflows;
}

export const NO_LISTS: WorkspaceLists = { projects: [], threads: [], opens: NO_OPENS, models: NO_MODELS, workflows: NO_WORKFLOWS };

/** The lists after one event. A snapshot replaces everything; changes replace or add one item. */
export function applyEvent(lists: WorkspaceLists, event: Event): WorkspaceLists {
  switch (event.type) {
    case 'snapshot':
      return { projects: event.projects as Project[], threads: event.threads as Thread[], opens: event.opens, models: event.models, workflows: event.workflows };
    case 'models_changed':
      return { ...lists, models: event.models };
    case 'settings_changed':
      return { ...lists, opens: event.opens, workflows: event.workflows };
    case 'project_added':
      return { ...lists, projects: upsert(lists.projects, event.project as Project) };
    case 'project_removed':
      return {
        ...lists,
        projects: lists.projects.filter((project) => project.id !== event.project_id),
        threads: lists.threads.filter((thread) => thread.project_id !== event.project_id),
      };
    case 'thread_changed':
      return { ...lists, threads: upsert(lists.threads, event.thread as Thread) };
    case 'entry_added':
    case 'limits_changed':
    case 'usage_recorded':
      return lists;
  }
}

/**
 * A conversation with entries added, kept in order and without repeats.
 *
 * An entry can arrive twice: once in the list read when following starts,
 * and again as an event, so entries are kept by identifier.
 */
export function withEntries(entries: readonly ThreadEntry[], added: readonly ThreadEntry[]): ThreadEntry[] {
  const byId = new Map(entries.map((entry) => [entry.id, entry]));
  for (const entry of added) byId.set(entry.id, entry);
  return [...byId.values()].sort((first, second) => first.position - second.position);
}

function upsert<Item extends { readonly id: string }>(items: readonly Item[], item: Item): Item[] {
  const index = items.findIndex((candidate) => candidate.id === item.id);
  if (index === -1) return [...items, item];
  const changed = [...items];
  changed[index] = item;
  return changed;
}
