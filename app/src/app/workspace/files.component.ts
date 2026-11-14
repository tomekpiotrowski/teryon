import { DOCUMENT } from '@angular/common';
import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, inject, Input, Output } from '@angular/core';
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
  switchMap,
  timer,
} from 'rxjs';
import {
  changeLetter,
  changeTag,
  directoryName,
  effectiveMode,
  type FileChange,
  type FileContent,
  type FileLine,
  languageOf,
  lineCountText,
  type Listing,
  liveText,
  modeAvailable,
  rowIndent,
  sameAnswer,
  selectedEntry,
  shownLines,
  splitPath,
  type TreeRow,
  treeRows,
  type ViewMode,
} from './files.model';
import { WorkspaceApi } from './workspace.api';
import { type Project, refusalText } from './workspace.model';

/** How often the files are read again while the screen is open, in milliseconds. */
const REFRESH_EVERY = 4000;

/** The files of a branch, or why they could not be read. Neither while the first answer is on its way. */
interface ListingState {
  readonly listing: Listing | null;
  readonly problem: string | null;
}

/** The content of the open file, or why it could not be read. */
interface ContentState {
  readonly content: FileContent | null;
  readonly problem: string | null;
}

/** The open file, ready to show. */
interface OpenFile {
  readonly path: string;
  readonly folders: string;
  readonly name: string;
  readonly tag: string;
  readonly mode: ViewMode;
  readonly fileAvailable: boolean;
  readonly diffAvailable: boolean;
  /** Nothing unless the file is text that has arrived. */
  readonly lines: readonly FileLine[] | null;
  /** What to say in place of the lines. */
  readonly notice: string;
  readonly language: string;
  readonly lineCount: string;
}

/** Everything the template needs at one moment. */
interface FilesView {
  readonly projectName: string;
  readonly repository: string;
  readonly live: string;
  readonly branches: readonly string[];
  readonly branch: string | null;
  readonly filter: string;
  readonly rows: readonly TreeRow[];
  readonly selected: string | null;
  /** What to say in place of the tree and the file. */
  readonly notice: string;
  readonly file: OpenFile | null;
}

const WAITING: ListingState = { listing: null, problem: null };
const NO_CONTENT: ContentState = { content: null, problem: null };

/**
 * The file browser: a project's files on the side, one of them in the middle.
 *
 * It exists so that what agents did to the files can be looked at without
 * leaving Teryon. The files are not synchronised like projects and threads,
 * because they live on disk rather than in the database. Instead the two
 * requests are repeated while the screen is open, which keeps everything
 * shown here an observable of the latest answer.
 *
 * What the person picked — branch, file, filter, opened folders, the way of
 * showing — sits in subjects. Handlers only push into those.
 */
@Component({
  selector: 'app-files',
  templateUrl: './files.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class FilesComponent {
  private readonly api = inject(WorkspaceApi);
  private readonly document = inject(DOCUMENT);
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);

  private readonly project$ = new ReplaySubject<Project>(1);
  private readonly liveAgents$ = new BehaviorSubject(0);

  @Input({ required: true }) set project(project: Project) {
    this.project$.next(project);
  }

  /** How many agents are running in the project right now. */
  @Input() set liveAgents(count: number) {
    this.liveAgents$.next(count);
  }

  /** The person asked to go back to the thread. */
  @Output() readonly closed = new EventEmitter<void>();
  /** The person wants to ask an agent about the file with this path. */
  @Output() readonly asked = new EventEmitter<string>();

  readonly rowIndent = rowIndent;
  readonly changeLetter = changeLetter;

  /** The branch asked for. Nothing means the project's directory as it is. */
  private readonly branch$ = new BehaviorSubject<string | null>(null);
  private readonly chosenPath$ = new BehaviorSubject<string | null>(null);
  private readonly filter$ = new BehaviorSubject('');
  /** The folders the person opened or closed themselves. */
  private readonly toggled$ = new BehaviorSubject<ReadonlyMap<string, boolean>>(new Map());
  private readonly mode$ = new BehaviorSubject<ViewMode | null>(null);

  private readonly source$ = combineLatest([
    this.project$.pipe(distinctUntilChanged((a, b) => a.id === b.id)),
    this.branch$,
  ]).pipe(shareReplay({ bufferSize: 1, refCount: true }));

  private readonly listing$: Observable<ListingState> = this.source$.pipe(
    switchMap(([project, branch]) =>
      this.repeated(() =>
        this.api.files(project.id, branch).pipe(
          map((listing): ListingState => ({ listing, problem: null })),
          catchError((failure: unknown) => {
            // A branch can be removed while it is looked at, for example by a merge.
            if (branch !== null && (failure as { status?: number } | null)?.status === 404) this.branch$.next(null);
            return of<ListingState>({ listing: null, problem: refusalText(failure, 'The files could not be read.') });
          }),
        ),
      ).pipe(startWith(WAITING)),
    ),
    distinctUntilChanged<ListingState>(sameAnswer),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  private readonly selectedPath$ = combineLatest([this.listing$, this.chosenPath$]).pipe(
    map(([state, chosen]) => selectedEntry(state.listing?.files ?? [], chosen)?.path ?? null),
    distinctUntilChanged(),
  );

  private readonly content$: Observable<ContentState> = combineLatest([this.source$, this.selectedPath$]).pipe(
    switchMap(([[project, branch], path]): Observable<ContentState> => {
      if (path === null) return of(NO_CONTENT);
      return this.repeated(() =>
        this.api.file(project.id, branch, path).pipe(
          map((file): ContentState => ({ content: file.content, problem: null })),
          catchError((failure: unknown) =>
            of<ContentState>({ content: null, problem: refusalText(failure, 'The file could not be read.') }),
          ),
        ),
      ).pipe(startWith(NO_CONTENT));
    }),
    distinctUntilChanged<ContentState>(sameAnswer),
  );

  readonly view$: Observable<FilesView> = combineLatest([
    this.project$,
    this.liveAgents$,
    this.listing$,
    this.content$,
    this.chosenPath$,
    this.filter$,
    this.toggled$,
    this.mode$,
  ]).pipe(
    map(([project, liveAgents, state, content, chosen, filterText, toggled, mode]) => {
      const files = state.listing?.files ?? [];
      const entry = selectedEntry(files, chosen);
      const rows = treeRows(files, filterText, toggled, entry?.path ?? null);
      return {
        projectName: project.name,
        repository: directoryName(project.directory),
        live: liveText(liveAgents),
        branches: state.listing?.branches ?? [],
        branch: state.listing?.branch ?? null,
        filter: filterText,
        rows,
        selected: entry?.path ?? null,
        notice: listingNotice(state, entry !== null),
        file: entry ? openFile(entry.path, entry.change, state.listing?.base ?? null, content, mode) : null,
      };
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  constructor() {
    // Keeps the open file's row in sight: the tree can be longer than its
    // space, most of all on a phone, and the file may have been picked for
    // the person rather than by them.
    this.view$
      .pipe(
        map((view) => view.selected),
        distinctUntilChanged(),
        takeUntilDestroyed(),
      )
      .subscribe(() =>
        requestAnimationFrame(() =>
          this.host.nativeElement.querySelector('.files-row.selected')?.scrollIntoView({ block: 'nearest' }),
        ),
      );
  }

  chooseBranch(event: Event): void {
    this.branch$.next((event.target as HTMLSelectElement).value || null);
  }

  updateFilter(event: Event): void {
    this.filter$.next((event.target as HTMLInputElement).value);
  }

  /** Opens a file, or opens or closes a folder. */
  choose(row: TreeRow): void {
    if (row.kind === 'file') {
      this.chosenPath$.next(row.path);
      this.mode$.next(null);
    } else {
      this.toggled$.next(new Map(this.toggled$.value).set(row.path, !row.open));
    }
  }

  chooseMode(mode: ViewMode): void {
    this.mode$.next(mode);
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
}

/** What to say when there is no file to show. Nothing when there is one. */
function listingNotice(state: ListingState, hasFile: boolean): string {
  if (state.problem) return state.problem;
  if (!state.listing) return 'Reading the files…';
  if (hasFile) return '';
  return state.listing.files.length ? 'Nothing has changed. Choose a file to read it.' : 'There are no files here yet.';
}

function openFile(
  path: string,
  change: FileChange | null,
  base: string | null,
  state: ContentState,
  chosenMode: ViewMode | null,
): OpenFile {
  const mode = effectiveMode(chosenMode, change);
  const lines = state.content?.kind === 'text' ? state.content.lines : null;
  return {
    path,
    ...splitPath(path),
    tag: changeTag(change, base),
    mode,
    fileAvailable: modeAvailable('file', change),
    diffAvailable: modeAvailable('diff', change),
    lines: lines && shownLines(lines, mode),
    notice: contentNotice(state),
    language: languageOf(path),
    lineCount: lines ? lineCountText(lines) : '',
  };
}

/** What to say in place of a file's lines. Nothing when there are lines to show. */
function contentNotice(state: ContentState): string {
  if (state.problem) return state.problem;
  switch (state.content?.kind) {
    case 'text':
      return state.content.lines.length ? '' : 'This file is empty.';
    case 'binary':
      return 'This file is not text, so it cannot be shown here.';
    case 'too_large':
      return 'This file is too large to show here.';
    default:
      return 'Reading the file…';
  }
}
