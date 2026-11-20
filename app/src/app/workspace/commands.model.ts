/**
 * Everything a person can do in the workspace, as one list of commands.
 *
 * The menu, the key bindings and the key hints on screen all read this one
 * list, so an action added here is reachable by typing, by its key and by
 * touch without being wired up three times.
 *
 * Free of Angular and Ionic, like `workspace.model.ts`, so it can be tested on its own.
 */

import type { Opener } from '../service/protocol';
import {
  AGENTS,
  type AgentName,
  agentLabel,
  changeBadge,
  isBareLetter,
  type KeyPress,
  type Opens,
  type Project,
  type ProjectGroup,
  reviewersFor,
  type Thread,
  threadLabel,
} from './workspace.model';

/** The sections of the menu, in the order they are listed before anything is typed. */
export type CommandGroup = 'attention' | 'thread' | 'project' | 'change' | 'action' | 'agent';

/** What choosing a command does, as plain data for the screen to carry out. */
export type CommandAction =
  | { readonly kind: 'openThread'; readonly thread: Thread }
  | { readonly kind: 'openProject'; readonly project: Project }
  | { readonly kind: 'showScreen'; readonly screen: 'files' | 'history' }
  | { readonly kind: 'newProject' }
  | { readonly kind: 'removeProject'; readonly project: Project }
  | { readonly kind: 'useAgent'; readonly agent: AgentName }
  | { readonly kind: 'toggleTheme' }
  | { readonly kind: 'stopAgent'; readonly thread: Thread }
  | { readonly kind: 'applyChange'; readonly thread: Thread }
  | { readonly kind: 'catchUp'; readonly thread: Thread }
  | { readonly kind: 'exploreChange'; readonly thread: Thread }
  | { readonly kind: 'reviewChange'; readonly thread: Thread; readonly agent: AgentName }
  | { readonly kind: 'passFindings'; readonly thread: Thread }
  | { readonly kind: 'discardChange'; readonly thread: Thread }
  | { readonly kind: 'sendForReview'; readonly thread: Thread }
  | { readonly kind: 'openReview'; readonly thread: Thread }
  | { readonly kind: 'copyLocation'; readonly path: string }
  | { readonly kind: 'openCopy'; readonly thread: Thread; readonly opener: Opener }
  | { readonly kind: 'draft'; readonly text: string };

export interface Command {
  readonly id: string;
  readonly title: string;
  /** A second, quieter text: the project of a thread, the directory of a project. */
  readonly detail: string;
  readonly group: CommandGroup;
  /** The bare letter that runs the command without opening the menu. */
  readonly key: string | null;
  /** A letter pressed before `key`, for commands that come in a family, such as `w` for a thread's change. */
  readonly leader: string | null;
  /** Whether an agent is working in what the command opens. */
  readonly live: boolean;
  /** Where the change of the thread the command opens stands, such as `ready to apply`. Nothing for other commands. */
  readonly badge: string;
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
  /** What a thread's copy can be opened in, besides the file manager. */
  readonly opens: Opens;
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
  change: 'This thread’s changes',
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
        {
          detail: projectNames.get(candidate.project_id) ?? '',
          live: candidate.status === 'running',
          badge: changeBadge(candidate),
        },
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
    ...AGENTS.filter((agent) => agent !== situation.agent).map((agent) =>
      command(`action:useAgent:${agent}`, `Use ${agentLabel(agent)} for new threads`, 'action', { kind: 'useAgent', agent }),
    ),
    command('action:newProject', 'New project', 'action', { kind: 'newProject' }),
  );
  if (project) {
    actions.push(command('action:removeProject', `Remove project ${project.name}`, 'action', { kind: 'removeProject', project }));
  }
  actions.push(
    command('action:toggleTheme', situation.theme === 'dark' ? 'Light theme' : 'Dark theme', 'action', { kind: 'toggleTheme' }),
  );

  return [...threads, ...projects, ...changeCommands(situation), ...actions, ...agentCommands(situation)];
}

/**
 * Sending the open thread's change for review on GitHub, or opening the review once sent.
 *
 * Only for projects on GitHub. A declined review can be sent again.
 */
function reviewCommands(situation: Situation, thread: Thread): Command[] {
  const change = thread.change;
  if (!change || !situation.project?.on_github) return [];
  const extra = (key: string) => ({ detail: threadLabel(thread), key, leader: CHANGE_LEADER });
  const commands: Command[] = [];
  if (change.review?.state !== 'in_review') {
    commands.push(command('change:sendForReview', 'Send for review on GitHub', 'change', { kind: 'sendForReview', thread }, extra('r')));
  }
  if (change.review) {
    commands.push(command('change:openReview', 'Open review on GitHub', 'change', { kind: 'openReview', thread }, extra('o')));
  }
  return commands;
}

/**
 * Asking another agent to review the change, and passing what it found to the author.
 *
 * Every agent but the thread's own is offered. The usual one, the one that
 * reviewed before or else the first other agent, has the key; asking it
 * again continues its review, while asking another starts afresh.
 *
 * Passing is only offered while the last review, finished, found something
 * and the author has not worked on the change since.
 */
function agentReviewCommands(thread: Thread): Command[] {
  const extra = (key: string) => ({ detail: threadLabel(thread), key, leader: CHANGE_LEADER });
  const verdict = thread.reviewer?.verdict;
  const [usual, ...others] = reviewersFor(thread);
  const again = verdict && thread.reviewer?.agent === usual ? ' again' : '';
  const commands = [
    command('change:askReview', `Ask ${agentLabel(usual)} to review${again}`, 'change', { kind: 'reviewChange', thread, agent: usual }, extra('c')),
    ...others.map((agent) =>
      command(`change:askReview:${agent}`, `Ask ${agentLabel(agent)} to review`, 'change', { kind: 'reviewChange', thread, agent }, { detail: threadLabel(thread) }),
    ),
  ];
  if (verdict && !verdict.answered && !verdict.cut && verdict.findings !== 0) {
    commands.push(
      command('change:passFindings', `Pass findings to ${agentLabel(thread.agent)}`, 'change', { kind: 'passFindings', thread }, extra('p')),
    );
  }
  return commands;
}

/** The leader key for the commands about the open thread's change. */
export const CHANGE_LEADER = 'w';

/**
 * What can be done with the open thread's change: see it, apply it, catch
 * it up, discard it, copy where its copy is, or open it.
 *
 * Applying and catching up are offered whenever the copy is there, and say
 * why when they cannot be done, rather than hiding: a hidden command teaches
 * nothing. Once the copy is gone only discarding is left, which tidies
 * away what remains, and opening the review on GitHub when there is one.
 * The editor and the terminal are only offered when set up, since Teryon
 * cannot guess which a person uses.
 */
function changeCommands(situation: Situation): Command[] {
  const thread = situation.thread;
  const change = thread?.change;
  if (!thread || !change) return [];
  const extra = (key: string) => ({ detail: threadLabel(thread), key, leader: CHANGE_LEADER });
  const discard = command('change:discard', 'Discard changes', 'change', { kind: 'discardChange', thread }, extra('d'));
  if (change.state?.copy_present === false) {
    const review = change.review ? [command('change:openReview', 'Open review on GitHub', 'change', { kind: 'openReview', thread }, extra('o'))] : [];
    return [...review, discard];
  }
  const commands = [
    command('change:explore', 'See the changes', 'change', { kind: 'exploreChange', thread }, extra('v')),
    command('change:apply', 'Apply to the project', 'change', { kind: 'applyChange', thread }, extra('a')),
    command('change:catchUp', 'Catch up with the project', 'change', { kind: 'catchUp', thread }, extra('u')),
    ...agentReviewCommands(thread),
    ...reviewCommands(situation, thread),
    discard,
    command('change:copyLocation', 'Copy location', 'change', { kind: 'copyLocation', path: change.path }, extra('y')),
    command('change:openFolder', 'Open folder', 'change', { kind: 'openCopy', thread, opener: 'folder' }, extra('f')),
  ];
  if (situation.opens.editor) {
    commands.push(command('change:openEditor', 'Open in editor', 'change', { kind: 'openCopy', thread, opener: 'editor' }, extra('e')));
  }
  if (situation.opens.terminal) {
    commands.push(command('change:openTerminal', 'Open terminal', 'change', { kind: 'openCopy', thread, opener: 'terminal' }, extra('t')));
  }
  return commands;
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

/** What a key press does with the keys bound to commands. */
export type KeyOutcome =
  | { readonly kind: 'run'; readonly command: Command }
  /** A leader was pressed: the next key picks the command. */
  | { readonly kind: 'wait'; readonly leader: string }
  /** The key after a leader picks nothing, so the leader is forgotten. */
  | { readonly kind: 'cancel' };

/** Keys that change another key rather than mean something themselves. */
const MODIFIERS = ['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'];

/**
 * What a key press does, given the leader pressed just before it, if any.
 *
 * A bare letter runs the command bound to it alone, or waits for a second
 * letter when it leads a family of commands. After a leader, the letter
 * runs the family's command; anything else, Escape included, cancels. A
 * modifier on its own changes nothing, so Shift can be held for a capital.
 * Nothing means the key is not for the commands at all.
 */
export function keyOutcome(press: KeyPress, commands: readonly Command[], leader: string | null): KeyOutcome | null {
  if (leader !== null) {
    if (MODIFIERS.includes(press.key)) return null;
    const command = commands.find(
      (candidate) => candidate.leader === leader && candidate.key !== null && isBareLetter(press, candidate.key),
    );
    return command ? { kind: 'run', command } : { kind: 'cancel' };
  }
  const single = commands.find(
    (candidate) => candidate.leader === null && candidate.key !== null && isBareLetter(press, candidate.key),
  );
  if (single) return { kind: 'run', command: single };
  const led = commands.find((candidate) => candidate.leader !== null && isBareLetter(press, candidate.leader));
  return led?.leader ? { kind: 'wait', leader: led.leader } : null;
}

/** The keys that run a command, as shown beside it: `C`, or `W Y` for one after a leader. */
export function keyHint(command: Command): string | null {
  if (command.key === null) return null;
  return [command.leader, command.key].filter((key) => key !== null).join(' ').toUpperCase();
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
  extra: {
    readonly detail?: string;
    readonly key?: string;
    readonly leader?: string;
    readonly live?: boolean;
    readonly badge?: string;
  } = {},
): Command {
  return {
    id,
    title,
    group,
    action,
    detail: extra.detail ?? '',
    key: extra.key ?? null,
    leader: extra.leader ?? null,
    live: extra.live ?? false,
    badge: extra.badge ?? '',
  };
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
