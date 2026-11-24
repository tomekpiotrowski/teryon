import type { Limits, UsageLine, UsageReport, Window } from '../service/protocol';
import { AGENTS, type AgentName, agentLabel, countText, modelLabel } from './workspace.model';

/**
 * What the usage screen shows: each agent's limits, and what Teryon's agents used in the last 14 days.
 *
 * Agents run on subscriptions, so the screen answers "how much room is
 * left, and when does it come back?" rather than "what did it cost?".
 *
 * Free of Angular, like `workspace.model.ts`, so it can be tested on its
 * own. Everything that depends on the time takes it as an input.
 */

export type { Limits, UsageLine, UsageReport, Window };

/** How many days the chart covers, today included. */
export const DAYS_SHOWN = 14;

/** From how much used a window is shown in the warning color. */
const WARNING_FROM = 80;

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** How close to its end a window is: room left, close to the end, or used up. */
export type Level = 'normal' | 'warning' | 'reached';

/** One limit window, ready to show as a bar. */
export interface LimitRow {
  readonly label: string;
  /** Share used, from 0 to 100, as wide as the bar is filled. */
  readonly used: number;
  readonly usedText: string;
  readonly level: Level;
  readonly resetText: string;
}

/** One agent's limits. */
export interface LimitBlock {
  readonly agent: AgentName;
  /** The agent, with its plan when known: "Codex · Plus". */
  readonly title: string;
  /** How old the reading is: "as of 12 minutes ago". Empty when there is none. */
  readonly asOf: string;
  readonly rows: readonly LimitRow[];
  /** Why there are no rows, or why they may be old. */
  readonly note: string | null;
}

/** One day of the chart. */
export interface DayBar {
  /** The local date, `2026-11-20`. */
  readonly date: string;
  /** "Fri 20". */
  readonly label: string;
  /** "Fri", left out where space is short. */
  readonly weekday: string;
  /** The day of the month, "20". */
  readonly day: number;
  readonly turns: number;
  readonly read: number;
  readonly written: number;
  /** How tall the bar is, from 0 to 100, against the busiest day. */
  readonly height: number;
  /** What hovering the bar says. */
  readonly detail: string;
}

/** One line of the tables by project and by model. */
export interface UsageRow {
  readonly key: string;
  readonly name: string;
  readonly turns: number;
  readonly read: string;
  readonly written: string;
}

/**
 * One block per agent Teryon knows, in the menu's order.
 *
 * Limits are only as fresh as each agent's last turn, so every block says
 * how old its reading is. A failed reading says so, and what to do about it
 * when the person can fix it, such as signing in to Grok again.
 */
export function limitBlocks(limits: readonly Limits[], now: Date): LimitBlock[] {
  return AGENTS.map((agent) => {
    const reading = limits.find((candidate) => candidate.agent === agent) ?? null;
    const rows = (reading?.windows ?? []).map((window) => limitRow(window, now));
    return {
      agent,
      title: reading?.plan ? `${agentLabel(agent)} · ${reading.plan}` : agentLabel(agent),
      asOf: reading && rows.length ? `as of ${agoText(new Date(reading.taken_at), now)}` : '',
      rows,
      note: limitNote(agent, reading, rows.length > 0),
    };
  });
}

function limitNote(agent: AgentName, reading: Limits | null, hasRows: boolean): string | null {
  if (reading?.failed) {
    const failure = hasRows
      ? `Reading ${agentLabel(agent)}’s limits after its last turn failed, so these may be older.`
      : `${agentLabel(agent)}’s limits could not be read.`;
    return reading.advice ? `${failure} ${reading.advice}` : failure;
  }
  if (hasRows) return null;
  if (agent === 'gemini') return 'Gemini does not report its usage limits.';
  return 'No limits seen yet. They appear after the agent’s next turn.';
}

/** A window as a bar. A window whose reset time has passed is shown empty, since it has started over. */
export function limitRow(window: Window, now: Date): LimitRow {
  const resetsAt = window.resets_at ? new Date(window.resets_at) : null;
  const passed = resetsAt !== null && resetsAt.getTime() <= now.getTime();
  const used = passed ? 0 : Math.round(Math.min(100, Math.max(0, window.used)));
  return {
    label: window.label,
    used,
    usedText: `${used}%`,
    level: levelOf(used, window.reached && !passed),
    resetText: resetText(resetsAt, window.reached, now),
  };
}

export function levelOf(used: number, reached: boolean): Level {
  if (reached) return 'reached';
  return used >= WARNING_FROM ? 'warning' : 'normal';
}

/**
 * When a window starts over: "resets in 2h 14m" within a day, "resets Fri 18:30" (local time) later.
 *
 * A used-up window says so first: "reached, resets in 40m".
 */
export function resetText(resetsAt: Date | null, reached: boolean, now: Date): string {
  if (resetsAt === null) return reached ? 'reached' : '';
  const left = resetsAt.getTime() - now.getTime();
  if (left <= 0) return 'reset since the last reading';
  const when = left < DAY ? `in ${leftText(left)}` : `${WEEKDAYS[resetsAt.getDay()]} ${clock(resetsAt)}`;
  return `${reached ? 'reached, ' : ''}resets ${when}`;
}

/** Time left, rounded up to the minute: "14m", "2h 14m", "2h". */
function leftText(milliseconds: number): string {
  const minutes = Math.max(1, Math.ceil(milliseconds / MINUTE));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return minutes % 60 ? `${hours}h ${minutes % 60}m` : `${hours}h`;
}

/** How long ago something happened: "just now", "12 minutes ago", "3 hours ago", "2 days ago". */
export function agoText(then: Date, now: Date): string {
  const ago = Math.max(0, now.getTime() - then.getTime());
  if (ago < MINUTE) return 'just now';
  if (ago < HOUR) return plural(Math.floor(ago / MINUTE), 'minute');
  if (ago < DAY) return plural(Math.floor(ago / HOUR), 'hour');
  return plural(Math.floor(ago / DAY), 'day');
}

function plural(count: number, unit: string): string {
  return `${count} ${unit}${count === 1 ? '' : 's'} ago`;
}

function clock(time: Date): string {
  return `${String(time.getHours()).padStart(2, '0')}:${String(time.getMinutes()).padStart(2, '0')}`;
}

/** The agents with turns among the lines, in the menu's order. */
export function agentsWithTurns(lines: readonly UsageLine[]): AgentName[] {
  return AGENTS.filter((agent) => lines.some((line) => line.agent === agent));
}

/** The agent the chart shows: the one asked for when it has turns, else the first that has. */
export function chosenAgent(agents: readonly AgentName[], wanted: AgentName | null): AgentName | null {
  return wanted !== null && agents.includes(wanted) ? wanted : (agents[0] ?? null);
}

/**
 * One bar per day for one agent, the last [`DAYS_SHOWN`] local days up to today, oldest first.
 *
 * Bars show tokens written. Tokens read are mostly text read again from the
 * cache, and would flatten everything else, so they appear on hover only.
 */
export function dayBars(lines: readonly UsageLine[], agent: AgentName | null, now: Date): DayBar[] {
  const days = Array.from({ length: DAYS_SHOWN }, (_, index) => {
    const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (DAYS_SHOWN - 1 - index));
    return { day, date: dateKey(day), turns: 0, read: 0, written: 0 };
  });
  const byDate = new Map(days.map((day) => [day.date, day]));
  for (const line of lines) {
    if (line.agent !== agent) continue;
    const day = byDate.get(dateKey(new Date(line.at)));
    if (!day) continue;
    day.turns += 1;
    day.read += line.read;
    day.written += line.written;
  }
  const busiest = Math.max(0, ...days.map((day) => day.written));
  return days.map(({ day, date, turns, read, written }) => {
    const weekday = WEEKDAYS[day.getDay()];
    const label = `${weekday} ${day.getDate()}`;
    return {
      date,
      label,
      weekday,
      day: day.getDate(),
      turns,
      read,
      written,
      height: busiest ? (written / busiest) * 100 : 0,
      detail: turns
        ? `${label}: ${turns} ${turns === 1 ? 'turn' : 'turns'}, ${countText(read)} read, ${countText(written)} written`
        : `${label}: nothing`,
    };
  });
}

/** One agent's turns by project, most written first. A project's name is the one on its latest turn. */
export function byProject(lines: readonly UsageLine[], agent: AgentName | null): UsageRow[] {
  return grouped(
    lines.filter((line) => line.agent === agent),
    (line) => line.project_id,
    (line) => line.project_name,
  );
}

/** One agent's turns by model, most written first, named by their labels: "Opus 5.5". Turns whose model is unknown are "default". */
export function byModel(lines: readonly UsageLine[], agent: AgentName | null): UsageRow[] {
  return grouped(
    lines.filter((line) => line.agent === agent),
    (line) => line.model ?? '',
    (line) => (line.model ? modelLabel(line.model) : 'default'),
  );
}

function grouped(lines: readonly UsageLine[], keyOf: (line: UsageLine) => string, nameOf: (line: UsageLine) => string): UsageRow[] {
  const groups = new Map<string, { name: string; turns: number; read: number; written: number }>();
  for (const line of lines) {
    const key = keyOf(line);
    const group = groups.get(key) ?? { name: '', turns: 0, read: 0, written: 0 };
    group.name = nameOf(line);
    group.turns += 1;
    group.read += line.read;
    group.written += line.written;
    groups.set(key, group);
  }
  return [...groups.entries()]
    .sort(([, first], [, second]) => second.written - first.written || first.name.localeCompare(second.name))
    .map(([key, group]) => ({
      key,
      name: group.name,
      turns: group.turns,
      read: countText(group.read),
      written: countText(group.written),
    }));
}

/** The report with an agent's new limits in place of its old ones. */
export function withLimits(report: UsageReport, limits: Limits): UsageReport {
  const others = report.limits.filter((candidate) => candidate.agent !== limits.agent);
  return { ...report, limits: [...others, limits] };
}

/** The report with one more turn. */
export function withLine(report: UsageReport, line: UsageLine): UsageReport {
  return { ...report, lines: [...report.lines, line] };
}

/** A local date as `2026-11-20`. */
function dateKey(time: Date): string {
  return `${time.getFullYear()}-${String(time.getMonth() + 1).padStart(2, '0')}-${String(time.getDate()).padStart(2, '0')}`;
}
