import {
  AfterViewInit,
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  EventEmitter,
  inject,
  Input,
  Output,
} from '@angular/core';
import { BehaviorSubject, combineLatest, map, type Observable } from 'rxjs';
import { type Command, keyHint, matching, type MenuRow, menuRows, stepped } from './commands.model';

/** A project a person asked to add: what to call it and where it is. */
export interface NewProject {
  readonly name: string;
  readonly directory: string;
}

/** Everything the menu's template needs at one moment. */
interface MenuView {
  readonly rows: readonly MenuRow[];
  /** The line Enter would choose. */
  readonly marked: number;
  /** Whether the fields for a new project take the place of the list. */
  readonly adding: boolean;
}

/**
 * The menu: every command in one list, narrowed by typing.
 *
 * It is the one way around the workspace, in place of a side list and a
 * settings corner: a box in the middle of a wide screen, a sheet from the
 * bottom of a narrow one. It only shows the commands it is given and says
 * which was chosen; the page carries them out. The one thing it does
 * itself is ask for a new project's name and directory, because those need
 * typing that a list cannot take.
 *
 * It exists only while open, so each opening starts with nothing typed.
 */
@Component({
  selector: 'app-command-menu',
  templateUrl: './command-menu.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class CommandMenuComponent implements AfterViewInit {
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);

  private readonly commands$ = new BehaviorSubject<readonly Command[]>([]);
  private readonly typed$ = new BehaviorSubject('');
  private readonly marked$ = new BehaviorSubject(0);
  private readonly adding$ = new BehaviorSubject(false);

  @Input({ required: true }) set commands(commands: readonly Command[]) {
    this.commands$.next(commands);
  }

  @Output() readonly chosen = new EventEmitter<Command>();
  @Output() readonly closed = new EventEmitter<void>();
  @Output() readonly projectAdded = new EventEmitter<NewProject>();

  readonly keyHint = keyHint;

  readonly view$: Observable<MenuView> = combineLatest([this.commands$, this.typed$, this.marked$, this.adding$]).pipe(
    map(([commands, typed, marked, adding]) => menuView(commands, typed, marked, adding)),
  );

  /**
   * Puts the cursor in the search field, except where that would raise an on-screen keyboard.
   *
   * On a phone the list is there to be tapped, and a keyboard would cover half of it.
   */
  ngAfterViewInit(): void {
    if (!matchMedia('(pointer: coarse)').matches) this.focusFirstField();
  }

  search(event: Event): void {
    this.typed$.next((event.target as HTMLInputElement).value);
    this.marked$.next(0);
  }

  mark(index: number): void {
    if (index !== this.marked$.value) this.marked$.next(index);
  }

  /**
   * Moves the mark with the arrows and chooses the marked line with Enter.
   *
   * The lines are worked out afresh from what is typed, not taken from what
   * is drawn: the screen is redrawn a moment after a key, and Enter pressed
   * right behind the last letter must choose from the narrowed list.
   */
  handleKey(event: KeyboardEvent): void {
    const view = menuView(this.commands$.value, this.typed$.value, this.marked$.value, this.adding$.value);
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      this.marked$.next(stepped(view.marked, event.key === 'ArrowDown' ? 1 : -1, view.rows.length));
      requestAnimationFrame(() => this.host.nativeElement.querySelector('.marked')?.scrollIntoView({ block: 'nearest' }));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const row = view.rows[view.marked];
      if (row) this.choose(row.command);
    }
  }

  choose(command: Command): void {
    if (command.action.kind !== 'newProject') return this.chosen.emit(command);
    this.adding$.next(true);
    requestAnimationFrame(() => this.focusFirstField());
  }

  addProject(event: Event, name: string, directory: string): void {
    event.preventDefault();
    this.projectAdded.emit({ name, directory });
  }

  private focusFirstField(): void {
    this.host.nativeElement.querySelector('input')?.focus();
  }
}

function menuView(commands: readonly Command[], typed: string, marked: number, adding: boolean): MenuView {
  const rows = menuRows(matching(commands, typed), typed);
  return { rows, marked: Math.min(marked, Math.max(rows.length - 1, 0)), adding };
}
