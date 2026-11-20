import { AfterViewInit, ChangeDetectionStrategy, Component, ElementRef, EventEmitter, inject, Output } from '@angular/core';
import { BehaviorSubject, catchError, combineLatest, debounceTime, map, type Observable, of, startWith, switchMap, tap } from 'rxjs';
import { stepped } from './commands.model';
import {
  adopted,
  carriedListing,
  directoryBaseName,
  directoryEnter,
  type DirectoryListing,
  type HeldListing,
  pickerView,
  type PickerView,
} from './new-project.model';
import { WorkspaceApi } from './workspace.api';
import { refusalText } from './workspace.model';

/** A project a person asked to add: what to call it and where it is. */
export interface NewProject {
  readonly name: string;
  readonly directory: string;
}

/** How long to wait, in milliseconds, before listing what was typed. */
const ASK_AFTER = 40;

/** A listing that has not arrived yet. */
interface BrowseState {
  readonly requested: string;
  readonly listing: DirectoryListing | null;
  readonly notice: string;
  readonly settled: boolean;
}

const PENDING: BrowseState = { requested: '', listing: null, notice: '', settled: false };

/**
 * The form for a new project: a directory to step through, and a name.
 *
 * The directory can be typed or picked from the list under the field. What
 * is typed narrows that list. The name starts as the directory's own name
 * and stays once the person changes it.
 *
 * It asks the service for the directories on this machine. The menu only
 * shows it, and the page adds the project.
 */
@Component({
  selector: 'app-new-project',
  templateUrl: './new-project.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class NewProjectComponent implements AfterViewInit {
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  private readonly api = inject(WorkspaceApi);

  private readonly path$ = new BehaviorSubject('');
  private readonly name$ = new BehaviorSubject('');
  private readonly marked$ = new BehaviorSubject(0);
  /** Whether the mark follows what was typed. The arrows and the pointer turn it off. */
  private followTyped = true;
  /** Whether the name is one the person typed. */
  private customName = false;
  /** The directory the current name was taken from, so clearing the name follows the next one. */
  private confirmed = '';
  /** The last listing, reused while the typed path stays in that directory. */
  private held: HeldListing | null = null;
  /**
   * Whether the next listing has to be asked for.
   *
   * Stepping in from the list sets the field to the child directory. That text
   * is the same text as typing the child's whole name, which should only narrow
   * the list, so the step itself is what asks for the child's contents.
   */
  private forceAsk = false;

  @Output() readonly added = new EventEmitter<NewProject>();
  @Output() readonly cancelled = new EventEmitter<void>();

  /**
   * The listing for what was typed.
   *
   * `path$` emits again when the text is unchanged. Stepping into the
   * directory whose name is already written still has to ask for its contents.
   */
  private readonly browse$: Observable<BrowseState> = this.path$.pipe(
    map((path) => path.trim()),
    debounceTime(ASK_AFTER),
    switchMap((path) => {
      const carried = this.forceAsk ? null : carriedListing(path, this.held);
      this.forceAsk = false;
      if (carried) {
        this.held = { requested: path, listing: carried };
        return of({ requested: path, listing: carried, notice: '', settled: true });
      }
      return this.api.directories(path).pipe(
        map((listing) => {
          this.held = { requested: path, listing };
          return { requested: path, listing, notice: '', settled: true };
        }),
        catchError((failure: unknown) => {
          this.held = null;
          return of({
            requested: path,
            listing: null,
            notice: refusalText(failure, 'The directories could not be read.'),
            settled: true,
          });
        }),
      );
    }),
    tap((state) => this.adopt(state)),
    startWith(PENDING),
  );

  readonly view$: Observable<PickerView> = combineLatest([this.path$, this.name$, this.marked$, this.browse$]).pipe(
    map(([path, name, marked, browse]) =>
      pickerView({
        path,
        name,
        marked,
        followTyped: this.followTyped,
        requested: browse.requested,
        listing: browse.listing,
        notice: browse.notice,
        settled: browse.settled,
      }),
    ),
  );

  /**
   * Puts the cursor in the directory field, except where that would raise an on-screen keyboard.
   *
   * On a phone the list is there to be tapped.
   */
  ngAfterViewInit(): void {
    if (!matchMedia('(pointer: coarse)').matches) this.focusDirectory();
  }

  typePath(event: Event): void {
    this.followTyped = true;
    this.marked$.next(0);
    this.path$.next((event.target as HTMLInputElement).value);
  }

  typeName(event: Event): void {
    const name = (event.target as HTMLInputElement).value;
    const automatic = directoryBaseName(this.confirmed);
    this.customName = name.trim() !== '' && name !== automatic;
    this.name$.next(name);
  }

  mark(index: number): void {
    this.followTyped = false;
    if (index !== this.marked$.value) this.marked$.next(index);
  }

  /**
   * Moves the mark with the arrows. Enter steps into the marked directory
   * while a name is being typed. When nothing matches, Enter stays, so a
   * path that is not there is not added. Otherwise the form adds the project.
   */
  move(event: KeyboardEvent, view: PickerView): void {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      this.followTyped = false;
      this.marked$.next(stepped(view.marked, event.key === 'ArrowDown' ? 1 : -1, view.rows.length));
      requestAnimationFrame(() => this.host.nativeElement.querySelector('.picker-row.marked')?.scrollIntoView({ block: 'nearest' }));
      return;
    }
    if (event.key !== 'Enter') return;
    const action = directoryEnter(view);
    if (action === 'submit') return;
    event.preventDefault();
    const row = view.rows[view.marked];
    if (action === 'step' && row) this.go(row.path);
  }

  /** Steps into a directory from the list. */
  go(path: string): void {
    this.forceAsk = true;
    this.followTyped = true;
    this.marked$.next(0);
    this.path$.next(path);
    this.focusDirectory();
  }

  add(event: Event): void {
    event.preventDefault();
    this.added.emit({ name: this.name$.value, directory: this.path$.value });
  }

  /** Follows a listing of a directory the field names in full. */
  private adopt(state: BrowseState): void {
    const next = adopted(state.requested, state.listing, this.path$.value, this.name$.value, this.customName);
    if (!next) return;
    this.customName = next.customName;
    this.confirmed = next.directory;
    if (next.path !== null && next.path !== this.path$.value) this.path$.next(next.path);
    if (next.name !== this.name$.value) this.name$.next(next.name);
  }

  private focusDirectory(): void {
    this.host.nativeElement.querySelector<HTMLInputElement>('[aria-label="Project directory"]')?.focus();
  }
}
