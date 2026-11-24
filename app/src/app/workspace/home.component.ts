import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, inject, Input, Output, ViewChild } from '@angular/core';
import { BehaviorSubject, combineLatest, distinctUntilChanged, map, type Observable, shareReplay } from 'rxjs';
import {
  agentOptions,
  continueMatches,
  type ContinueRow,
  type HomeChoice,
  type HomeRow,
  homeList,
  type HomeList,
  NO_HOME_CHOICE,
  promptOf,
  readHomeChoice,
  runningLines,
  type RunningLines,
  startAgent,
  startProject,
  startWorkflow,
} from './home.model';
import { stepped } from './commands.model';
import { agentLabel, type AgentName, type Project, type ProjectGroup, type Starting, type Thread, type Workflows, NO_WORKFLOWS } from './workspace.model';

/** Where the browser remembers the project and workflow last chosen on Home. */
const MEMORY_KEY = 'teryon.home';

/** What the person asked Home to start. */
export interface StartRequest {
  readonly projectId: string;
  /** The workflow's name, one that is still set up. */
  readonly workflow: string;
  readonly text: string;
}

/** What the person asked Home to say to an existing thread. */
export interface ContinueRequest {
  readonly thread: Thread;
  readonly text: string;
}

/** Which small list under the prompt's buttons is open. */
type Chooser = 'project' | 'agent' | 'workflow' | null;

/** Everything the template shows, at one moment. */
interface HomeView {
  readonly list: HomeList;
  readonly running: RunningLines;
  readonly text: string;
  readonly searching: boolean;
  readonly matches: readonly ContinueRow[];
  /** Which match `Enter` picks. */
  readonly match: number;
  readonly continuing: Thread | null;
  readonly projects: readonly Project[];
  readonly project: Project | null;
  readonly agents: readonly AgentName[];
  /** The agent displayed in the chooser. */
  readonly agent: AgentName | null;
  readonly workflows: readonly string[];
  readonly workflow: string;
  readonly chooser: Chooser;
  /** Which row of the list is marked. Nothing while the prompt has the keys. */
  readonly cursor: number | null;
  readonly placeholder: string;
}

/**
 * Home: the prompt to start or continue a thread, and the threads that need the person.
 *
 * Home answers two questions: do I start something, and which thread do I
 * go to next. Acting happens in the thread, so every item on the list is one
 * line and its only action is to open it, see `home.model.ts`.
 *
 * The prompt holds the keys until `Tab` hands them to the list, where `j`
 * and `k` move and `Enter` opens. Starting with `@` it searches threads to
 * continue instead; choosing one makes it a chip the prompt's text is sent to.
 */
@Component({
  selector: 'app-home',
  templateUrl: './home.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class HomeComponent {
  @Output() readonly started = new EventEmitter<StartRequest>();
  @Output() readonly continued = new EventEmitter<ContinueRequest>();
  @Output() readonly opened = new EventEmitter<Thread>();
  @Output() readonly boardOpened = new EventEmitter<void>();
  @Output() readonly historyOpened = new EventEmitter<void>();
  @Output() readonly menuOpened = new EventEmitter<void>();
  @Output() readonly shortcutsOpened = new EventEmitter<void>();
  /** An agent was chosen for new threads. */
  @Output() readonly agentChosen = new EventEmitter<AgentName>();
  /** The prompt's text changed by typing, or by Home itself when it sends or clears it. */
  @Output() readonly textChanged = new EventEmitter<string>();

  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  private readonly groups$ = new BehaviorSubject<readonly ProjectGroup[]>([]);
  private readonly installed$ = new BehaviorSubject<readonly string[]>([]);
  private readonly workflows$ = new BehaviorSubject<Workflows>(NO_WORKFLOWS);
  private readonly starting$ = new BehaviorSubject<Starting | null>(null);
  private readonly picked$ = new BehaviorSubject<AgentName | null>(null);
  private readonly open$ = new BehaviorSubject<Project | null>(null);
  private readonly sending$ = new BehaviorSubject(false);
  private readonly text$ = new BehaviorSubject('');
  private readonly chip$ = new BehaviorSubject<Thread | null>(null);
  private readonly match$ = new BehaviorSubject(0);
  private readonly choice$ = new BehaviorSubject<HomeChoice>(this.remembered());
  private readonly chooser$ = new BehaviorSubject<Chooser>(null);
  private readonly cursor$ = new BehaviorSubject<number | null>(null);

  @ViewChild('prompt') private prompt?: ElementRef<HTMLInputElement>;
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

  /** The agent the person picked for new threads, here or in the menu. Nothing lets the workflow's first stage choose. */
  @Input() set pickedAgent(agent: AgentName | null) {
    this.picked$.next(agent);
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

  private readonly rest$: Observable<Pick<HomeView, 'list' | 'running' | 'projects' | 'project' | 'agents' | 'agent' | 'workflows' | 'workflow'>> = combineLatest([
    this.groups$,
    this.installed$,
    this.workflows$,
    this.starting$,
    this.picked$,
    this.open$,
    this.choice$,
  ]).pipe(
    map(([groups, installed, workflows, starting, picked, open, choice]) => {
      const { project, workflow } = this.resolve(groups, workflows, open, choice);
      const agents = agentOptions(installed);
      return {
        list: homeList(groups),
        running: runningLines(groups),
        projects: groups.map((group) => group.project),
        project,
        agents,
        agent: startAgent(agents, picked, starting, workflow, workflows),
        workflows: workflows.workflows.map((candidate) => candidate.name),
        workflow,
      };
    }),
  );

  readonly view$: Observable<HomeView> = combineLatest([
    this.rest$,
    this.groups$,
    this.text$,
    this.chip$,
    this.match$,
    this.chooser$,
    this.cursor$,
  ]).pipe(
    map(([rest, groups, text, chip, match, chooser, cursor]) => {
      const prompt = promptOf(text, chip);
      const matches = prompt.kind === 'searching' ? continueMatches(groups, prompt.query) : [];
      return {
        ...rest,
        text,
        searching: prompt.kind === 'searching',
        matches,
        match: Math.min(match, Math.max(0, matches.length - 1)),
        continuing: chip,
        chooser,
        cursor: cursor === null ? null : Math.min(cursor, Math.max(0, rest.list.rows.length - 1)),
        placeholder: chip ? 'Message to this thread' : 'Start a thread — or type @ to continue one',
      };
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** Whether anything at all is waiting, for the empty and the busy layouts. */
  readonly empty$: Observable<boolean> = this.view$.pipe(
    map((view) => view.list.total === 0),
    distinctUntilChanged(),
  );

  readonly agentLabel = agentLabel;

  updateText(event: Event): void {
    this.setText((event.target as HTMLInputElement).value);
    this.match$.next(0);
    this.chooser$.next(null);
  }

  /** Answers the keys of the prompt: Enter sends or picks, Tab goes to the list, Escape and Backspace step back. */
  handlePromptKey(event: KeyboardEvent, view: HomeView): void {
    if (event.isComposing) return;
    // Keys can arrive faster than the screen is drawn, so what they act on is read from the latest state, not from `view`.
    const searching = this.text$.value.startsWith('@') && !this.chip$.value;
    const matches = searching ? continueMatches(this.groups$.value, this.text$.value.slice(1)) : [];
    const match = Math.min(this.match$.value, Math.max(0, matches.length - 1));
    switch (event.key) {
      case 'Enter':
        event.preventDefault();
        if (searching) return this.chooseMatch(matches[match]);
        return this.send();
      case 'ArrowDown':
        if (!searching) return;
        event.preventDefault();
        return this.match$.next(stepped(match, 1, matches.length));
      case 'ArrowUp':
        if (!searching) return;
        event.preventDefault();
        return this.match$.next(stepped(match, -1, matches.length));
      case 'Tab':
        if (event.shiftKey || searching) return;
        if (!view.list.rows.length) return;
        event.preventDefault();
        return this.focusList();
      case 'Escape':
        if (this.chooser$.value) {
          event.preventDefault();
          return this.chooser$.next(null);
        }
        if (searching || this.chip$.value || this.text$.value) {
          event.preventDefault();
          this.chip$.next(null);
          return this.setText('');
        }
        return (event.target as HTMLElement).blur();
      case 'Backspace':
        if (this.chip$.value && !this.text$.value) {
          event.preventDefault();
          this.chip$.next(null);
        }
        return;
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
   * Escape dismisses the open chooser, `j` and `k` move through the list, and `@` starts a search.
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
      target?.closest('.home-popover')?.closest('.home-chooser')?.querySelector<HTMLButtonElement>('.home-option')?.focus();
      return;
    }
    if (target && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable)) return;
    if (target?.closest('[data-home-list]')) return;
    if (event.key === '@') {
      event.preventDefault();
      this.setText('@');
      this.match$.next(0);
      return this.focusPrompt();
    }
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

  chooseMatch(row: ContinueRow | undefined): void {
    if (!row) return;
    this.chip$.next(row.thread);
    this.setText('');
    this.focusPrompt();
  }

  dropChip(): void {
    this.chip$.next(null);
    this.focusPrompt();
  }

  /**
   * Asks for the prompt to be sent: to the chosen thread, or as the first message of a new one.
   *
   * Does nothing with nothing typed. The text and the chosen thread stay as
   * they are: the request can be refused, and then they are still there to
   * send again. Once it succeeds the workspace opens the thread, which
   * replaces Home, and clears the text.
   */
  send(): void {
    const text = this.text$.value.trim();
    if (!text || this.sending$.value) return;
    const chip = this.chip$.value;
    if (chip) return this.continued.emit({ thread: chip, text });
    const { project, workflow } = this.resolved();
    if (project) this.started.emit({ projectId: project.id, workflow, text });
  }

  chooseAgent(agent: AgentName): void {
    this.chooser$.next(null);
    this.agentChosen.emit(agent);
    this.focusPrompt();
  }

  chooseProject(project: Project): void {
    this.chooser$.next(null);
    this.remember({ ...this.choice$.value, projectId: project.id });
    this.focusPrompt();
  }

  chooseWorkflow(name: string): void {
    this.chooser$.next(null);
    this.remember({ ...this.choice$.value, workflow: name });
    this.focusPrompt();
  }

  toggleChooser(which: Exclude<Chooser, null>): void {
    this.chooser$.next(this.chooser$.value === which ? null : which);
  }

  @HostListener('document:click', ['$event'])
  dismissChooser(event: MouseEvent): void {
    if (this.chooser$.value && !(event.target instanceof Element && event.target.closest('.home-chooser'))) {
      this.chooser$.next(null);
    }
  }

  markRow(index: number): void {
    this.cursor$.next(index);
  }

  /** The list lost the keys, so no row is marked. */
  leftList(): void {
    this.cursor$.next(null);
  }

  /** The project and workflow a new thread starts with now, read from the latest state rather than what was last drawn. */
  private resolved(): { project: Project | null; workflow: string } {
    return this.resolve(this.groups$.value, this.workflows$.value, this.open$.value, this.choice$.value);
  }

  private resolve(groups: readonly ProjectGroup[], workflows: Workflows, open: Project | null, choice: HomeChoice): { project: Project | null; workflow: string } {
    const known = workflows.workflows.map((workflow) => workflow.name);
    return {
      project: startProject(groups.map((group) => group.project), choice, open),
      workflow: startWorkflow(known, workflows.default, choice),
    };
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
