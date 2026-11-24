import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, Input, OnDestroy, Output, inject } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { BehaviorSubject, combineLatest, distinctUntilChanged, map, type Observable, shareReplay } from 'rxjs';
import { type BoardColumn, type BoardCursor, type SeenFinish, boardColumns, cardAt, cursorOn, movedCursor, rememberedFinishes, settledCursor } from './board.model';
import { WorkspaceStore } from './workspace.store';
import { headerName, isQuiet, type Thread } from './workspace.model';

interface BoardView {
  readonly columns: readonly BoardColumn[];
  readonly cursor: BoardCursor;
  /** Whether the board shows every project's threads rather than one project's. */
  readonly everyProject: boolean;
  /** The project heading, or Teryon for the board across projects. */
  readonly heading: string;
  /** Every thread, so a finish seen on another screen of this visit is still remembered. */
  readonly threads: readonly Thread[];
  /** How many quiet threads the board is showing, and so can put away. */
  readonly quiet: number;
}

/**
 * The board: every thread of a project, or of every project, in a column for its stage.
 *
 * For looking only. Stages move as a result of work, so no key moves a
 * card; Enter opens the thread, where the work is. `h` `j` `k` `l` move
 * between cards, `a` switches between the project and every project, and
 * `x` puts the quiet threads on screen away, after a confirmation.
 */
@Component({
  selector: 'app-board',
  templateUrl: './board.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class BoardComponent implements OnDestroy {
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  @Output() readonly opened = new EventEmitter<Thread>();
  @Output() readonly menuOpened = new EventEmitter<void>();
  @Output() readonly shortcutsOpened = new EventEmitter<void>();
  /** The finishes rendered during this visit, sent when the board closes. */
  @Output() readonly doneSeen = new EventEmitter<readonly SeenFinish[]>();
  /** Asks to put away the quiet threads of one project, or of every project when nothing. */
  @Output() readonly putAway = new EventEmitter<string | null>();
  /** The board switched between one project and every project. */
  @Output() readonly everyProjectChange = new EventEmitter<boolean>();

  private readonly store = inject(WorkspaceStore);
  private readonly projectId$ = new BehaviorSubject<string | null>(null);
  private readonly projectName$ = new BehaviorSubject<string>('');
  private readonly everyProject$ = new BehaviorSubject(false);
  private readonly cursor$ = new BehaviorSubject<BoardCursor | null>(null);
  private readonly startOn$ = new BehaviorSubject<string | null>(null);

  /** The project whose board this is. Nothing shows every project's threads. */
  @Input() set project(project: { readonly id: string; readonly name: string } | null) {
    this.projectId$.next(project?.id ?? null);
    this.projectName$.next(project?.name ?? '');
    if (!project) this.everyProject$.next(true);
  }

  /** Whether the board starts on every project. */
  @Input() set everyProject(everyProject: boolean) {
    this.everyProject$.next(everyProject);
  }

  /** The thread open before the board, where the cursor starts. */
  @Input() set openThreadId(threadId: string | null) {
    this.startOn$.next(threadId);
  }

  readonly view$: Observable<BoardView> = combineLatest([
    this.store.threads$,
    this.store.projects$,
    this.store.workflows$,
    combineLatest([this.projectId$, this.projectName$, this.everyProject$]).pipe(distinctUntilChanged((a, b) => a.every((part, index) => part === b[index]))),
    combineLatest([this.cursor$, this.startOn$]),
  ]).pipe(
    map(([threads, projects, workflows, [projectId, projectName, everyProject], [cursor, startOn]]) => {
      const every = everyProject || projectId === null;
      const columns = boardColumns(threads, projects, workflows, every ? null : projectId);
      const scoped = threads.filter((thread) => every || thread.project_id === projectId);
      return {
        columns,
        cursor: cursor === null ? cursorOn(columns, startOn) : settledCursor(columns, cursor),
        everyProject: every,
        heading: headerName(projectName, !every),
        threads,
        quiet: scoped.filter(isQuiet).length,
      };
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** The board as last shown, for a key press, which has to be answered at once. */
  private current: BoardView | null = null;
  /** Finishes rendered during this visit. Marked seen only when the board closes. */
  private receipts: SeenFinish[] = [];

  constructor() {
    this.view$.pipe(takeUntilDestroyed()).subscribe((view) => {
      this.current = view;
      this.receipts = rememberedFinishes(this.receipts, view.threads, view.columns, new Date().toISOString());
    });
  }

  ngOnDestroy(): void {
    if (this.receipts.length) this.doneSeen.emit(this.receipts);
  }

  @HostListener('document:keydown', ['$event'])
  handleKey(event: KeyboardEvent): void {
    if (this.host.nativeElement.closest('[inert]')) return;
    // A confirmation owns the keys. Escape has to be able to dismiss it.
    if (document.querySelector('ion-alert')) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (event.ctrlKey || event.metaKey || event.altKey || !this.current) return;
    if (target && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable)) return;
    const view = this.current;
    switch (event.key) {
      case 'h':
      case 'j':
      case 'k':
      case 'l':
        event.preventDefault();
        return this.cursor$.next(movedCursor(view.columns, view.cursor, event.key));
      case 'Enter': {
        const thread = cardAt(view.columns, view.cursor);
        if (!thread) return;
        event.preventDefault();
        return this.opened.emit(thread);
      }
      case 'a':
        if (this.projectId$.value === null) return;
        event.preventDefault();
        this.cursor$.next(null);
        this.everyProject$.next(!view.everyProject);
        return this.everyProjectChange.emit(!view.everyProject);
      case 'x':
        if (!view.quiet) return;
        event.preventDefault();
        return this.putAway.emit(view.everyProject ? null : this.projectId$.value);
    }
  }

  /** A card clicked: the cursor goes there, and the thread opens. */
  open(thread: Thread): void {
    this.opened.emit(thread);
  }

  /** Whether the cursor is on a card, for the template. */
  isAt(view: BoardView, column: number, row: number): boolean {
    return view.cursor.column === column && view.cursor.row === row;
  }
}
