/**
 * Everything a person can do in the workspace, as one list of commands.
 *
 * The menu, the key bindings and the key hints on screen all read this one
 * list, so an action added here is reachable by typing, by its key and by
 * touch without being wired up three times.
 *
 * Free of Angular and Ionic, like `workspace.model.ts`, so it can be tested on its own.
 */

import {
  type AgentName,
  agentLabel,
  isBareLetter,
  type KeyPress,
  otherAgent,
  type Project,
  type ProjectGroup,
  type Thread,
  threadLabel,
} from './workspace.model';

/** The sections of the menu, in the order they are listed before anything is typed. */
export type CommandGroup = 'attention' | 'thread' | 'project' | 'action' | 'agent';

/** What choosing a command does, as plain data for the screen to carry out. */
export type CommandAction =
  | { readonly kind: 'openThread'; readonly thread: Thread }
  | { readonly kind: 'openProject'; readonly project: Project }
  | { readonly kind: 'showScreen'; readonly screen: 'files' | 'history' }
  | { readonly kind: 'newProject' }
  | { readonly kind: 'removeProject'; readonly project: Project }
  | { readonly kind: 'toggleAgent' }
  | { readonly kind: 'toggleTheme' }
  | { readonly kind: 'openSettings' }
  | { readonly kind: 'signOut' }
  | { readonly kind: 'stopAgent'; readonly thread: Thread }
  | { readonly kind: 'draft'; readonly text: string };

export interface Command {
  readonly id: string;
  readonly title: string;
  /** A second, quieter text: the project of a thread, the directory of a project. */
  readonly detail: string;
  readonly group: CommandGroup;
  /** The bare letter that runs the command without opening the menu. */
  readonly key: string | null;
  /** Whether an agent is working in what the command opens. */
  readonly live: boolean;
  readonly action: CommandAction;
}

/** What the list of commands depends on. */
export interface Situation {
  readonly groups: readonly ProjectGroup[];
  readonly project: Project | null;
  readonly thread: Thread | null;
  /** The agent a new thread would be started with. */
  readonly agent: AgentName;
  readonly theme: 'light' | 'dark';
}

/** One line of the menu: a command, under a heading when it starts a section. */
export interface MenuRow {
  readonly command: Command;
  readonly heading: string | null;
}

const HEADINGS: Readonly<Record<CommandGroup, string>> = {
  attention: 'Waiting for you',
  thread: 'Threads',
  project: 'Projects',
  action: 'Actions',
  agent: 'Agent commands',
};

/**
 * The commands available right now: the threads waiting to be looked at,
 * the other threads newest first, every project, the actions, then the
 * agent's own slash commands.
 *
 * An action that would do nothing is left out rather than shown disabled:
 * the files and the history need a project, stopping needs a running agent.
 */
export function commandList(situation: Situation): Command[] {
  const { groups, project, thread } = situation;
  const projectNames = new Map(groups.map((group) => [group.project.id, group.project.name]));

  const threads = groups
    .flatMap((group) => group.threads)
    .sort((a, b) => b.created_at.localeCompare(a.created_at))
    .map((candidate) =>
      command(
        `thread:${candidate.id}`,
        threadLabel(candidate),
        candidate.unread && candidate.status === 'idle' ? 'attention' : 'thread',
        { kind: 'openThread', thread: candidate },
        { detail: projectNames.get(candidate.project_id) ?? '', live: candidate.status === 'running' },
      ),
    )
    // A stable sort: the waiting threads move to the top and both parts stay newest first.
    .sort((a, b) => Number(b.group === 'attention') - Number(a.group === 'attention'));

  const projects = groups.map((group) =>
    command(`project:${group.project.id}`, group.project.name, 'project', { kind: 'openProject', project: group.project }, {
      detail: group.project.directory,
      live: group.running,
    }),
  );

  const actions: Command[] = [];
  if (project) {
    actions.push(
      command('action:newThread', 'New thread', 'action', { kind: 'openProject', project }, { detail: project.name, key: 'n' }),
      command('action:files', 'Files', 'action', { kind: 'showScreen', screen: 'files' }, { detail: project.name, key: 'c' }),
      command('action:history', 'History', 'action', { kind: 'showScreen', screen: 'history' }, { detail: project.name, key: 'h' }),
    );
  }
  if (thread?.status === 'running') {
    actions.push(command('action:stopAgent', 'Stop the agent', 'action', { kind: 'stopAgent', thread }, { detail: threadLabel(thread) }));
  }
  actions.push(
    command('action:toggleAgent', `Use ${agentLabel(otherAgent(situation.agent))} for new threads`, 'action', { kind: 'toggleAgent' }),
    command('action:newProject', 'New project', 'action', { kind: 'newProject' }),
  );
  if (project) {
    actions.push(command('action:removeProject', `Remove project ${project.name}`, 'action', { kind: 'removeProject', project }));
  }
  actions.push(
    command('action:toggleTheme', situation.theme === 'dark' ? 'Light theme' : 'Dark theme', 'action', { kind: 'toggleTheme' }),
    command('action:openSettings', 'Settings', 'action', { kind: 'openSettings' }),
    command('action:signOut', 'Sign out', 'action', { kind: 'signOut' }),
  );

  return [...threads, ...projects, ...actions, ...agentCommands(situation)];
}

/**
 * The agent's own slash commands, each putting itself in the message box.
 *
 * A command is not sent at once, because many take something after their
 * name. The list is the one the agent program gave for the open thread.
 * A thread that is yet to start has none, so it borrows the list of the
 * newest thread with the same agent: from the same project if there is
 * one, because a project can bring commands of its own, else from any.
 */
function agentCommands(situation: Situation): Command[] {
  const agent = situation.thread?.agent ?? situation.agent;
  return offeredNames(situation, agent).map((name) =>
    command(`agent:${name}`, `/${name}`, 'agent', { kind: 'draft', text: `/${name} ` }, { detail: `${agentLabel(agent)} command` }),
  );
}

function offeredNames(situation: Situation, agent: AgentName): readonly string[] {
  if (situation.thread) return situation.thread.commands;
  const candidates = situation.groups
    .flatMap((group) => group.threads)
    .filter((thread) => thread.agent === agent && thread.commands.length > 0)
    .sort((a, b) => b.created_at.localeCompare(a.created_at));
  const nearest = candidates.find((thread) => thread.project_id === situation.project?.id) ?? candidates[0];
  return nearest?.commands ?? [];
}

/**
 * The commands that fit what was typed, best fit first.
 *
 * Nothing typed keeps every command in its listed order. Otherwise a title
 * that starts with what was typed comes first, then one with a word that
 * does, then one that holds it somewhere, then one that holds its letters
 * in order but spread out. After all titles come the details, ranked the
 * same way, except that spread out letters do not count there: a detail is
 * often a long directory, which holds almost any few letters in order.
 * Equal fits keep their listed order.
 */
export function matching(commands: readonly Command[], typed: string): Command[] {
  const query = typed.trim().toLowerCase();
  if (!query) return [...commands];
  return commands
    .map((candidate, index) => ({ candidate, index, fit: fitOf(candidate, query) }))
    .filter((scored) => scored.fit !== null)
    .sort((a, b) => (a.fit as number) - (b.fit as number) || a.index - b.index)
    .map((scored) => scored.candidate);
}

/** The lines of the menu. Sections are only named while nothing is typed, because a search mixes them. */
export function menuRows(commands: readonly Command[], typed: string): MenuRow[] {
  const sectioned = !typed.trim();
  return commands.map((candidate, index) => ({
    command: candidate,
    heading: sectioned && candidate.group !== commands[index - 1]?.group ? HEADINGS[candidate.group] : null,
  }));
}

/**
 * Whether a key press asks for the menu.
 *
 * `/` asks for it wherever it would not be typed as text, and also in the
 * message box while that is empty, the way the agents' own programs take a
 * leading slash. Anywhere else in a text it stays a character, so a path
 * can be typed. Ctrl+K (Command+K on a Mac) asks for it from anywhere.
 */
export function menuAsked(press: KeyPress, draft: string): boolean {
  if (press.altKey) return false;
  if (press.key.toLowerCase() === 'k') return press.ctrlKey !== press.metaKey;
  if (press.key !== '/' || press.ctrlKey || press.metaKey) return false;
  if (isBareLetter(press, '/')) return true;
  return press.element.toUpperCase() === 'TEXTAREA' && !draft.trim();
}

/** The command a bare letter is bound to. Nothing for any other key press. */
export function boundCommand(press: KeyPress, commands: readonly Command[]): Command | null {
  return commands.find((candidate) => candidate.key !== null && isBareLetter(press, candidate.key)) ?? null;
}

/** The line marked after moving up or down a list of the given length, going round at either end. */
export function stepped(index: number, step: number, length: number): number {
  if (length <= 0) return 0;
  return (((index + step) % length) + length) % length;
}

function command(
  id: string,
  title: string,
  group: CommandGroup,
  action: CommandAction,
  extra: { readonly detail?: string; readonly key?: string; readonly live?: boolean } = {},
): Command {
  return { id, title, group, action, detail: extra.detail ?? '', key: extra.key ?? null, live: extra.live ?? false };
}

/** How well a command fits a query: lower is better, nothing when it does not fit at all. */
function fitOf(candidate: Command, query: string): number | null {
  const title = textFit(candidate.title.toLowerCase(), query);
  if (title !== null) return title;
  const detail = textFit(candidate.detail.toLowerCase(), query);
  return detail === null || detail === SPREAD_OUT ? null : detail + SPREAD_OUT + 1;
}

const SPREAD_OUT = 3;

function textFit(text: string, query: string): number | null {
  if (text.startsWith(query)) return 0;
  if (text.split(/[\s/._-]+/).some((word) => word.startsWith(query))) return 1;
  if (text.includes(query)) return 2;
  return holdsInOrder(text, query) ? SPREAD_OUT : null;
}

function holdsInOrder(text: string, query: string): boolean {
  let from = 0;
  for (const letter of query) {
    const found = text.indexOf(letter, from);
    if (found < 0) return false;
    from = found + 1;
  }
  return true;
}
