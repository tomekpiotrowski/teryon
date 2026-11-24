import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, inject, Output } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { BehaviorSubject, catchError, combineLatest, distinctUntilChanged, map, type Observable, of, scan, shareReplay, startWith, switchMap, timer } from 'rxjs';
import { ServiceConnection } from '../service/service-connection';
import {
  agentsWithTurns,
  byModel,
  byProject,
  chosenAgent,
  type DayBar,
  dayBars,
  type LimitBlock,
  limitBlocks,
  type UsageReport,
  type UsageRow,
  withLimits,
  withLine,
} from './usage.model';
import { WorkspaceApi } from './workspace.api';
import { type AgentName, agentLabel, refusalText } from './workspace.model';

/** How often the times on the screen ("as of 12 minutes ago", "resets in 2h") are worked out again. */
const CLOCK_EVERY = 60_000;

/** The report as it stands, or why it could not be read. Neither while the first answer is on its way. */
interface ReportState {
  readonly report: UsageReport | null;
  readonly problem: string | null;
}

/** An agent button above the chart. */
interface AgentChoice {
  readonly agent: AgentName;
  readonly label: string;
  /** The number key that chooses it. */
  readonly key: string;
  readonly chosen: boolean;
}

interface UsageView {
  readonly problem: string | null;
  readonly loading: boolean;
  readonly blocks: readonly LimitBlock[];
  readonly choices: readonly AgentChoice[];
  /** The agent the chart and tables show. Nothing when no agent has turns. */
  readonly agent: AgentName | null;
  readonly days: readonly DayBar[];
  readonly projects: readonly UsageRow[];
  readonly models: readonly UsageRow[];
}

/**
 * The usage screen: how much of each agent's subscription is used, and what Teryon's agents used it on.
 *
 * It belongs to no project: limits are an account's, and the project is
 * only one of the ways the last 14 days are split. The report is asked for
 * once when the screen opens; after that each turn's events keep it
 * current, and a clock keeps the times on it current.
 */
@Component({
  selector: 'app-usage',
  templateUrl: './usage.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class UsageComponent {
  /** The mark in this screen's header. Usage covers every project. */
  readonly projectName = 'ALL PROJECTS';
  @Output() readonly closed = new EventEmitter<void>();
  @Output() readonly menuOpened = new EventEmitter<void>();
  @Output() readonly shortcutsOpened = new EventEmitter<void>();

  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  private readonly api = inject(WorkspaceApi);
  private readonly connection = inject(ServiceConnection);

  /** The agent asked for with a button or a number key. */
  private readonly wanted$ = new BehaviorSubject<AgentName | null>(null);

  private readonly state$: Observable<ReportState> = this.api.usage().pipe(
    switchMap((first) =>
      this.connection.events$.pipe(
        scan((report, event) => {
          if (event.type === 'limits_changed') return withLimits(report, event.limits);
          if (event.type === 'usage_recorded') return withLine(report, event.line);
          return report;
        }, first),
        startWith(first),
      ),
    ),
    map((report): ReportState => ({ report, problem: null })),
    catchError((failure: unknown) => of<ReportState>({ report: null, problem: refusalText(failure, 'The usage could not be read.') })),
    startWith<ReportState>({ report: null, problem: null }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  readonly view$: Observable<UsageView> = combineLatest([this.state$, this.wanted$, timer(0, CLOCK_EVERY)]).pipe(
    map(([state, wanted]) => {
      const now = new Date();
      const lines = state.report?.lines ?? [];
      const agents = agentsWithTurns(lines);
      const agent = chosenAgent(agents, wanted);
      return {
        problem: state.problem,
        loading: state.report === null && state.problem === null,
        blocks: limitBlocks(state.report?.limits ?? [], now),
        choices: agents.map((candidate, index) => ({
          agent: candidate,
          label: agentLabel(candidate),
          key: `${index + 1}`,
          chosen: candidate === agent,
        })),
        agent,
        days: dayBars(lines, agent, now),
        projects: byProject(lines, agent),
        models: byModel(lines, agent),
      };
    }),
  );

  /** The agents with turns, in the order their number keys choose them. */
  private readonly agents$ = this.state$.pipe(map((state) => agentsWithTurns(state.report?.lines ?? [])));
  readonly agentCount$ = this.agents$.pipe(map((agents) => agents.length), distinctUntilChanged());
  private agents: readonly AgentName[] = [];

  constructor() {
    this.agents$.pipe(takeUntilDestroyed()).subscribe((agents) => (this.agents = agents));
  }

  /** `1`, `2` and `3` choose the agent the chart shows, as the hints on its buttons say. */
  @HostListener('document:keydown', ['$event'])
  handleKey(event: KeyboardEvent): void {
    if (this.host.nativeElement.closest('[inert]')) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (target && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable)) return;
    const agent = /^[1-9]$/.test(event.key) ? this.agents[Number(event.key) - 1] : undefined;
    if (!agent) return;
    event.preventDefault();
    this.wanted$.next(agent);
  }

  choose(agent: AgentName): void {
    this.wanted$.next(agent);
  }
}
