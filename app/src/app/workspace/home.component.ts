import { AfterViewChecked, ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, inject, Input, OnDestroy, Output, QueryList, ViewChild, ViewChildren } from '@angular/core';
import { BehaviorSubject, combineLatest, distinctUntilChanged, map, type Observable, shareReplay, Subscription } from 'rxjs';
import type { Command } from './commands.model';
import { HomeCardComponent } from './home-card.component';
import {
  type HomeAgentState,
  homeAgentState,
  type HomeCard,
  type HomeChoice,
  type HomeQueue,
  homeQueue,
  NO_HOME_CHOICE,
  primaryCardAction,
  readHomeChoice,
  startProject,
} from './home.model';
import {
  type Choice,
  composerAction,
  type EntryLine,
  FIRST_NEW_THREADS,
  insertNewline,
  type MessageSource,
  type ModelLists,
  type NewThreads,
  NO_MODELS,
  NO_WORKFLOWS,
  type Project,
  type ProjectGroup,
  type Starting,
  type Thread,
  type Voice,
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

/** A reply placed into an open card from somewhere else, such as findings to pass on. */
export interface CardNote {
  readonly threadId: string;
  readonly text: string;
  readonly to: Voice;
  /** Follow what is already typed in the reply instead of replacing it. */
  readonly add?: boolean;
}

/** Which small list under the prompt's buttons is open. */
type Chooser = 'project' | null;

/** Everything the template shows, at one moment. */
interface HomeView {
  readonly queue: HomeQueue;
  readonly text: string;
  readonly projects: readonly Project[];
  readonly project: Project | null;
  readonly installed: readonly string[];
  readonly models: ModelLists;
  readonly remembered: NewThreads;
  /** What the agent chooser shows and a new thread asks for. */
  readonly agent: HomeAgentState;
  readonly chooser: Chooser;
  readonly expandedId: string | null;
  readonly highlightedId: string | null;
  readonly drafts: Readonly<Record<string, string>>;
  readonly voices: Readonly<Record<string, Voice>>;
  readonly commands: readonly Command[];
  readonly putAwayId: string | null;
  readonly workflows: Workflows;
  readonly placeholder: string;
}

/**
 * Home: the prompt to start a thread, and the threads to work with from here.
 *
 * Starting a thread stays on Home. Each thread is a card: collapsed to a
 * line, or open in place so its latest turn can be read, answered and
 * acted on. One card is open at a time. The prompt holds the keys until
 * `Tab` hands them to the cards.
 */
@Component({
  selector: 'app-home',
  templateUrl: './home.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class HomeComponent implements AfterViewChecked, OnDestroy {
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
  /** The card open in place. Nothing when they are all collapsed. */
  @Output() readonly expanded = new EventEmitter<string | null>();
  @Output() readonly replied = new EventEmitter<{ thread: Thread; text: string; to: Voice }>();
  @Output() readonly answered = new EventEmitter<{ thread: Thread; text: string }>();
  @Output() readonly markedRead = new EventEmitter<Thread>();
  @Output() readonly stopped = new EventEmitter<Thread>();
  @Output() readonly planOpened = new EventEmitter<Thread>();
  @Output() readonly commandRun = new EventEmitter<Command>();
  @Output() readonly moreOpened = new EventEmitter<void>();
  @Output() readonly questionsPutAway = new EventEmitter<string>();
  @Output() readonly questionsRestored = new EventEmitter<void>();
  @Output() readonly linesChange = new EventEmitter<readonly EntryLine[]>();
  @Output() readonly sourceOpened = new EventEmitter<MessageSource>();

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
  private readonly expandedId$ = new BehaviorSubject<string | null>(null);
  private readonly highlightedId$ = new BehaviorSubject<string | null>(null);
  private readonly drafts$ = new BehaviorSubject<Readonly<Record<string, string>>>({});
  private readonly voices$ = new BehaviorSubject<Readonly<Record<string, Voice>>>({});
  private readonly commands$ = new BehaviorSubject<readonly Command[]>([]);
  private readonly putAwayId$ = new BehaviorSubject<string | null>(null);
  private seenNote: CardNote | null = null;
  private scrollTo: string | null = null;

  @ViewChild('prompt') private prompt?: ElementRef<HTMLTextAreaElement>;
  @ViewChild('list') private list?: ElementRef<HTMLElement>;
  @ViewChildren(HomeCardComponent) private cards?: QueryList<HomeCardComponent>;

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

  /** The card the workspace last had open, so it is open again when Home returns. */
  @Input() set expandedId(id: string | null) {
    if (id !== this.expandedId$.value) this.expandedId$.next(id);
  }

  /** The thread just started, highlighted until the workspace says otherwise. */
  @Input() set highlightedId(id: string | null) {
    const arrived = id !== null && id !== this.highlightedId$.value;
    this.highlightedId$.next(id);
    if (!arrived) return;
    this.scrollTo = id;
  }

  /** Scrolls to the highlighted card once it is drawn, which can be after the thread update arrives. */
  ngAfterViewChecked(): void {
    if (!this.scrollTo) return;
    const card = this.host.nativeElement.querySelector(`[data-thread="${this.scrollTo}"]`);
    if (!card) return;
    this.scrollTo = null;
    card.scrollIntoView({ block: 'nearest' });
  }

  /** Commands for the open card, the same ones the menu would run. */
  @Input() set commands(commands: readonly Command[]) {
    this.commands$.next(commands);
  }

  /** The questions put away, so the reply can be written instead. */
  @Input() set putAwayId(id: string | null) {
    this.putAwayId$.next(id);
  }

  private seenClear = -1;

  /** Clears one card's reply after it was sent. */
  @Input() set cardClear(clear: { id: string; at: number } | null) {
    if (!clear || clear.at === this.seenClear) return;
    this.seenClear = clear.at;
    this.drafts$.next({ ...this.drafts$.value, [clear.id]: '' });
  }

  /** A reply drafted elsewhere for the open card. */
  @Input() set cardNote(note: CardNote | null) {
    if (!note || note === this.seenNote) return;
    this.seenNote = note;
    const typed = (this.drafts$.value[note.threadId] ?? '').trim();
    const text = note.add && typed ? `${typed} ${note.text}` : note.text;
    this.drafts$.next({ ...this.drafts$.value, [note.threadId]: text });
    this.voices$.next({ ...this.voices$.value, [note.threadId]: note.to });
    if (this.expandedId$.value !== note.threadId) this.expand(note.threadId);
    setTimeout(() => this.cardOf(note.threadId)?.focusReply());
  }

  private readonly rest$: Observable<Pick<HomeView, 'queue' | 'projects' | 'project' | 'installed' | 'models' | 'agent' | 'workflows'>> = combineLatest([
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
        queue: homeQueue(groups, workflows),
        projects,
        project: startProject(projects, choice, open),
        installed,
        models,
        agent: homeAgentState(installed, models, workflows, picked, starting),
        workflows,
      };
    }),
  );

  readonly view$: Observable<HomeView> = combineLatest([
    this.rest$,
    this.text$,
    this.chooser$,
    this.rememberedAgents$,
    this.expandedId$,
    this.highlightedId$,
    this.drafts$,
    this.voices$,
    this.commands$,
    this.putAwayId$,
  ]).pipe(
    map(([rest, text, chooser, remembered, expandedId, highlightedId, drafts, voices, commands, putAwayId]) => ({
      ...rest,
      remembered,
      text,
      chooser,
      expandedId: rest.queue.cards.some((card) => card.thread.id === expandedId) ? expandedId : null,
      highlightedId,
      drafts,
      voices,
      commands,
      putAwayId,
      placeholder: 'Start a thread',
    })),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** Whether anything at all is on Home, for the calm layout. */
  readonly calm$: Observable<boolean> = this.view$.pipe(
    map((view) => view.queue.cards.length === 0),
    distinctUntilChanged(),
  );

  /** Drops an open card once its thread has left Home, so the keys are not left on nothing. */
  private readonly staying: Subscription = this.view$.subscribe((view) => {
    const id = this.expandedId$.value;
    if (!id || view.expandedId !== null) return;
    queueMicrotask(() => {
      if (this.expandedId$.value === id) this.expand(null);
    });
  });

  ngOnDestroy(): void {
    this.staying.unsubscribe();
  }

  updateText(event: Event): void {
    this.setText((event.target as HTMLTextAreaElement).value);
    this.chooser$.next(null);
  }

  /** Answers the keys of the prompt: Enter sends, Shift+Enter and Control+Enter start a new line, Tab goes to the cards, and Escape clears. */
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
        if (!view.queue.cards.length) return;
        event.preventDefault();
        return this.focusList(view);
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

  /**
   * Answers the keys of the cards.
   *
   * `j` and `k` move and open the card they land on. Enter does what that
   * card needs. Escape closes it and returns to the prompt. Letters that
   * belong to the open card stay here, and the rest are left to the screen.
   */
  handleListKey(event: KeyboardEvent, view: HomeView): void {
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (target?.closest('textarea, input, select, app-questions')) return;
    if (event.key === 'Enter' && target?.closest('button')) return;
    if (!this.answerCardKey(event, view)) return;
    event.preventDefault();
    event.stopPropagation();
  }

  /**
   * Handles Home keys outside the prompt and the cards.
   *
   * Escape dismisses the open chooser. With a card open, the card's keys
   * work from anywhere that is not a text box. `j` and `k` with nothing
   * open step into the first card.
   *
   * Left alone while another screen covers Home, and while the person types.
   */
  @HostListener('document:keydown', ['$event'])
  handleBareKey(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || event.isComposing) return;
    if (this.host.nativeElement.closest('[inert]')) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (document.querySelector('app-command-menu, ion-alert')) return;
    if (event.key === 'Escape' && this.chooser$.value) {
      event.preventDefault();
      this.chooser$.next(null);
      target?.closest('.home-chooser')?.querySelector<HTMLButtonElement>('.home-option')?.focus();
      return;
    }
    if (target && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable)) return;
    if (target?.closest('[data-home-list], app-questions')) return;
    let view: HomeView | undefined;
    this.view$.subscribe((next) => (view = next)).unsubscribe();
    if (!view) return;
    if (!view.expandedId && (event.key === 'j' || event.key === 'k' || event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      if (!view.queue.cards.length) return;
      event.preventDefault();
      return this.focusList(view);
    }
    if (!view.expandedId) return;
    if (!this.answerCardKey(event, view)) return;
    event.preventDefault();
  }

  /** Opens the card, or closes it when it was already open. */
  toggle(card: HomeCard): void {
    this.expand(this.expandedId$.value === card.thread.id ? null : card.thread.id);
    this.focusListElement();
  }

  rememberDraft(thread: Thread, text: string): void {
    this.drafts$.next({ ...this.drafts$.value, [thread.id]: text });
  }

  rememberVoice(thread: Thread, to: Voice): void {
    this.voices$.next({ ...this.voices$.value, [thread.id]: to });
  }

  rememberLines(card: HomeCard, lines: readonly EntryLine[]): void {
    if (card.thread.id === this.expandedId$.value) this.linesChange.emit(lines);
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

  /** Moves the keys to the open card's reply. */
  focusReply(): void {
    const id = this.expandedId$.value;
    if (id) this.cardOf(id)?.focusReply();
  }

  /** Moves the keys to the open card's questions. */
  focusQuestions(): void {
    const id = this.expandedId$.value;
    if (id) this.cardOf(id)?.focusQuestions();
  }

  private answerCardKey(event: KeyboardEvent, view: HomeView): boolean {
    const cards = view.queue.cards;
    const at = cards.findIndex((card) => card.thread.id === this.expandedId$.value);
    const card = at < 0 ? undefined : cards[at];
    const step = (index: number) => this.expand(cards[index]?.thread.id ?? null);
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        step(at < 0 ? 0 : Math.min(cards.length - 1, at + 1));
        return true;
      case 'k':
      case 'ArrowUp':
        step(at < 0 ? 0 : Math.max(0, at - 1));
        return true;
      case 'Enter':
        if (card) this.act(card, this.commands$.value);
        return true;
      case 'r':
        if (card && card.thread.status !== 'running') this.cardOf(card.thread.id)?.focusReply();
        return !!card;
      case 'o':
        if (card) this.opened.emit(card.thread);
        return !!card;
      case 'm':
        if (card?.thread.unread && card.thread.asking !== true) this.markedRead.emit(card.thread);
        return !!card && card.thread.unread && card.thread.asking !== true;
      case 'q':
        if (card) this.cardOf(card.thread.id)?.focusQuestions();
        return !!card;
      case 'Escape':
        this.expand(null);
        this.focusPrompt();
        return true;
      default:
        return false;
    }
  }

  private act(card: HomeCard, commands: readonly Command[]): void {
    switch (primaryCardAction(card, commands)) {
      case 'apply':
        return this.commandRun.emit(this.commandFor('change:apply', card.thread) ?? this.localCommand(card.thread, 'apply'));
      case 'catchUp':
        return this.commandRun.emit(this.commandFor('change:catchUp', card.thread) ?? this.localCommand(card.thread, 'catchUp'));
      case 'questions':
        return this.cardOf(card.thread.id)?.focusQuestions();
      case 'reply':
        return this.cardOf(card.thread.id)?.focusReply();
      case 'markRead':
        return this.markedRead.emit(card.thread);
      case 'open':
        return this.opened.emit(card.thread);
    }
  }

  private commandFor(id: string, thread: Thread): Command | null {
    return this.commands$.value.find((command) => command.id === id && commandThread(command) === thread.id) ?? null;
  }

  /**
   * A stand-in command when the menu's list has not caught up with the card.
   *
   * Enter still has to apply or catch up. The real command replaces this
   * as soon as the list names it.
   */
  private localCommand(thread: Thread, which: 'apply' | 'catchUp'): Command {
    return {
      id: which === 'apply' ? 'change:apply' : 'change:catchUp',
      title: which === 'apply' ? 'Apply to the project' : 'Catch up with the project',
      detail: '',
      group: 'change',
      key: null,
      leader: null,
      live: false,
      asking: false,
      badge: '',
      action: which === 'apply' ? { kind: 'applyChange', thread } : { kind: 'catchUp', thread },
    };
  }

  private expand(id: string | null): void {
    if (id === this.expandedId$.value) return;
    this.expandedId$.next(id);
    this.expanded.emit(id);
    if (!id) this.linesChange.emit([]);
  }

  private cardOf(id: string): HomeCardComponent | undefined {
    return this.cards?.find((card) => card.threadId === id);
  }

  private setText(text: string): void {
    this.text$.next(text);
    this.textChanged.emit(text);
  }

  private focusPrompt(): void {
    requestAnimationFrame(() => {
      const box = this.prompt?.nativeElement;
      box?.focus();
      box?.setSelectionRange(box.value.length, box.value.length);
    });
  }

  private focusList(view: HomeView): void {
    if (!view.expandedId) this.expand(view.queue.cards[0]?.thread.id ?? null);
    this.focusListElement();
  }

  private focusListElement(): void {
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

function commandThread(command: Command): string | null {
  const action = command.action;
  return 'thread' in action ? action.thread.id : null;
}
