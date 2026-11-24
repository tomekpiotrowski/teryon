import { ChangeDetectionStrategy, Component, ElementRef, HostListener, inject, OnDestroy, ViewChild } from '@angular/core';
import { toObservable } from '@angular/core/rxjs-interop';
import { AlertController } from '@ionic/angular/alert-controller';
import { ErnoAlertsService } from 'erno-angular';
import {
  BehaviorSubject,
  catchError,
  combineLatest,
  distinctUntilChanged,
  EMPTY,
  filter,
  fromEvent,
  map,
  mergeMap,
  type Observable,
  of,
  shareReplay,
  skip,
  pairwise,
  startWith,
  Subject,
  Subscription,
  switchMap,
  take,
  takeUntil,
  tap,
  timer,
} from 'rxjs';
import { ThemeService } from '../theme.service';
import { type NewProject } from './command-menu.component';
import type { Opener } from '../service/protocol';
import { type SeenFinish } from './board.model';
import { type Command, type CommandGroup, commandList, keyHint, keyOutcome, menuAsked, putAwayScope, SHORTCUTS_KEY } from './commands.model';
import { askDraft } from './files.model';
import { citationFromHref, type Citation } from './markdown';
import { homeList } from './home.model';
import { type CardNote, type StartRequest } from './home.component';
import { type Asked, isRecommended } from './questions.model';
import { type Commit, revertDraft } from './history.model';
import { shortcutSections } from './shortcuts.model';
import { UsageComponent } from './usage.component';
import { WorkspaceApi } from './workspace.api';
import {
  agentLabel,
  coversThread,
  type Choice,
  allowedThreadAgents,
  FIRST_NEW_THREADS,
  choosing,
  choosingAutomatic,
  type NewThreads,
  threadChoice,
  APPLY_RESOLVING_NOTICE,
  CATCH_UP_REQUEST,
  type ChangeCard,
  changeCard,
  changeBadge,
  citationPlace,
  citationRoots,
  codePlace,
  copyPlace,
  discardWarning,
  composerAction,
  insertNewline,
  type EntryLine,
  type MessageSource,
  pendingQuestions,
  sourceLabel,
  threadLines,
  groupThreads,
  NO_MODELS,
  NO_PLACE,
  NO_WORKFLOWS,
  NOTHING_SELECTED,
  type Place,
  placeIn,
  type Project,
  type ProjectGroup,
  refusalText,
  CONNECTING,
  connectionText,
  offersRestart,
  resolveSelection,
  runningCount,
  type Screen,
  screenAfter,
  type Selection,
  seenNow,
  readNewThreads,
  startingChoice,
  isQuiet,
  putAwayText,
  stageText,
  type Thread,
  threadLabel,
  type Voice,
  findingsDraft,
  reviewerLabel,
  reviewerOf,
} from './workspace.model';
import { WorkspaceStore } from './workspace.store';

/** Where the browser remembers who new threads start with. */
const NEW_THREADS_KEY = 'teryon.newThreads';

/** What the browser remembers of who new threads start with. Nothing where there is no storage, or nothing was kept. */
function rememberedNewThreads(): string | null {
  try {
    return localStorage.getItem(NEW_THREADS_KEY);
  } catch {
    return null;
  }
}

function rememberNewThreads(newThreads: NewThreads): void {
  try {
    localStorage.setItem(NEW_THREADS_KEY, JSON.stringify(newThreads));
  } catch {
    // The choice only lasts for this visit.
  }
}

/** Everything the template needs about the workspace at one moment. */
/** The keys the board answers itself. */
const BOARD_KEYS = ['h', 'j', 'k', 'l', 'a', 'x', 'Enter'];

interface WorkspaceView {
  readonly groups: readonly ProjectGroup[];
  readonly project: Project | null;
  readonly thread: Thread | null;
  readonly lines: readonly EntryLine[];
  readonly running: boolean;
  /** How many agents are running in the open project. */
  readonly liveAgents: number;
  /** How many threads need the person, across every project, for Home. */
  readonly needing: number;
  /** The open project's threads, for the history to name where a commit came from. */
  readonly projectThreads: readonly Thread[];
  /** The part of the open project's repository the files and the history show. */
  readonly place: Place;
  /** The project directory and, when the open thread has a copy, that copy. Citations resolve against them. */
  readonly roots: readonly string[];
  /** Where the open thread's change stands. Nothing when the thread never changed anything. */
  readonly change: ChangeCard | null;
  /** The open thread's stage, as a person reads it, such as `Implementing`. Empty without a thread. */
  readonly stage: string;
}

/** A button on the change line: a command with its keys. */
interface LineAction {
  readonly command: Command;
  readonly hint: string | null;
}

/** The line above the message box about the open thread's change. */
interface ChangeLine {
  readonly card: ChangeCard;
  /** The likely next steps, as buttons. */
  readonly actions: readonly LineAction[];
}

/**
 * The workspace screen: one thread, the files or the history, with the menu as the way around.
 *
 * Everything shown is derived from observables: the synchronised lists in
 * `WorkspaceStore` and a handful of subjects holding what the person has
 * selected and typed. Handlers only push into those subjects or send a request.
 *
 * Whatever a person can do here is a command in `commands.model.ts`. The
 * menu lists them and the keys run them, so this screen has no other
 * place where projects, threads or settings are reached.
 */
@Component({
  selector: 'app-workspace',
  templateUrl: './workspace.page.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class WorkspacePage implements OnDestroy {
  readonly theme = inject(ThemeService);
  private readonly store = inject(WorkspaceStore);
  private readonly api = inject(WorkspaceApi);
  private readonly alerts = inject(ErnoAlertsService);
  private readonly alertController = inject(AlertController);
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);

  readonly threadLabel = threadLabel;
  readonly agentLabel = agentLabel;
  readonly allowedThreadAgents = allowedThreadAgents;
  readonly firstNewThreads = FIRST_NEW_THREADS;
  readonly threadChoice = threadChoice;
  readonly coversThread = coversThread;
  readonly installed$ = this.store.installed$;
  readonly models$ = this.store.models$;
  readonly emptyModels = NO_MODELS;
  readonly workflows$ = this.store.workflows$;
  readonly emptyWorkflows = NO_WORKFLOWS;
  readonly connectionText = connectionText;
  readonly offersRestart = offersRestart;

  readonly selection$ = new BehaviorSubject<Selection>(NOTHING_SELECTED);
  /** The project the menu picked for Home's prompt. Separate from the open project, which Home does not have. */
  readonly promptProject$ = new BehaviorSubject<Project | null>(null);
  readonly draft$ = new BehaviorSubject('');
  /** The card open on Home. Separate from the thread open on its own screen. */
  readonly homeThreadId$ = new BehaviorSubject<string | null>(null);
  /** The thread just started, so Home can point it out. Cleared after a moment. */
  readonly highlightedId$ = new BehaviorSubject<string | null>(null);
  /** A reply written into the open card, from a plan, the files or a review. */
  readonly cardNote$ = new BehaviorSubject<CardNote | null>(null);
  /** Clears a card's reply once it has been sent. */
  readonly cardClear$ = new BehaviorSubject<{ id: string; at: number } | null>(null);
  /** The open card's conversation, for a drafted reply such as a review's findings. */
  readonly homeLines$ = new BehaviorSubject<readonly EntryLine[]>([]);
  /** A thread made for a message that was refused, so trying again reuses it. */
  private unsentThread: { readonly projectId: string; readonly thread: Thread } | null = null;
  private highlightTimer: ReturnType<typeof setTimeout> | undefined;
  private preferCardReply = false;
  /** The agent and model new threads start with, and the model remembered for each agent. */
  readonly newThreads$ = new BehaviorSubject<NewThreads>(readNewThreads(rememberedNewThreads()));
  private readonly remembering: Subscription = this.newThreads$.pipe(skip(1)).subscribe(rememberNewThreads);
  /** The agent and model picked for new threads. Nothing while the first stage chooses. */
  readonly pickedChoice$: Observable<Choice | null> = this.newThreads$.pipe(map(startingChoice));
  /** Who the default workflow's first stage chooses for new threads. */
  readonly starting$ = this.store.starting$;
  readonly menuOpen$ = new BehaviorSubject(false);
  readonly shortcutsOpen$ = new BehaviorSubject(false);
  /** The one section the menu was opened on, such as the thread's changes. Nothing for the whole menu. */
  readonly menuSection$ = new BehaviorSubject<CommandGroup | null>(null);
  /** Which of the open thread's agents the message box writes to. */
  readonly to$ = new BehaviorSubject<Voice>('author');
  /** True while a request is on its way, so a second press does not send it twice. */
  readonly sending$ = new BehaviorSubject(false);
  /** Which screen fills the workspace. Home and a thread are both `thread`. */
  readonly screen$ = new BehaviorSubject<Screen>('thread');
  /** Whether the board shows every project's threads rather than the open project's. */
  readonly boardEvery$ = new BehaviorSubject(false);
  /** The branch or commit the files and the history were last asked to show. */
  readonly place$ = new BehaviorSubject<Place>(NO_PLACE);
  /**
   * The file citation the code is opening.
   *
   * Held here because the code screen is created on the way in and destroyed
   * on the way out. Leaving the code drops it, so the mark does not stick
   * while the open file stays remembered.
   */
  readonly citation$ = new BehaviorSubject<Citation | null>(null);
  /** Kept so the markdown pipe sees the same directories until they actually change. */
  private shownRoots: readonly string[] = [];
  private shownActiveRoots: readonly string[] = [];
  /** A leader key pressed, waiting for the key that picks a command of its family. */
  readonly leader$ = new BehaviorSubject<string | null>(null);

  readonly reviewerOf = reviewerOf;
  readonly reviewerLabel = reviewerLabel;
  readonly sourceLabel = sourceLabel;
  readonly isRecommended = isRecommended;

  /**
   * The open project and thread, as soon as they are chosen.
   *
   * Apart from `view$`, which waits for the thread's conversation to arrive
   * and until then still holds the thread opened before. A key pressed in
   * between has to act on the thread the person just opened.
   */
  private readonly selected$ = combineLatest([
    combineLatest([this.store.projects$, this.store.threads$]).pipe(
      map(([projects, threads]) => groupThreads(projects, threads)),
    ),
    this.selection$,
  ]).pipe(
    map(([groups, selection]) => ({ groups, ...resolveSelection(groups, selection) })),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /**
   * The thread the commands and the plan act on.
   *
   * The thread open on its own screen wins. Otherwise it is the card open on
   * Home, so a plan or a change can be handled without leaving Home.
   */
  readonly activeThread$: Observable<Thread | null> = combineLatest([this.selected$, this.homeThreadId$]).pipe(
    map(([{ groups, thread }, id]) => thread ?? groups.flatMap((group) => group.threads).find((candidate) => candidate.id === id) ?? null),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** The citation directories for the thread open on its own screen or in a Home card. */
  readonly activeRoots$: Observable<readonly string[]> = combineLatest([this.selected$, this.activeThread$]).pipe(
    map(([{ groups, project }, active]) => {
      const owner = active ? groups.find((group) => group.project.id === active.project_id)?.project ?? project : project;
      const next = citationRoots(owner, active);
      if (next.length === this.shownActiveRoots.length && next.every((root, index) => root === this.shownActiveRoots[index])) return this.shownActiveRoots;
      this.shownActiveRoots = next;
      return next;
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  readonly view$: Observable<WorkspaceView> = combineLatest([this.selected$, this.place$, this.store.workflows$]).pipe(
    switchMap(([{ groups, project, thread }, place, workflows]) => {
      const projectThreads = groups.find((group) => group.project.id === project?.id)?.threads ?? [];
      const entries$ = thread ? this.store.entries$(thread.id) : of([]);
      return entries$.pipe(
        map((entries) => ({
          groups,
          project,
          thread,
          lines: threadLines(entries),
          running: thread?.status === 'running',
          liveAgents: projectThreads.filter((candidate) => candidate.status === 'running').length,
          needing: homeList(groups).total,
          projectThreads,
          place: project ? placeIn(place, project.id) : NO_PLACE,
          roots: this.rootsFor(project, thread),
          change: thread ? changeCard(thread, project) : null,
          stage: thread ? stageText(workflows, thread) : '',
        })),
      );
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /**
   * Who the message box writes to: the agent the person picked, or the author while the thread has no reviewer.
   *
   * Goes back to the author whenever another thread opens.
   */
  readonly target$: Observable<Voice> = combineLatest([
    this.view$.pipe(
      map((view) => view.thread),
      distinctUntilChanged((before, after) => before?.id === after?.id && !!before?.reviewer === !!after?.reviewer),
      tap(() => this.to$.next('author')),
    ),
    this.to$,
  ]).pipe(
    map(([thread, to]) => (thread?.reviewer ? to : 'author')),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** The questions the open thread's agent waits for answers to. Nothing once anything was said after them. */
  private readonly pending$: Observable<Asked | null> = this.view$.pipe(
    map((view) => pendingQuestions(view.lines, view.running)),
    distinctUntilChanged((before, after) => before?.id === after?.id),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /** The questions the person put away with Escape, to write a message instead. Asking again brings new ones up. */
  readonly putAway$ = new BehaviorSubject<string | null>(null);

  /** The questions to answer above the message box: the pending ones, unless put away. */
  readonly questions$: Observable<Asked | null> = combineLatest([this.pending$, this.putAway$]).pipe(
    map(([pending, putAway]) => (pending && pending.id !== putAway ? pending : null)),
  );

  /** The commands on offer, for the menu to list and the keys to run. */
  readonly commands$: Observable<readonly Command[]> = combineLatest([
    this.view$,
    this.newThreads$,
    toObservable(this.theme.theme),
    this.store.opens$,
    this.models$,
    this.installed$,
    this.starting$,
    this.target$,
    this.store.workflows$,
    combineLatest([this.pending$, this.putAway$]).pipe(map(([pending, putAway]) => pending !== null && pending.id === putAway)),
    this.screen$,
    this.boardEvery$,
    this.homeThreadId$,
  ]).pipe(
    map(([view, newThreads, theme, opens, models, installed, starting, to, workflows, questionsPutAway, screen, everyProject, homeThreadId]) => {
      const cardScreen = screen === 'thread' || screen === 'files' || screen === 'plan';
      const homeThread = view.thread || !cardScreen ? null : view.groups.flatMap((group) => group.threads).find((candidate) => candidate.id === homeThreadId) ?? null;
      const thread = view.thread ?? homeThread;
      const project = thread ? view.groups.find((group) => group.project.id === thread.project_id)?.project ?? view.project : view.project;
      return commandList({
        ...view,
        thread,
        project,
        newThreads,
        theme,
        opens,
        models,
        installed,
        starting,
        to,
        workflows,
        questionsPutAway,
        putAwayScope: putAwayScope(screen, everyProject, project),
      });
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  /**
   * The line about the open thread's change, with its likely next steps as buttons.
   *
   * The buttons are the same commands the menu and the `W` keys run, so the
   * line teaches those keys while it offers them. The rest are in the menu.
   */
  readonly changeLine$: Observable<ChangeLine | null> = combineLatest([this.view$, this.commands$]).pipe(
    map(([view, commands]) => {
      if (!view.change) return null;
      const actions = view.change.next
        .map((id) => commands.find((command) => command.id === id))
        .filter((command): command is Command => command !== undefined)
        .map((command) => ({ command, hint: keyHint(command) }));
      return { card: view.change, actions };
    }),
  );

  /** What the menu lists: every command, or one section when opened on it. */
  readonly menuCommands$: Observable<readonly Command[]> = combineLatest([this.commands$, this.menuSection$]).pipe(
    map(([commands, section]) => (section ? commands.filter((command) => command.group === section) : commands)),
  );

  /** The number keys offered by Usage, while that screen is present. */
  private readonly usageAgentCounts$ = new BehaviorSubject<Observable<number>>(of(0));

  @ViewChild(UsageComponent) private set usage(usage: UsageComponent | undefined) {
    this.usageAgentCounts$.next(usage?.agentCount$ ?? of(0));
  }

  readonly shortcuts$ = combineLatest([
    this.commands$,
    this.view$,
    this.screen$,
    this.usageAgentCounts$.pipe(switchMap((count$) => count$)),
  ]).pipe(
    map(([commands, view, screen, usageAgents]) => {
      const place = coversThread(screen, view.project) ? screen : view.thread ? 'thread' : 'home';
      const badge = view.thread ? changeBadge(view.thread) : '';
      return shortcutSections(commands, place, badge, {
        waitingThreads: view.needing > 0,
        homeCards: view.needing > 0 || runningCount(view.groups) > 0,
        usageAgents,
      });
    }),
  );

  /** The commands as last offered, for a key press, which has to be answered at once. */
  private commands: readonly Command[] = [];
  private readonly offering: Subscription = this.commands$.subscribe((commands) => (this.commands = commands));

  /**
   * Tells the server when the person is looking at a thread that is marked as unread.
   *
   * The mark comes back cleared through the synchronisation, which also
   * clears it on the person's other devices.
   */
  private readonly seeing: Subscription = combineLatest([
    this.view$,
    this.screen$,
    fromEvent(document, 'visibilitychange').pipe(
      startWith(null),
      map(() => document.visibilityState === 'visible'),
    ),
  ])
    .pipe(
      map(([view, screen, visible]) => (seenNow(view.thread, screen, visible) ? view.thread?.id : undefined)),
      distinctUntilChanged(),
      filter((threadId): threadId is string => threadId !== undefined),
      mergeMap((threadId) => this.api.markSeen(threadId).pipe(catchError(() => EMPTY))),
    )
    .subscribe();

  private readonly typingWanted$ = new Subject<void>();

  /**
   * Puts the cursor at the end of the message box when asked to.
   *
   * A command that opens another thread or project replaces the message box
   * a moment later, once the thread's lines are read. So the cursor is put
   * in the box that is there now and again in the one that follows, if one
   * follows within a second.
   */
  private readonly typing: Subscription = this.typingWanted$
    .pipe(
      switchMap(() =>
        combineLatest([this.view$, this.screen$]).pipe(
          map(([view, screen]) => `${screen}:${view.project?.id}:${view.thread?.id}`),
          distinctUntilChanged(),
          take(2),
          takeUntil(timer(1000)),
        ),
      ),
    )
    .subscribe(() =>
      requestAnimationFrame(() => {
        const prefer = this.preferCardReply;
        this.preferCardReply = false;
        const boxes = Array.from(this.host.nativeElement.querySelectorAll<HTMLTextAreaElement>('textarea'));
        const cardBox = prefer ? this.host.nativeElement.querySelector<HTMLTextAreaElement>('[data-home-reply]') : null;
        const box = cardBox && !cardBox.closest('[inert]') ? cardBox : boxes.find((candidate) => !candidate.closest('[inert]'));
        box?.focus();
        box?.setSelectionRange(box.value.length, box.value.length);
      }),
    );

  /** Whether the menu was opened from the keyboard, so that closing it goes back to typing. */
  private menuOpenedByKey = false;
  private shortcutTrigger: HTMLElement | null = null;

  @ViewChild('thread') private thread?: ElementRef<HTMLElement>;

  /** Keeps the newest line in sight as a thread grows. */
  private readonly following: Subscription = this.view$
    .pipe(
      map((view) => `${view.thread?.id}:${view.lines.length}:${view.running}`),
      distinctUntilChanged(),
    )
    .subscribe(() => requestAnimationFrame(() => this.thread?.nativeElement.scrollTo({ top: Number.MAX_SAFE_INTEGER })));

  /** How the connection to the local service stands, to explain an empty screen. */
  readonly connection$ = this.store.connection$;
  /** Shown until the connection says otherwise. */
  readonly connecting = CONNECTING;

  /** Restarts a service this window cannot talk to, once the person chose to. */
  restartService(): void {
    this.store.restartService();
  }

  /** The threads and projects as last reported, so a confirmation counts what is quiet now. */
  private threadsNow: readonly Thread[] = [];
  private projectsNow: readonly Project[] = [];
  private readonly listed: Subscription = combineLatest([this.store.threads$, this.store.projects$]).subscribe(([threads, projects]) => {
    this.threadsNow = threads;
    this.projectsNow = projects;
  });

  constructor() {
    this.store.start();
  }

  /**
   * Drops a citation once the code is left, and drops a project kept only so the files or the history could open.
   *
   * Home names no project. A card's project is kept while the files, the
   * history or the plan of that card are open, and let go on the way back,
   * so those screens are offered only for an expanded card.
   */
  private readonly leavingScreen: Subscription = this.screen$.pipe(distinctUntilChanged(), pairwise()).subscribe(([before, after]) => {
    if (before === 'files' && after !== 'files') this.citation$.next(null);
    if (after === 'thread' && !this.selection$.value.threadId && this.selection$.value.projectId) this.selection$.next(NOTHING_SELECTED);
  });

  ngOnDestroy(): void {
    this.following.unsubscribe();
    this.remembering.unsubscribe();
    this.offering.unsubscribe();
    this.typing.unsubscribe();
    this.seeing.unsubscribe();
    this.listed.unsubscribe();
    this.leavingScreen.unsubscribe();
    clearTimeout(this.highlightTimer);
  }

  /**
   * Answers the keys that work anywhere on the screen.
   *
   * `/` and Ctrl+K open the menu, and Ctrl+K or Escape close it. A bare
   * letter runs the command bound to it, as the hints on screen say, or
   * waits for a second one when it leads a family, such as `w` for the
   * thread's change. While the menu is open the letters are left alone.
   * `?` opens the shortcuts list and leaves typing to its filter. With both
   * dialogs closed, Escape returns from any other screen to the thread,
   * including when a filter has focus. Escape is left to confirmations,
   * text composition, and any control that has already handled the key.
   */
  @HostListener('document:keydown', ['$event'])
  handleShortcut(event: KeyboardEvent): void {
    if (event.key === 'Escape' && (event.defaultPrevented || event.isComposing || document.querySelector('ion-alert'))) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    const press = {
      key: event.key,
      ctrlKey: event.ctrlKey,
      metaKey: event.metaKey,
      altKey: event.altKey,
      element: target?.tagName ?? '',
      editable: target?.isContentEditable ?? false,
      messageBox: target?.matches('textarea') ?? false,
    };

    if (this.shortcutsOpen$.value) {
      if (event.isComposing) return;
      const typed = this.host.nativeElement.querySelector<HTMLInputElement>('app-shortcuts input')?.value ?? '';
      const bare = !event.ctrlKey && !event.metaKey && !event.altKey;
      if (event.key === 'Escape' || (event.key === SHORTCUTS_KEY && bare && !typed)) {
        event.preventDefault();
        return this.closeShortcuts();
      }
      if ((event.key === '/' && bare && !typed) || (event.key.toLowerCase() === 'k' && menuAsked(press, ''))) {
        event.preventDefault();
        return this.openMenu(true);
      }
      return;
    }

    if (this.menuOpen$.value) {
      const closing = event.key === 'Escape' || (press.key.toLowerCase() === 'k' && menuAsked(press, ''));
      if (!closing) return;
      event.preventDefault();
      return this.closeMenu();
    }
    if (event.key === 'Escape' && this.screen$.value !== 'thread') {
      event.preventDefault();
      this.leader$.next(null);
      return this.screen$.next('thread');
    }
    // With a thread open and nothing else, Escape goes back to Home, where the prompt and the waiting threads are.
    if (event.key === 'Escape' && this.screen$.value === 'thread' && this.selection$.value.threadId && !press.editable && !['INPUT', 'TEXTAREA', 'SELECT'].includes(press.element.toUpperCase())) {
      event.preventDefault();
      return this.openHome();
    }
    // The board moves between cards with these, see `BoardComponent`; `h` would otherwise open the history.
    if (this.screen$.value === 'board' && this.leader$.value === null && BOARD_KEYS.includes(event.key)) return;
    if (menuAsked(press, this.draft$.value)) {
      event.preventDefault();
      return this.openMenu(true);
    }
    if (event.key === SHORTCUTS_KEY && (event.defaultPrevented || event.isComposing)) return;
    const outcome = keyOutcome(press, this.commands, this.leader$.value);
    if (!outcome) return;
    event.preventDefault();
    if (outcome.kind === 'wait') return this.leader$.next(outcome.leader);
    this.leader$.next(null);
    if (outcome.kind === 'run') this.run(outcome.command);
  }

  /**
   * Opens the menu, whole or on one section.
   *
   * `byKey` tells a key press from a tap, which should not end with a keyboard raised.
   */
  openMenu(byKey = false, section: CommandGroup | null = null): void {
    this.leader$.next(null);
    this.closeShortcuts(false);
    this.menuSection$.next(section);
    this.menuOpenedByKey = byKey;
    this.menuOpen$.next(true);
  }

  /** Closes the menu. Someone who opened it from the keyboard goes back to the message box. */
  closeMenu(restoreFocus = true): void {
    this.menuOpen$.next(false);
    if (restoreFocus && this.menuOpenedByKey) this.focusMessageBox();
    this.menuOpenedByKey = false;
  }

  openShortcuts(): void {
    if (this.shortcutsOpen$.value) return;
    const focused = document.activeElement;
    this.shortcutTrigger = focused instanceof HTMLElement && focused.closest('.workspace-layers') ? focused : null;
    this.leader$.next(null);
    this.closeMenu(false);
    this.shortcutsOpen$.next(true);
  }

  closeShortcuts(restoreFocus = true): void {
    if (!this.shortcutsOpen$.value) return;
    this.shortcutsOpen$.next(false);
    const trigger = this.shortcutTrigger;
    this.shortcutTrigger = null;
    if (restoreFocus && trigger) {
      requestAnimationFrame(() => {
        if (!this.shortcutsOpen$.value && !this.menuOpen$.value) trigger.focus();
      });
    }
  }

  /** Carries out a command chosen in the menu or asked for with its key. */
  run(command: Command): void {
    const action = command.action;
    if (action.kind === 'showShortcuts') return this.openShortcuts();
    this.closeMenu();
    switch (action.kind) {
      case 'openThread':
        return this.openThread(action.thread);
      case 'openProject':
        // A new thread is started from the prompt, so the card that was open is closed.
        return this.usePromptProject(action.project);
      case 'showHome':
        return this.openHome();
      case 'showBoard':
        return this.showBoard(action.everyProject);
      case 'chooseWorkflow':
        return this.chooseWorkflow(action.thread, action.workflow);
      case 'showScreen':
        if (action.screen === 'files') return this.showCode();
        if (action.screen === 'history') return this.showHistory();
        if (action.screen === 'plan') {
          this.keepHomeProject();
          return this.screen$.next(screenAfter(this.screen$.value, 'plan'));
        }
        return this.screen$.next(screenAfter(this.screen$.value, action.screen));
      case 'removeProject':
        return void this.confirmRemoveProject(action.project);
      case 'useChoice':
        return this.chooseForNewThreads(action.choice);
      case 'useAutomatic':
        return this.chooseAutomaticForNewThreads();
      case 'useChoiceInThread':
        return this.changeAgent(action.thread, action.to, action.choice);
      case 'toggleTheme':
        return this.theme.toggle();
      case 'stopAgent':
        return this.stop(action.thread);
      case 'applyChange':
        return this.applyChange(action.thread);
      case 'catchUp':
        return this.catchUp(action.thread);
      case 'exploreChange':
        return this.exploreChange(action.thread);
      case 'reviewChange':
        return this.reviewChange(action.thread, action.choice);
      case 'passFindings':
        return this.passFindings();
      case 'discardChange':
        return void this.confirmDiscard(action.thread);
      case 'sendForReview':
        return this.sendForReview(action.thread);
      case 'openReview':
        return this.openReview(action.thread);
      case 'copyLocation':
        return this.copyLocation(action.path);
      case 'openCopy':
        return this.openCopy(action.thread, action.opener);
      case 'draft':
        return this.draftAndReturn(action.text);
      case 'answerQuestions':
        if (!this.selection$.value.threadId && this.homeThreadId$.value) {
          this.putAway$.next(null);
          return;
        }
        this.screen$.next('thread');
        return this.putAway$.next(null);
      case 'putAwayQuiet':
        return this.confirmPutAway(action.projectId);
      case 'newProject':
        // The menu asks for the name and the directory itself, then calls `addProject`.
        return;
    }
  }

  /** Goes back to the thread with the file named in the message box, ready to be asked about. */
  askAbout(path: string): void {
    const card = this.cardTarget();
    this.draftAndReturn(askDraft(card ? '' : this.draft$.value, path), card !== null);
  }

  /**
   * Goes back to the thread with a request to revert a commit in the message box.
   *
   * Teryon never changes a repository itself. Reverting is work for an
   * agent, and the person still decides whether to send the request.
   */
  askToRevert(commit: Commit): void {
    const card = this.cardTarget();
    this.draftAndReturn(revertDraft(card ? '' : this.draft$.value, commit), card !== null);
  }

  /** Shows a branch of a project in the files and the history. Nothing means the directory as it is. */
  chooseBranch(project: Project, branch: string | null): void {
    this.place$.next({ projectId: project.id, threadId: null, branch, commit: null });
  }

  /**
   * Opens a file citation in the code.
   *
   * The link is an ordinary anchor, which is what survives Angular's cleaning
   * of the message HTML. Ctrl, Shift, Meta and the middle button are left to
   * the browser. This app does not use the fragment to pick a screen, so a
   * new tab just opens it as usual.
   */
  @HostListener('click', ['$event'])
  openCitation(event: MouseEvent): void {
    if (event.button !== 0 || event.ctrlKey || event.shiftKey || event.metaKey) return;
    const start = event.target instanceof Element ? event.target : null;
    const link = start?.closest('a.file-citation');
    if (!(link instanceof HTMLAnchorElement)) return;
    const citation = citationFromHref(link.getAttribute('href') ?? '');
    if (!citation) return;
    event.preventDefault();
    this.selected$.pipe(take(1)).subscribe(({ groups, project, thread }) => {
      const active = thread ?? groups.flatMap((group) => group.threads).find((candidate) => candidate.id === this.homeThreadId$.value) ?? null;
      const owner = active ? groups.find((group) => group.project.id === active.project_id)?.project ?? project : project;
      if (!owner) return;
      if (active && !thread) this.keepProject(active);
      this.citation$.next(citation);
      this.place$.next(citationPlace(this.place$.value, owner.id, active));
      this.screen$.next('files');
    });
  }

  /** The directories citations in the open project resolve against, stable until those directories change. */
  private rootsFor(project: Project | null, thread: Thread | null): readonly string[] {
    const next = citationRoots(project, thread);
    if (next.length === this.shownRoots.length && next.every((root, index) => root === this.shownRoots[index])) return this.shownRoots;
    this.shownRoots = next;
    return next;
  }

  /**
   * Shows the code, or goes back to the thread when it is already shown.
   *
   * Asked for from a thread with a copy of its own, the code is that copy's,
   * where its agent works. From the history the place chosen there stays,
   * since the two screens share it.
   */
  showCode(): void {
    if (this.screen$.value === 'thread') {
      this.keepHomeProject();
      this.selected$.pipe(take(1)).subscribe(({ groups, project, thread }) => {
        const active = thread ?? groups.flatMap((group) => group.threads).find((candidate) => candidate.id === this.homeThreadId$.value) ?? null;
        const owner = active ? groups.find((group) => group.project.id === active.project_id)?.project ?? project : project;
        if (owner) this.place$.next(codePlace(this.place$.value, owner.id, active));
      });
    }
    this.screen$.next(screenAfter(this.screen$.value, 'files'));
  }

  /**
   * Shows the history, or goes back when it is already shown.
   *
   * From Home the history is the expanded card's project, kept before the
   * screen changes so the history has a project to open.
   */
  showHistory(): void {
    this.keepHomeProject();
    this.screen$.next(screenAfter(this.screen$.value, 'history'));
  }

  /** The name of a thread's project, for the plan screen's own header. */
  projectLabel(thread: Thread | null, groups: readonly ProjectGroup[]): string {
    return groups.find((group) => group.project.id === thread?.project_id)?.project.name ?? '';
  }

  /** Opens the files on what a thread's agent changed. */
  exploreChange(thread: Thread): void {
    const change = thread.change;
    if (!change) return;
    this.keepProject(thread);
    this.place$.next(copyPlace(thread, change));
    this.screen$.next('files');
  }

  /** Shows the files of a project as a commit left them. */
  browseAt(project: Project, commit: string): void {
    this.place$.next({ ...placeIn(this.place$.value, project.id), commit });
    this.screen$.next('files');
  }

  /** Shows the thread a commit came from. */
  openThread(thread: Thread): void {
    this.selectThread(thread);
    this.screen$.next('thread');
  }

  /** Opens the thread that sent a cross-thread message, when it is still present. */
  openSourceThread(source: MessageSource): void {
    const thread = this.threadsNow.find((candidate) => candidate.id === source.thread_id);
    if (thread) this.openThread(thread);
  }

  selectProject(project: Project): void {
    this.selection$.next({ projectId: project.id, threadId: null });
    this.draft$.next('');
  }

  selectThread(thread: Thread): void {
    this.selection$.next({ projectId: thread.project_id, threadId: thread.id });
    this.draft$.next('');
  }

  /** Switches the message box between the thread's agent and its reviewer. */
  toggleTarget(): void {
    this.to$.next(this.to$.value === 'author' ? 'reviewer' : 'author');
  }

  reviewChange(thread: Thread, choice: Choice): void {
    this.api.reviewChange(thread.id, choice).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The change could not be reviewed.')),
    });
  }

  /** Opens the open thread's plan. */
  showPlan(): void {
    const thread = this.homeThreadId$.value;
    if (!this.selection$.value.threadId && thread) this.keepHomeProject();
    this.screen$.next('plan');
  }

  /** Opens a card's plan and comes back to that card. */
  showPlanFor(thread: Thread): void {
    this.keepProject(thread);
    this.screen$.next('plan');
  }

  /** Opens the board of the open project, or of every project; asked again, goes back to the thread. */
  showBoard(everyProject = false): void {
    this.boardEvery$.next(everyProject);
    this.screen$.next(screenAfter(this.screen$.value, 'board'));
  }

  /** Puts a thread on another workflow, from the menu. */
  chooseWorkflow(thread: Thread, workflow: string): void {
    this.api.chooseWorkflow(thread.id, workflow).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The workflow could not be changed.')),
    });
  }

  /** A thread opened from the board: it fills the screen in place of the board. */
  openFromBoard(thread: Thread): void {
    this.openThread(thread);
    this.screen$.next('thread');
  }

  /** Switches a thread's agent, or its reviewer, to another agent, model, or effort, from the next message on. */
  changeAgent(thread: Thread, to: Voice, choice: Choice): void {
    this.api.changeAgent(thread.id, to, choice).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The agent could not be changed.')),
    });
  }

  /** Puts what the reviewer found in the message box, addressed to the author, for the person to trim and send. */
  passFindings(): void {
    this.view$.pipe(take(1)).subscribe((view) => {
      this.to$.next('author');
      this.draftAndReturn(findingsDraft(view.thread ? view.lines : this.homeLines$.value));
    });
  }

  updateDraft(event: Event): void {
    this.draft$.next((event.target as HTMLTextAreaElement).value);
  }

  /** Sends the message on Enter, starts a new line on Shift+Enter or Control+Enter, and leaves the message box on Escape, so that the keys bound to letters work. */
  handleComposerKeydown(event: KeyboardEvent, view: WorkspaceView): void {
    const action = composerAction(event);
    if (!action) return;
    event.preventDefault();
    if (action === 'leave') return (event.target as HTMLElement).blur();
    if (action === 'newline') {
      const text = insertNewline(event.target as HTMLTextAreaElement);
      if (text !== this.draft$.value) this.draft$.next(text);
      return;
    }
    this.send(view);
  }

  /**
   * Sends the answers to the agent's questions, as a message from the person to the thread's own agent.
   *
   * What was typed in the message box stays there.
   */
  answer(view: WorkspaceView, text: string): void {
    const thread = view.thread;
    if (!thread || view.running) return;
    this.answerThread(thread, text);
  }

  /** Sends answers typed on a card, without leaving Home. */
  answerThread(thread: Thread, text: string): void {
    if (thread.status === 'running' || this.sending$.value) return;
    this.sending$.next(true);
    this.api.sendMessage(thread.id, text, 'author').subscribe({
      next: () => this.sending$.next(false),
      error: (failure: unknown) => {
        this.sending$.next(false);
        this.alerts.error(refusalText(failure, 'The answers could not be sent.'));
      },
    });
  }

  /** Sends the open card's reply and clears it once it has gone. */
  replyTo(thread: Thread, text: string, to: Voice): void {
    if (thread.status === 'running' || this.sending$.value || !text.trim()) return;
    this.sending$.next(true);
    this.api.sendMessage(thread.id, text.trim(), to).subscribe({
      next: () => {
        this.sending$.next(false);
        this.cardClear$.next({ id: thread.id, at: Date.now() });
      },
      error: (failure: unknown) => {
        this.sending$.next(false);
        this.alerts.error(refusalText(failure, 'The message could not be sent.'));
      },
    });
  }

  /** Marks a card read. Opening the card does not, so it stays until the person says so. */
  markThreadSeen(thread: Thread): void {
    this.api.markSeen(thread.id).pipe(catchError(() => EMPTY)).subscribe();
  }

  /** Puts the agent's questions away and goes to the message box, to write something else. */
  putAwayQuestions(asked: Asked): void {
    this.putAway$.next(asked.id);
    this.focusMessageBox();
  }

  /** Puts a card's questions away and goes to that card's reply. */
  putAwayCardQuestions(id: string): void {
    this.putAway$.next(id);
    this.focusCardReply();
  }

  /** Sends what was typed in the open thread's message box to the agent, or to the reviewer when it was picked. */
  send(view: WorkspaceView): void {
    const text = this.draft$.value.trim();
    const thread = view.thread;
    if (!text || !thread || view.running || this.sending$.value) return;

    this.sending$.next(true);
    const to = thread.reviewer ? this.to$.value : 'author';
    this.api.sendMessage(thread.id, text, to).subscribe({
      next: () => {
        this.draft$.next('');
        this.sending$.next(false);
      },
      error: (failure: unknown) => {
        this.sending$.next(false);
        this.alerts.error(refusalText(failure, 'The message could not be sent.'));
      },
    });
  }

  /**
   * Starts a thread from Home with its first message, and stays on Home.
   *
   * The agent and model come from the Home or menu selection when present.
   * Otherwise the workflow's first stage chooses. The new thread is highlighted
   * in In progress once its first message is sent. What was typed stays in the prompt
   * until the message is sent, so a refusal loses nothing.
   */
  startThread(request: StartRequest): void {
    if (this.sending$.value) return;
    this.sending$.next(true);
    const made = this.unsentThread?.projectId === request.projectId ? of(this.unsentThread.thread) : this.api.createThread(request.projectId, request.choice, request.workflow);
    made
      .pipe(
        tap((thread) => (this.unsentThread = { projectId: request.projectId, thread })),
        switchMap((thread) => this.api.sendMessage(thread.id, request.text, 'author').pipe(map(() => thread))),
      )
      .subscribe({
        next: (thread) => {
          this.unsentThread = null;
          this.highlight(thread.id);
          this.draft$.next('');
          this.sending$.next(false);
          this.focusMessageBox();
        },
        error: (failure: unknown) => {
          this.sending$.next(false);
          this.alerts.error(refusalText(failure, 'The thread could not be started.'));
        },
      });
  }

  /** Uses the agent and model chosen on Home or in the command menu for new threads. */
  chooseForNewThreads(choice: Choice): void {
    this.newThreads$.next(choosing(this.newThreads$.value, choice));
  }

  /** Lets the default workflow's first stage choose the agent of new threads again. */
  chooseAutomaticForNewThreads(): void {
    this.newThreads$.next(choosingAutomatic(this.newThreads$.value));
  }

  /** Goes to Home: no project and no thread open, every project's waiting threads and the prompt. */
  openHome(): void {
    this.leader$.next(null);
    this.selection$.next(NOTHING_SELECTED);
    this.draft$.next('');
    this.screen$.next('thread');
  }

  /**
   * Points Home's prompt at a project and shows Home.
   *
   * The project is not opened. Home has no open project; the pick only
   * decides where the next thread starts, and Home remembers it.
   */
  private usePromptProject(project: Project): void {
    this.homeThreadId$.next(null);
    this.promptProject$.next({ ...project });
    this.openHome();
  }

  stop(thread: Thread): void {
    this.api.stop(thread.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The agent could not be stopped.')),
    });
  }

  /**
   * Applies the change directly to the project without opening a description dialog.
   *
   * The entry description comes from the agent's commit. If the change overlaps
   * with newer work, the agent starts catching up and finishes applying once done.
   */
  applyChange(thread: Thread): void {
    this.api.applyChange(thread.id).subscribe({
      next: (resolving) => {
        if (resolving) this.alerts.info(APPLY_RESOLVING_NOTICE);
        else this.alerts.success('Applied to the project.');
      },
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The change could not be applied.')),
    });
  }

  /**
   * Asks for the change's description, then sends it for review on GitHub.
   *
   * The first line becomes the review's title, the rest its text.
   */
  sendForReview(thread: Thread): void {
    this.describe(thread, {
      header: 'Send for review on GitHub',
      message: 'Describe the change. The first line becomes the title of the review.',
      button: 'Send',
      failure: 'The change could not be sent for review.',
      then: (description) =>
        this.api.sendForReview(thread.id, description).subscribe({
          next: () => this.alerts.success('Sent for review on GitHub.'),
          error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The change could not be sent for review.')),
        }),
    });
  }

  openReview(thread: Thread): void {
    this.api.openReview(thread.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The review could not be opened.')),
    });
  }

  /** Asks for a description of the change, starting from what the agent wrote, and hands it on. */
  private describe(
    thread: Thread,
    ask: { header: string; message: string; button: string; failure: string; then: (description: string) => void },
  ): void {
    this.api.draftDescription(thread.id).subscribe({
      next: async (draft) => {
        const alert = await this.alertController.create({
          header: ask.header,
          message: ask.message,
          inputs: [{ name: 'description', type: 'textarea', value: draft, attributes: { 'aria-label': 'Description', rows: 6 } }],
          buttons: [
            { text: 'Cancel', role: 'cancel' },
            { text: ask.button, handler: (values: { description: string }) => ask.then(values.description) },
          ],
        });
        await alert.present();
      },
      error: (failure: unknown) => this.alerts.error(refusalText(failure, ask.failure)),
    });
  }

  /**
   * Brings the change up to date with newer work in the project.
   *
   * When the two overlap nothing changes, and a request for the agent to sort
   * it out waits in the message box for the person to send.
   */
  catchUp(thread: Thread): void {
    this.api.catchUp(thread.id).subscribe({
      next: (overlaps) => {
        if (!overlaps) return this.alerts.success('Caught up with the project.');
        this.alerts.info('This change overlaps with newer work in the project. The message box has a request for the agent.');
        this.draftAndReturn(CATCH_UP_REQUEST);
      },
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The change could not catch up.')),
    });
  }

  /**
   * Asks before putting away the quiet threads in `projectId`, or in every project.
   *
   * The count is taken when the question is asked, not when the command was
   * listed. A thread that stopped being quiet is left by the service, so the
   * number that leave can be smaller than the one confirmed.
   */
  confirmPutAway(projectId: string | null): void {
    const quiet = this.threadsNow.filter((thread) => (projectId === null || thread.project_id === projectId) && isQuiet(thread));
    if (!quiet.length) return;
    const scope = projectId === null ? 'every project' : (this.projectsNow.find((project) => project.id === projectId)?.name ?? '');
    void this.askPutAway(projectId, quiet.length, scope);
  }

  private async askPutAway(projectId: string | null, count: number, scope: string): Promise<void> {
    const alert = await this.alertController.create({
      header: 'Put away quiet threads',
      message: putAwayText(count, scope),
      buttons: [
        { text: 'Cancel', role: 'cancel' },
        { text: 'Put away', handler: () => this.putAwayQuiet(projectId) },
      ],
    });
    await alert.present();
  }

  private putAwayQuiet(projectId: string | null): void {
    this.api.putAwayQuiet(projectId).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The threads could not be put away.')),
    });
  }

  /** Marks the finishes this visit showed, so the next opening of the board leaves them out. */
  acknowledgeDone(seen: readonly SeenFinish[]): void {
    this.api
      .acknowledgeDone(seen.map((finish) => ({ thread_id: finish.id, at: finish.at })))
      .pipe(catchError(() => EMPTY))
      .subscribe();
  }

  async confirmDiscard(thread: Thread): Promise<void> {
    if (!thread.change) return;
    const alert = await this.alertController.create({
      header: 'Discard changes',
      message: discardWarning(thread.change),
      buttons: [
        { text: 'Cancel', role: 'cancel' },
        { text: 'Discard', role: 'destructive', handler: () => this.discard(thread) },
      ],
    });
    await alert.present();
  }

  private discard(thread: Thread): void {
    this.api.discardChange(thread.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The changes could not be discarded.')),
    });
  }

  /** Puts where a thread's copy is on the clipboard, for a terminal or another program. */
  copyLocation(path: string): void {
    navigator.clipboard.writeText(path).then(
      () => this.alerts.success('Location copied.'),
      () => this.alerts.error('The location could not be copied.'),
    );
  }

  openCopy(thread: Thread, opener: Opener): void {
    this.api.openCopy(thread.id, opener).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The copy could not be opened.')),
    });
  }

  /** Adds a project and points Home's prompt at it. A refused one leaves the menu open, to be corrected. */
  addProject({ name, directory }: NewProject): void {
    this.api.createProject(name, directory).subscribe({
      next: (project) => {
        this.usePromptProject(project);
        this.closeMenu();
      },
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The project could not be added.')),
    });
  }

  async confirmRemoveProject(project: Project): Promise<void> {
    const alert = await this.alertController.create({
      header: 'Remove project',
      message: `Remove ${project.name} and its threads from Teryon? The directory itself is left untouched.`,
      buttons: [
        { text: 'Cancel', role: 'cancel' },
        { text: 'Remove', role: 'destructive', handler: () => this.removeProject(project) },
      ],
    });
    await alert.present();
  }

  /**
   * Puts a drafted reply where the person will send it.
   *
   * An open card on Home gets it in its own reply. With no card, it goes to
   * the prompt. A thread open on its own screen gets it there. With `add`,
   * the card keeps what was typed in its reply and the draft follows it.
   */
  private draftAndReturn(draft: string, add = false): void {
    const homeId = this.cardTarget();
    if (homeId) {
      this.cardNote$.next({ threadId: homeId, text: draft, to: this.to$.value, add });
      this.screen$.next('thread');
      this.focusCardReply();
      return;
    }
    this.draft$.next(draft);
    this.screen$.next('thread');
    this.focusMessageBox();
  }

  /** The open card on Home, when no thread is on its own screen. */
  private cardTarget(): string | null {
    return this.selection$.value.threadId ? null : this.homeThreadId$.value;
  }

  /** Asks for the cursor at the end of the message box. */
  private focusMessageBox(): void {
    this.typingWanted$.next();
  }

  /** Asks for the cursor in the open card's reply. */
  private focusCardReply(): void {
    this.preferCardReply = true;
    this.typingWanted$.next();
  }

  /** Points out a thread that just appeared, for a short while. */
  private highlight(id: string): void {
    this.highlightedId$.next(id);
    clearTimeout(this.highlightTimer);
    this.highlightTimer = setTimeout(() => {
      if (this.highlightedId$.value === id) this.highlightedId$.next(null);
    }, 2000);
  }

  /** Keeps Home in the project of the card a plan or a change was opened from. */
  private keepHomeProject(): void {
    const thread = this.threadsNow.find((candidate) => candidate.id === this.homeThreadId$.value);
    if (thread) this.keepProject(thread);
  }

  private keepProject(thread: Thread): void {
    if (this.selection$.value.threadId) return;
    if (this.selection$.value.projectId === thread.project_id) return;
    this.selection$.next({ projectId: thread.project_id, threadId: null });
  }

  private removeProject(project: Project): void {
    this.api.removeProject(project.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The project could not be removed.')),
    });
  }
}
