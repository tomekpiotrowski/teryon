import { DOCUMENT } from '@angular/common';
import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, inject, Input, Output } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import {
  BehaviorSubject,
  catchError,
  combineLatest,
  distinctUntilChanged,
  exhaustMap,
  filter,
  map,
  type Observable,
  of,
  ReplaySubject,
  shareReplay,
  startWith,
  Subject,
  switchMap,
  takeWhile,
  timer,
  withLatestFrom,
} from 'rxjs';
import { changeLetter, type FileContent, languageOf, liveText, sameAnswer, splitPath } from './files.model';
import { type CodeLine, colouredLines } from './highlight';
import {
  aheadText,
  type ChangeRow,
  changeRows,
  type Changes,
  chosenCommits,
  type Commit,
  commitCountText,
  type DiffMode,
  type DiffRow,
  diffRows,
  type GraphRow,
  graphRows,
  type History,
  looseRows,
  type Maker,
  matchesFilter,
  momentText,
  moveStep,
  neighbour,
  selectedChange,
  shortHash,
  type SplitRow,
  splitRows,
  timeText,
  type Who,
  whoMade,
} from './history.model';
import { WorkspaceApi } from './workspace.api';
import { type Project, refusalText, type Thread, threadLabel } from './workspace.model';

/** How often the history is read again while the screen is open, in milliseconds. */
const REFRESH_EVERY = 4000;

/** How often the times beside the commits are worked out again, in milliseconds. */
const CLOCK_EVERY = 30_000;

/** The commits of a branch, or why they could not be read. Neither while the first answer is on its way. */
interface HistoryState {
  readonly history: History | null;
  readonly problem: string | null;
}

/** A commit with who made it. */
interface Made {
  readonly commit: Commit;
  readonly who: Who;
}

/** Every commit listed, and those of them the filter leaves. */
interface Listed {
  readonly state: HistoryState;
  readonly all: readonly Made[];
  readonly shown: readonly Made[];
}

/** The commit shown and the one it is compared with, by their names. */
interface Picked {
  readonly commit: string | null;
  /** Nothing when the commit is compared with the one it continues. */
  readonly from: string | null;
}

/** What a commit changed, or why that could not be read. */
interface ChangesState extends Picked {
  readonly changes: Changes | null;
  readonly problem: string | null;
}

/** The changed file to show, in the commit it belongs to. */
interface Target extends Picked {
  readonly path: string | null;
}

/** One changed file's lines, or why they could not be read. */
interface ContentState {
  readonly path: string | null;
  readonly content: FileContent | null;
  readonly problem: string | null;
}

/** One changed file laid out both ways. */
interface LaidOut extends ContentState {
  /** Nothing unless the file is text that has arrived. */
  readonly unified: readonly DiffRow<CodeLine>[] | null;
  readonly split: readonly SplitRow<CodeLine>[] | null;
}

/** One row of the list of commits, ready to show. */
interface CommitLine {
  readonly hash: string;
  readonly short: string;
  readonly subject: string;
  readonly who: string;
  /** The thread a commit came from, or the main branch for a commit it has too. */
  readonly tag: string;
  /** Whether the tag names the thread open in the workspace. */
  readonly tagCurrent: boolean;
  readonly time: string;
  readonly merge: boolean;
  readonly graph: GraphRow;
  readonly selected: boolean;
  /** Whether the commit is the other one of two being compared. */
  readonly compared: boolean;
}

/** The changed file open in the middle, ready to show. */
interface OpenChange {
  readonly path: string;
  readonly folders: string;
  readonly name: string;
  readonly mode: DiffMode;
  readonly unified: readonly DiffRow<CodeLine>[] | null;
  readonly split: readonly SplitRow<CodeLine>[] | null;
  /** What to say in place of the lines. */
  readonly notice: string;
}

/** The commit open in the middle, ready to show. */
interface Detail {
  readonly commit: Commit;
  readonly short: string;
  readonly title: string;
  /** Whether two commits are compared, in place of one commit and the one it continues. */
  readonly comparing: boolean;
  /** Who made the commit. Nothing while two commits are compared. */
  readonly name: string;
  /** What follows the name: the thread and the time, or the two commits compared. */
  readonly byline: string;
  readonly thread: Thread | null;
  /** The short name of the commit this one continues. */
  readonly parent: string;
  readonly body: string;
  readonly fileCount: string;
  readonly rows: readonly ChangeRow[];
  readonly selectedPath: string | null;
  /** What to say in place of the changed files. */
  readonly notice: string;
  readonly file: OpenChange | null;
}

/** Everything the template needs at one moment. */
interface HistoryView {
  readonly projectName: string;
  readonly live: string;
  readonly branches: readonly string[];
  readonly branch: string | null;
  /** The branch as the label and the foot name it. */
  readonly branchText: string;
  readonly ahead: string;
  readonly filter: string;
  readonly maker: Maker;
  readonly count: string;
  readonly commits: readonly CommitLine[];
  /** What to say in place of a commit. */
  readonly notice: string;
  readonly detail: Detail | null;
}

const WAITING: HistoryState = { history: null, problem: null };
const NO_CONTENT: ContentState = { path: null, content: null, problem: null };

/**
 * The history screen: a branch's commits on the side, one of them in the middle.
 *
 * It exists so that what agents committed, and in which thread, can be
 * followed without leaving Teryon. Like the files, the history lives in the
 * repository rather than in the database, so the list of commits is asked
 * for again and again while the screen is open. What a commit changed never
 * changes, so that is asked for once.
 *
 * What the person picked — commit, filter, changed file, the way of laying
 * out a change — sits in subjects. Handlers only push into those. The branch
 * looked at is handed in, because the file browser shares it.
 */
@Component({
  selector: 'app-history',
  templateUrl: './history.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class HistoryComponent {
  private readonly api = inject(WorkspaceApi);
  private readonly document = inject(DOCUMENT);
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);

  private readonly project$ = new ReplaySubject<Project>(1);
  private readonly liveAgents$ = new BehaviorSubject(0);
  private readonly branch$ = new BehaviorSubject<string | null>(null);
  private readonly threads$ = new BehaviorSubject<readonly Thread[]>([]);
  private readonly openThread$ = new BehaviorSubject<string | null>(null);
  private readonly ownEmail$ = new BehaviorSubject<string | null>(null);

  @Input({ required: true }) set project(project: Project) {
    this.project$.next(project);
  }

  /** How many agents are running in the project right now. */
  @Input() set liveAgents(count: number) {
    this.liveAgents$.next(count);
  }

  /** The branch to show. Nothing means whatever the project's directory is on. */
  @Input() set branch(branch: string | null) {
    this.branch$.next(branch);
  }

  /** The project's threads, to name the one a commit came from. */
  @Input() set threads(threads: readonly Thread[]) {
    this.threads$.next(threads);
  }

  /** The thread open in the workspace, whose commits stand out. */
  @Input() set openThread(threadId: string | null) {
    this.openThread$.next(threadId);
  }

  /** The address of whoever is signed in, to call their commits `you`. */
  @Input() set ownEmail(email: string | null) {
    this.ownEmail$.next(email);
  }

  /** The person asked to go back to the thread. */
  @Output() readonly closed = new EventEmitter<void>();
  /** The person asked for the files. */
  @Output() readonly filesOpened = new EventEmitter<void>();
  /** The person picked a branch, or the one shown is gone and the directory's own should be. */
  @Output() readonly branchChosen = new EventEmitter<string | null>();
  /** The person wants the files as the commit with this name left them. */
  @Output() readonly browsed = new EventEmitter<string>();
  /** The person wants to see the thread a commit came from. */
  @Output() readonly threadOpened = new EventEmitter<Thread>();
  /** The person wants to ask an agent to revert this commit. */
  @Output() readonly revertAsked = new EventEmitter<Commit>();

  readonly changeLetter = changeLetter;
  readonly makers: readonly { readonly value: Maker; readonly label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'agents', label: 'Agents' },
    { value: 'people', label: 'People' },
  ];

  private readonly filter$ = new BehaviorSubject('');
  private readonly maker$ = new BehaviorSubject<Maker>('all');
  private readonly chosen$ = new BehaviorSubject<string | null>(null);
  /** The second commit of two being compared. */
  private readonly compared$ = new BehaviorSubject<string | null>(null);
  private readonly chosenPath$ = new BehaviorSubject<string | null>(null);
  /** The folders of changed files the person closed. */
  private readonly closedFolders$ = new BehaviorSubject<ReadonlySet<string>>(new Set());
  private readonly mode$ = new BehaviorSubject<DiffMode>('unified');
  /** Steps through the commits asked for with the keyboard. */
  private readonly steps$ = new Subject<number>();

  private readonly projectId$ = this.project$.pipe(
    map((project) => project.id),
    distinctUntilChanged(),
  );

  private readonly history$: Observable<HistoryState> = combineLatest([
    this.projectId$,
    this.branch$.pipe(distinctUntilChanged()),
  ]).pipe(
    switchMap(([projectId, branch]) =>
      this.repeated(() =>
        this.api.history(projectId, branch).pipe(
          map((history): HistoryState => ({ history, problem: null })),
          catchError((failure: unknown) => {
            // A branch can be removed while it is looked at, for example by a merge.
            if (branch !== null && (failure as { status?: number } | null)?.status === 404) this.branchChosen.emit(null);
            return of<HistoryState>({ history: null, problem: refusalText(failure, 'The history could not be read.') });
          }),
        ),
      ).pipe(startWith(WAITING)),
    ),
    distinctUntilChanged<HistoryState>(sameAnswer),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  private readonly listed$: Observable<Listed> = combineLatest([
    this.history$,
    this.threads$,
    this.ownEmail$,
    this.filter$,
    this.maker$,
  ]).pipe(
    map(([state, threads, ownEmail, filterText, maker]) => {
      const all = (state.history?.commits ?? []).map((commit) => ({ commit, who: whoMade(commit, threads, ownEmail) }));
      return { state, all, shown: all.filter(({ commit, who }) => matchesFilter(commit, who, filterText, maker)) };
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  private readonly picked$: Observable<Picked> = combineLatest([this.listed$, this.chosen$, this.compared$]).pipe(
    map(([listed, chosen, compared]): Picked => {
      const picked = chosenCommits(commitsOf(listed.shown), chosen, compared);
      return { commit: picked.commit?.hash ?? null, from: picked.from?.hash ?? null };
    }),
    distinctUntilChanged<Picked>(sameAnswer),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  private readonly changes$: Observable<ChangesState> = combineLatest([this.projectId$, this.picked$]).pipe(
    switchMap(([projectId, picked]): Observable<ChangesState> => {
      const waiting: ChangesState = { ...picked, changes: null, problem: null };
      const commit = picked.commit;
      if (commit === null) return of(waiting);
      return this.untilAnswered(() =>
        this.api.changes(projectId, commit, picked.from).pipe(
          map((changes): ChangesState => ({ ...picked, changes, problem: null })),
          catchError((failure: unknown) =>
            of<ChangesState>({ ...picked, changes: null, problem: refusalText(failure, 'The commit could not be read.') }),
          ),
        ),
      ).pipe(startWith(waiting));
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** Follows what the commit changed, so a file is never asked for in a commit it was not picked in. */
  private readonly target$: Observable<Target> = combineLatest([this.changes$, this.chosenPath$]).pipe(
    map(([state, chosen]): Target => {
      const path = selectedChange(state.changes?.files ?? [], chosen)?.path ?? null;
      return { commit: state.commit, from: state.from, path };
    }),
    distinctUntilChanged<Target>(sameAnswer),
  );

  /**
   * The open file's changes, coloured and laid out once for each answer.
   *
   * Colouring and laying out are the costly part of showing a change, so
   * they sit here and not in `view$`, which is worked out again on every key
   * typed into the filter.
   */
  private readonly content$: Observable<LaidOut> = combineLatest([this.projectId$, this.target$]).pipe(
    switchMap(([projectId, { commit, from, path }]): Observable<ContentState> => {
      if (commit === null || path === null) return of(NO_CONTENT);
      return this.untilAnswered(() =>
        this.api.change(projectId, commit, from, path).pipe(
          map((file): ContentState => ({ path, content: file.content, problem: null })),
          catchError((failure: unknown) =>
            of<ContentState>({ path, content: null, problem: refusalText(failure, 'The file could not be read.') }),
          ),
        ),
      ).pipe(startWith({ ...NO_CONTENT, path }));
    }),
    map(laidOut),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** The present moment in seconds since 1970, for telling today from yesterday. */
  private readonly now$ = timer(0, CLOCK_EVERY).pipe(map(() => Date.now() / 1000));

  readonly view$: Observable<HistoryView> = combineLatest([
    this.project$,
    this.liveAgents$,
    this.listed$,
    this.picked$,
    this.changes$,
    this.content$,
    this.chosenPath$,
    this.closedFolders$,
    this.filter$,
    this.maker$,
    this.mode$,
    this.openThread$,
    this.now$,
  ]).pipe(
    map(([project, liveAgents, listed, picked, changes, content, chosenPath, closed, filterText, maker, mode, openThread, now]) => {
      const history = listed.state.history;
      const shown = commitsOf(listed.shown);
      const whole = listed.shown.length === listed.all.length;
      const graph = whole ? graphRows(shown, history?.more ?? false) : looseRows(shown);
      const made = listed.shown.find((candidate) => candidate.commit.hash === picked.commit) ?? null;
      const from = listed.shown.find((candidate) => candidate.commit.hash === picked.from)?.commit ?? null;
      const answered = changes.commit === picked.commit && changes.from === picked.from ? changes : null;
      return {
        projectName: project.name,
        live: liveText(liveAgents),
        branches: history?.branches ?? [],
        branch: history?.branch ?? null,
        branchText: history ? (history.branch ?? 'not on a branch') : '',
        ahead: aheadText(history?.base ?? null, history?.ahead ?? 0),
        filter: filterText,
        maker,
        count: history ? commitCountText(listed.shown.length, listed.all.length, history.more) : '',
        commits: listed.shown.map(({ commit, who }, row) => ({
          hash: commit.hash,
          short: shortHash(commit.hash),
          subject: commit.subject,
          who: who.name,
          tag: who.thread ? threadLabel(who.thread) : commit.on_base ? (history?.base ?? '') : '',
          tagCurrent: who.thread !== null && who.thread.id === openThread,
          time: timeText(commit.time, now),
          merge: commit.parents.length > 1,
          graph: graph[row],
          selected: commit.hash === picked.commit,
          compared: commit.hash === picked.from,
        })),
        notice: listNotice(listed),
        detail: made ? detail(made, from, answered, content, chosenPath, closed, mode, now) : null,
      };
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  constructor() {
    this.steps$
      .pipe(withLatestFrom(this.listed$, this.picked$), takeUntilDestroyed())
      .subscribe(([step, listed, picked]) => {
        const next = neighbour(commitsOf(listed.shown), picked.commit, step);
        if (next) this.show(next);
      });

    // Keeps the open commit's row in sight: the list can be longer than its
    // space, and the commit may have been reached with the keyboard.
    this.picked$
      .pipe(
        map((picked) => picked.commit),
        distinctUntilChanged(),
        takeUntilDestroyed(),
      )
      .subscribe(() =>
        requestAnimationFrame(() =>
          this.host.nativeElement.querySelector('.history-commit.selected')?.scrollIntoView({ block: 'nearest' }),
        ),
      );
  }

  /** `J` and `K` move down and up the commits, as the hint under the list says. */
  @HostListener('document:keydown', ['$event'])
  handleKey(event: KeyboardEvent): void {
    const target = event.target instanceof HTMLElement ? event.target : null;
    const step = moveStep({
      key: event.key,
      ctrlKey: event.ctrlKey,
      metaKey: event.metaKey,
      altKey: event.altKey,
      element: target?.tagName ?? '',
      editable: target?.isContentEditable ?? false,
    });
    if (!step) return;
    event.preventDefault();
    this.steps$.next(step);
  }

  /**
   * Opens a commit. With Shift held, compares it with the commit already open.
   *
   * `open` is the commit in the middle at the moment of the click.
   */
  choose(hash: string, event: MouseEvent, open: string | null): void {
    if (event.shiftKey && open !== null && open !== hash) {
      this.chosen$.next(open);
      this.compared$.next(hash);
      this.chosenPath$.next(null);
    } else {
      this.show(hash);
    }
  }

  updateFilter(event: Event): void {
    this.filter$.next((event.target as HTMLInputElement).value);
  }

  chooseMaker(maker: Maker): void {
    this.maker$.next(maker);
  }

  chooseFile(path: string): void {
    this.chosenPath$.next(path);
  }

  /** Opens or closes a folder of changed files. */
  toggleFolder(path: string, open: boolean): void {
    const closed = new Set(this.closedFolders$.value);
    if (open) closed.add(path);
    else closed.delete(path);
    this.closedFolders$.next(closed);
  }

  chooseMode(mode: DiffMode): void {
    this.mode$.next(mode);
  }

  private show(hash: string): void {
    this.chosen$.next(hash);
    this.compared$.next(null);
    this.chosenPath$.next(null);
  }

  /**
   * Repeats a request for as long as someone is subscribed.
   *
   * A request still on its way is not started again, and nothing is asked
   * while the page is in the background.
   */
  private repeated<Answer>(request: () => Observable<Answer>): Observable<Answer> {
    return timer(0, REFRESH_EVERY).pipe(
      filter(() => this.document.visibilityState !== 'hidden'),
      exhaustMap(request),
    );
  }

  /**
   * Repeats a request until it is answered without a problem.
   *
   * What a commit changed never changes, so one answer is enough, but a
   * request that failed for a passing reason should not stay failed.
   */
  private untilAnswered<State extends { readonly problem: string | null }>(
    request: () => Observable<State>,
  ): Observable<State> {
    return this.repeated(request).pipe(takeWhile((state) => state.problem !== null, true));
  }
}

function commitsOf(made: readonly Made[]): Commit[] {
  return made.map(({ commit }) => commit);
}

/** What to say when there is no commit to show. Nothing when there is one. */
function listNotice(listed: Listed): string {
  if (listed.state.problem) return listed.state.problem;
  if (!listed.state.history) return 'Reading the history…';
  if (listed.shown.length) return '';
  return listed.all.length ? 'No commit matches.' : 'There are no commits here yet.';
}

/** Colours an answer's lines by the language its file's name tells, and lays them out both ways. */
function laidOut(state: ContentState): LaidOut {
  if (state.path === null || state.content?.kind !== 'text') return { ...state, unified: null, split: null };
  const unified = diffRows(colouredLines(state.content.lines, languageOf(state.path)));
  return { ...state, unified, split: splitRows(unified) };
}

function detail(
  made: Made,
  from: Commit | null,
  changes: ChangesState | null,
  content: LaidOut,
  chosenPath: string | null,
  closed: ReadonlySet<string>,
  mode: DiffMode,
  now: number,
): Detail {
  const { commit, who } = made;
  const files = changes?.changes?.files ?? [];
  const selected = selectedChange(files, chosenPath);
  const short = shortHash(commit.hash);
  return {
    commit,
    short,
    title: from ? `Changes from ${shortHash(from.hash)} to ${short}` : commit.subject,
    comparing: from !== null,
    name: from ? '' : who.name,
    byline: from ? `${from.subject} → ${commit.subject}` : byline(who, commit, now),
    thread: from ? null : who.thread,
    parent: from ? '' : shortHash(commit.parents[0] ?? ''),
    body: from ? '' : (changes?.changes?.body ?? ''),
    fileCount: changes?.changes ? String(files.length) : '',
    rows: changeRows(files, closed),
    selectedPath: selected?.path ?? null,
    notice: changesNotice(changes, from !== null),
    file: selected ? openChange(selected.path, content, mode) : null,
  };
}

/** What follows the name of who made a commit: for an agent its thread, and for everyone the time. */
function byline(who: Who, commit: Commit, now: number): string {
  const parts = [who.kind === 'agent' ? ' agent' : ''];
  if (who.thread) parts.push(`thread “${threadLabel(who.thread)}”`);
  parts.push(momentText(commit.time, now));
  return parts.join(' · ');
}

/** What to say in place of a commit's changed files. Nothing when there are some. */
function changesNotice(state: ChangesState | null, comparing: boolean): string {
  if (state?.problem) return state.problem;
  if (!state?.changes) return 'Reading the commit…';
  if (state.changes.files.length) return '';
  return comparing ? 'Nothing differs between these two commits.' : 'This commit changed no files.';
}

function openChange(path: string, content: LaidOut, mode: DiffMode): OpenChange {
  const arrived = content.path === path ? content : { ...NO_CONTENT, unified: null, split: null };
  return {
    path,
    ...splitPath(path),
    mode,
    unified: arrived.unified,
    split: arrived.split,
    notice: contentNotice(arrived),
  };
}

/** What to say in place of a changed file's lines. Nothing when there are lines to show. */
function contentNotice(state: LaidOut): string {
  if (state.problem) return state.problem;
  switch (state.content?.kind) {
    case 'text':
      return state.unified?.length ? '' : 'The lines of this file did not change.';
    case 'binary':
      return 'This file is not text, so its changes cannot be shown here.';
    case 'too_large':
      return 'This file is too large to show here.';
    default:
      return 'Reading the file…';
  }
}
