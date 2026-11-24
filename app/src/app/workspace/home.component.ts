import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, inject, Input, Output, ViewChild } from '@angular/core';
import { BehaviorSubject, combineLatest, distinctUntilChanged, map, type Observable, shareReplay } from 'rxjs';
import {
  type HomeChoice,
  type HomeRow,
  homeList,
  type HomeList,
  NO_HOME_CHOICE,
  readHomeChoice,
  runningLines,
  type RunningLines,
  homeAgentState,
  type HomeAgentState,
  startProject,
} from './home.model';
import { stepped } from './commands.model';
import {
  type Choice,
  composerAction,
  FIRST_NEW_THREADS,
  insertNewline,
  type ModelLists,
  type NewThreads,
  NO_MODELS,
  NO_WORKFLOWS,
  type Project,
  type ProjectGroup,
  type Starting,
  type Thread,
  type Workflows,
} from './workspace.model';

/** Where the browser remembers the project last chosen on Home. */
const MEMORY_KEY = 'teryon.home';

/** What the person asked Home to start. */
export interface StartRequest {
  readonly projectId: string;
  /** The workflow's name, one that is still set up. */
  readonly workflow: string;
  readonly text: string;
  readonly choice: Choice | null;
}

/** Which small list under the prompt's buttons is open. */
type Chooser = 'project' | null;

/** Everything the template shows, at one moment. */
interface HomeView {
  readonly list: HomeList;
  readonly running: RunningLines;
  readonly text: string;
  readonly projects: readonly Project[];
  readonly project: Project | null;
  readonly installed: readonly string[];
  readonly models: ModelLists;
  readonly remembered: NewThreads;
  /** What the agent chooser shows and a new thread asks for. */
  readonly agent: HomeAgentState;
  readonly chooser: Chooser;
  /** Which row of the list is marked. Nothing while the prompt has the keys. */
  readonly cursor: number | null;
  readonly placeholder: string;
}

/**
 * Home: the prompt to start a thread, and the threads that need the person.
 *
 * Home answers two questions: do I start something, and which thread do I
 * go to next. Acting happens in the thread, so every item on the list is one
 * line and its only action is to open it, see `home.model.ts`.
 *
 * The prompt holds the keys until `Tab` hands them to the list, where `j`
 * and `k` move and `Enter` opens.
 */
@Component({
  selector: 'app-home',
  templateUrl: './home.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class HomeComponent {
  @Output() readonly started = new EventEmitter<StartRequest>();
  @Output() readonly opened = new EventEmitter<Thread>();
  @Output() readonly boardOpened = new EventEmitter<void>();
  @Output() readonly historyOpened = new EventEmitter<void>();
  @Output() readonly menuOpened = new EventEmitter<void>();
  @Output() readonly shortcutsOpened = new EventEmitter<void>();
  /** An agent and model were chosen for new threads. */
  @Output() readonly choiceChosen = new EventEmitter<Choice>();
  /** The person went back to letting the first stage choose new threads' agent. */
  @Output() readonly automaticChosen = new EventEmitter<void>();
  /** The prompt's text changed by typing, or by Home itself when it sends or clears it. */
  @Output() readonly textChanged = new EventEmitter<string>();

  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  private readonly groups$ = new BehaviorSubject<readonly ProjectGroup[]>([]);
  private readonly installed$ = new BehaviorSubject<readonly string[]>([]);
  private readonly workflows$ = new BehaviorSubject<Workflows>(NO_WORKFLOWS);
  private readonly starting$ = new BehaviorSubject<Starting | null>(null);
  private readonly picked$ = new BehaviorSubject<Choice | null>(null);
  private readonly models$ = new BehaviorSubject<ModelLists>(NO_MODELS);
  private readonly rememberedAgents$ = new BehaviorSubject<NewThreads>(FIRST_NEW_THREADS);
  private readonly open$ = new BehaviorSubject<Project | null>(null);
  private readonly sending$ = new BehaviorSubject(false);
  private readonly text$ = new BehaviorSubject('');
  private readonly choice$ = new BehaviorSubject<HomeChoice>(this.remembered());
  private readonly chooser$ = new BehaviorSubject<Chooser>(null);
  private readonly cursor$ = new BehaviorSubject<number | null>(null);

  @ViewChild('prompt') private prompt?: ElementRef<HTMLTextAreaElement>;
  @ViewChild('list') private list?: ElementRef<HTMLElement>;

  /** Every project with its threads. */
  @Input() set groups(groups: readonly ProjectGroup[]) {
    this.groups$.next(groups);
  }

  /** The agents installed on this machine. */
  @Input() set installed(installed: readonly string[]) {
    this.installed$.next(installed);
  }

  @Input() set workflows(workflows: Workflows) {
    this.workflows$.next(workflows);
  }

  /** Who the default workflow's first stage chooses for a new thread, shown in the chooser while nobody picked one. */
  @Input() set starting(starting: Starting | null) {
    this.starting$.next(starting);
  }

  /** The choice picked for new threads, here or in the menu. Nothing lets the first stage choose. */
  @Input() set pickedChoice(choice: Choice | null) {
    this.picked$.next(choice);
  }

  /** The models discovered for each agent, without their default. */
  @Input() set models(models: ModelLists) {
    this.models$.next(models);
  }

  /** The last model and efforts chosen for each agent. */
  @Input() set rememberedAgents(remembered: NewThreads) {
    this.rememberedAgents$.next(remembered);
  }

  /** The project open before Home, which a new thread starts in until the person picks another. */
  @Input() set project(project: Project | null) {
    this.open$.next(project);
  }

  /**
   * What the prompt holds.
   *
   * Owned by the workspace, so that other screens can put text in the prompt,
   * such as a question about a file, and it survives leaving Home.
   */
  @Input() set text(text: string) {
    if (text !== this.text$.value) this.text$.next(text);
  }

  /** Whether a request is on its way, so a second press does not send it twice. */
  @Input() set sending(sending: boolean) {
    this.sending$.next(sending);
  }

  private readonly rest$: Observable<Pick<HomeView, 'list' | 'running' | 'projects' | 'project' | 'installed' | 'models' | 'agent'>> = combineLatest([
    this.groups$,
    this.installed$,
    this.models$,
    this.workflows$,
    this.starting$,
    this.picked$,
    this.open$,
    this.choice$,
  ]).pipe(
    map(([groups, installed, models, workflows, starting, picked, open, choice]) => {
      const projects = groups.map((group) => group.project);
      return {
        list: homeList(groups),
        running: runningLines(groups),
        projects,
        project: startProject(projects, choice, open),
        installed,
        models,
        agent: homeAgentState(installed, models, workflows, picked, starting),
      };
    }),
  );

  readonly view$: Observable<HomeView> = combineLatest([
    this.rest$,
    this.text$,
    this.chooser$,
    this.rememberedAgents$,
    this.cursor$,
  ]).pipe(
    map(([rest, text, chooser, remembered, cursor]) => ({
      ...rest,
      remembered,
      text,
      chooser,
      cursor: cursor === null ? null : Math.min(cursor, Math.max(0, rest.list.rows.length - 1)),
      placeholder: 'Start a thread',
    })),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** Whether anything at all is waiting, for the empty and the busy layouts. */
  readonly empty$: Observable<boolean> = this.view$.pipe(
    map((view) => view.list.total === 0),
    distinctUntilChanged(),
  );

  updateText(event: Event): void {
    this.setText((event.target as HTMLTextAreaElement).value);
    this.chooser$.next(null);
  }

  /** Answers the keys of the prompt: Enter sends, Shift+Enter and Control+Enter start a new line, Tab goes to the list, and Escape clears. */
  handlePromptKey(event: KeyboardEvent, view: HomeView): void {
    if (event.isComposing) return;
    switch (event.key) {
      case 'Enter': {
        const action = composerAction(event);
        if (!action) return;
        event.preventDefault();
        if (action === 'send') return this.send();
        const text = insertNewline(event.target as HTMLTextAreaElement);
        if (text !== this.text$.value) this.setText(text);
        return this.chooser$.next(null);
      }
      case 'Tab':
        if (event.shiftKey) return;
        if (!view.list.rows.length) return;
        event.preventDefault();
        return this.focusList();
      case 'Escape':
        if (this.chooser$.value) {
          event.preventDefault();
          return this.chooser$.next(null);
        }
        if (this.text$.value) {
          event.preventDefault();
          return this.setText('');
        }
        return (event.target as HTMLElement).blur();
    }
  }

  /** Answers the keys of the list: `j` and `k` or the arrows move, Enter opens, Tab and Escape go back to the prompt. */
  handleListKey(event: KeyboardEvent, view: HomeView): void {
    const rows = view.list.rows.length;
    const at = this.cursor$.value ?? 0;
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        event.preventDefault();
        return this.cursor$.next(Math.min(rows - 1, at + 1));
      case 'k':
      case 'ArrowUp':
        event.preventDefault();
        return this.cursor$.next(Math.max(0, at - 1));
      case 'Enter':
        event.preventDefault();
        return this.openRow(view.list.rows[this.cursor$.value ?? 0]);
      case 'Tab':
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        if (event.key === 'Escape') this.chooser$.next(null);
        return this.focusPrompt();
    }
  }

  /**
   * Handles Home keys outside the prompt.
   *
   * Escape dismisses the open chooser, and `j` and `k` move through the list.
   *
   * Left alone while another screen covers Home, and while the person types.
   */
  @HostListener('document:keydown', ['$event'])
  handleBareKey(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) return;
    if (this.host.nativeElement.closest('[inert]')) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (document.querySelector('app-command-menu, ion-alert')) return;
    if (event.key === 'Escape' && !event.isComposing && this.chooser$.value) {
      event.preventDefault();
      this.chooser$.next(null);
      target?.closest('.home-chooser')?.querySelector<HTMLButtonElement>('.home-option')?.focus();
      return;
    }
    if (target && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable)) return;
    if (target?.closest('[data-home-list]')) return;
    if (event.key === 'j' || event.key === 'k') {
      if (!this.list) return;
      event.preventDefault();
      this.cursor$.next(0);
      this.focusList();
    }
  }

  private openRow(row: HomeRow | undefined): void {
    if (row) this.opened.emit(row.thread);
  }

  /** Starts a thread with the prompt's text when a project is selected. */
  send(): void {
    const text = this.text$.value.trim();
    if (!text || this.sending$.value) return;
    const projects = this.groups$.value.map((group) => group.project);
    const project = startProject(projects, this.choice$.value, this.open$.value);
    if (!project) return;
    const workflows = this.workflows$.value;
    const { sendChoice } = homeAgentState(this.installed$.value, this.models$.value, workflows, this.picked$.value, this.starting$.value);
    this.started.emit({ projectId: project.id, workflow: workflows.default, text, choice: sendChoice });
  }

  chooseProject(project: Project): void {
    this.chooser$.next(null);
    this.remember({ ...this.choice$.value, projectId: project.id });
    this.focusPrompt();
  }

  toggleProjectChooser(): void {
    this.chooser$.next(this.chooser$.value === 'project' ? null : 'project');
  }

  @HostListener('document:click', ['$event'])
  dismissChooser(event: MouseEvent): void {
    const chooser = this.chooser$.value;
    if (!chooser || (event.target instanceof Element && event.target.closest('.home-chooser'))) return;
    this.chooser$.next(null);
  }

  markRow(index: number): void {
    this.cursor$.next(index);
  }

  /** The list lost the keys, so no row is marked. */
  leftList(): void {
    this.cursor$.next(null);
  }

  private setText(text: string): void {
    this.text$.next(text);
    this.textChanged.emit(text);
  }

  private focusPrompt(): void {
    this.cursor$.next(null);
    requestAnimationFrame(() => {
      const box = this.prompt?.nativeElement;
      box?.focus();
      box?.setSelectionRange(box.value.length, box.value.length);
    });
  }

  private focusList(): void {
    if (this.cursor$.value === null) this.cursor$.next(0);
    requestAnimationFrame(() => this.list?.nativeElement.focus());
  }

  private remember(choice: HomeChoice): void {
    this.choice$.next(choice);
    try {
      localStorage.setItem(MEMORY_KEY, JSON.stringify(choice));
    } catch {
      // The choice only lasts for this visit.
    }
  }

  private remembered(): HomeChoice {
    try {
      return readHomeChoice(localStorage.getItem(MEMORY_KEY));
    } catch {
      return NO_HOME_CHOICE;
    }
  }
}
