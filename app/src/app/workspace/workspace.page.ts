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
import { type Command, type CommandGroup, commandList, keyHint, keyOutcome, menuAsked } from './commands.model';
import { askDraft } from './files.model';
import { type Asked, isRecommended } from './questions.model';
import { type Commit, revertDraft } from './history.model';
import { WorkspaceApi } from './workspace.api';
import {
  agentLabel,
  barTitle,
  type Choice,
  choiceLabel,
  chosen,
  choosing,
  FIRST_NEW_THREADS,
  type NewThreads,
  switchingAgent,
  threadChoice,
  CATCH_UP_REQUEST,
  type ChangeCard,
  changeCard,
  codePlace,
  copyPlace,
  discardWarning,
  composerAction,
  type EntryLine,
  pendingQuestions,
  threadLines,
  groupThreads,
  NO_PLACE,
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
  runningText,
  type Screen,
  screenAfter,
  type Selection,
  seenNow,
  startingChoice,
  newThreadLabel,
  firstStageChooses,
  stageLabel,
  stageOf,
  type Thread,
  threadLabel,
  type Voice,
  waitingText,
  findingsDraft,
  reviewerLabel,
  reviewerOf,
} from './workspace.model';
import { WorkspaceStore } from './workspace.store';

/** Everything the template needs about the workspace at one moment. */
/** The keys the board answers itself. */
const BOARD_KEYS = ['h', 'j', 'k', 'l', 'a', 'Enter'];

interface WorkspaceView {
  readonly groups: readonly ProjectGroup[];
  readonly project: Project | null;
  readonly thread: Thread | null;
  readonly lines: readonly EntryLine[];
  readonly running: boolean;
  /** How many agents are running in the selected project. */
  readonly liveAgents: number;
  /** The selected project's threads, for the history to name where a commit came from. */
  readonly projectThreads: readonly Thread[];
  /** The part of the selected project's repository the files and the history show. */
  readonly place: Place;
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
  readonly choiceLabel = choiceLabel;
  readonly threadChoice = threadChoice;
  readonly barTitle = barTitle;
  readonly runningText = runningText;
  readonly waitingText = waitingText;
  readonly connectionText = connectionText;
  readonly offersRestart = offersRestart;

  readonly selection$ = new BehaviorSubject<Selection>(NOTHING_SELECTED);
  readonly draft$ = new BehaviorSubject('');
  /** The agent and model new threads start with, and the model remembered for each agent. */
  readonly newThreads$ = new BehaviorSubject<NewThreads>(FIRST_NEW_THREADS);
  /** The agent and model a new thread would start with, for the button beside the message box. */
  readonly choice$: Observable<Choice> = this.newThreads$.pipe(map(chosen));
  /** What the button beside the message box says about new threads: the first stage's choice until the person picks. */
  readonly newThreadLabel$: Observable<string> = combineLatest([this.newThreads$, this.store.workflows$]).pipe(
    map(([newThreads, workflows]) => newThreadLabel(newThreads, workflows)),
  );
  readonly menuOpen$ = new BehaviorSubject(false);
  /** The one section the menu was opened on, such as the thread's changes. Nothing for the whole menu. */
  readonly menuSection$ = new BehaviorSubject<CommandGroup | null>(null);
  /** Which of the open thread's agents the message box writes to. */
  readonly to$ = new BehaviorSubject<Voice>('author');
  /** True while a request is on its way, so a second press does not send it twice. */
  readonly sending$ = new BehaviorSubject(false);
  /** Whether the selected project's threads, files or history fill the screen. */
  readonly screen$ = new BehaviorSubject<Screen>('thread');
  /** Whether the board shows every project's threads rather than the open project's. */
  readonly boardEvery$ = new BehaviorSubject(false);
  /** The branch or commit the files and the history were last asked to show. */
  readonly place$ = new BehaviorSubject<Place>(NO_PLACE);
  /** A leader key pressed, waiting for the key that picks a command of its family. */
  readonly leader$ = new BehaviorSubject<string | null>(null);

  readonly reviewerOf = reviewerOf;
  readonly reviewerLabel = reviewerLabel;
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
          projectThreads,
          place: project ? placeIn(place, project.id) : NO_PLACE,
          change: thread ? changeCard(thread, project) : null,
          stage: thread ? stageLabel(workflows, stageOf(thread)) : '',
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
    this.store.models$,
    this.target$,
    this.store.workflows$,
    combineLatest([this.pending$, this.putAway$]).pipe(map(([pending, putAway]) => pending !== null && pending.id === putAway)),
  ]).pipe(
    map(([view, newThreads, theme, opens, models, to, workflows, questionsPutAway]) =>
      commandList({ ...view, newThreads, theme, opens, models, to, workflows, questionsPutAway }),
    ),
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
        const box = this.host.nativeElement.querySelector('textarea');
        box?.focus();
        box?.setSelectionRange(box.value.length, box.value.length);
      }),
    );

  /** Whether the menu was opened from the keyboard, so that closing it goes back to typing. */
  private menuOpenedByKey = false;

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

  constructor() {
    this.store.start();
  }

  ngOnDestroy(): void {
    this.following.unsubscribe();
    this.offering.unsubscribe();
    this.typing.unsubscribe();
    this.seeing.unsubscribe();
  }

  /**
   * Answers the keys that work anywhere on the screen.
   *
   * `/` and Ctrl+K open the menu, and Ctrl+K or Escape close it. A bare
   * letter runs the command bound to it, as the hints on screen say, or
   * waits for a second one when it leads a family, such as `w` for the
   * thread's change. While the menu is open the letters are left alone.
   * With the menu closed, Escape returns from any other screen to the thread,
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
    };

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
    // The board moves between cards with these, see `BoardComponent`; `h` would otherwise open the history.
    if (this.screen$.value === 'board' && BOARD_KEYS.includes(event.key)) return;
    if (menuAsked(press, this.draft$.value)) {
      event.preventDefault();
      return this.openMenu(true);
    }
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
    this.menuSection$.next(section);
    this.menuOpenedByKey = byKey;
    this.menuOpen$.next(true);
  }

  /** Closes the menu. Someone who opened it from the keyboard goes back to the message box. */
  closeMenu(): void {
    this.menuOpen$.next(false);
    if (this.menuOpenedByKey) this.focusMessageBox();
    this.menuOpenedByKey = false;
  }

  /** Carries out a command chosen in the menu or asked for with its key. */
  run(command: Command): void {
    const action = command.action;
    this.closeMenu();
    switch (action.kind) {
      case 'openThread':
        return this.openThread(action.thread);
      case 'openProject':
        this.selectProject(action.project);
        return this.screen$.next('thread');
      case 'showBoard':
        return this.showBoard(action.everyProject);
      case 'chooseWorkflow':
        return this.chooseWorkflow(action.thread, action.workflow);
      case 'showScreen':
        return action.screen === 'files' ? this.showCode() : this.screen$.next(screenAfter(this.screen$.value, action.screen));
      case 'removeProject':
        return void this.confirmRemoveProject(action.project);
      case 'useChoice':
        return this.newThreads$.next(choosing(this.newThreads$.value, action.choice));
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
        this.screen$.next('thread');
        return this.putAway$.next(null);
      case 'newProject':
        // The menu asks for the name and the directory itself, then calls `addProject`.
        return;
    }
  }

  /** Goes back to the thread with the file named in the message box, ready to be asked about. */
  askAbout(path: string): void {
    this.draftAndReturn(askDraft(this.draft$.value, path));
  }

  /**
   * Goes back to the thread with a request to revert a commit in the message box.
   *
   * Teryon never changes a repository itself. Reverting is work for an
   * agent, and the person still decides whether to send the request.
   */
  askToRevert(commit: Commit): void {
    this.draftAndReturn(revertDraft(this.draft$.value, commit));
  }

  /** Shows a branch of a project in the files and the history. Nothing means the directory as it is. */
  chooseBranch(project: Project, branch: string | null): void {
    this.place$.next({ projectId: project.id, threadId: null, branch, commit: null });
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
      this.selected$.pipe(take(1)).subscribe(({ project, thread }) => {
        if (project) this.place$.next(codePlace(this.place$.value, project.id, thread));
      });
    }
    this.screen$.next(screenAfter(this.screen$.value, 'files'));
  }

  /** Opens the files on what a thread's agent changed. */
  exploreChange(thread: Thread): void {
    const change = thread.change;
    if (!change) return;
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

  /** Switches a thread's agent, or its reviewer, to another agent or model, from the next message on. */
  /** Opens the open thread's plan. */
  showPlan(): void {
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

  changeAgent(thread: Thread, to: Voice, choice: Choice): void {
    this.api.changeAgent(thread.id, to, choice).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The agent could not be changed.')),
    });
  }

  /** Puts what the reviewer found in the message box, addressed to the author, for the person to trim and send. */
  passFindings(): void {
    this.view$.pipe(take(1)).subscribe((view) => {
      this.to$.next('author');
      this.draftAndReturn(findingsDraft(view.lines));
    });
  }

  /** Moves new threads on to the next agent, on the model last chosen for it, for the button beside the message box. */
  switchAgent(): void {
    this.store.workflows$.pipe(take(1)).subscribe((workflows) =>
      this.newThreads$.next(switchingAgent(this.newThreads$.value, firstStageChooses(workflows))),
    );
  }

  updateDraft(event: Event): void {
    this.draft$.next((event.target as HTMLTextAreaElement).value);
  }

  /** Sends the message on Enter and leaves the message box on Escape, so that the keys bound to letters work. */
  handleComposerKeydown(event: KeyboardEvent, view: WorkspaceView): void {
    const action = composerAction(event);
    if (!action) return;
    event.preventDefault();
    if (action === 'leave') return (event.target as HTMLElement).blur();
    this.send(view);
  }

  /**
   * Sends the answers to the agent's questions, as a message from the person to the thread's own agent.
   *
   * What was typed in the message box stays there.
   */
  answer(view: WorkspaceView, text: string): void {
    const thread = view.thread;
    if (!thread || view.running || this.sending$.value) return;
    this.sending$.next(true);
    this.api.sendMessage(thread.id, text, 'author').subscribe({
      next: () => this.sending$.next(false),
      error: (failure: unknown) => {
        this.sending$.next(false);
        this.alerts.error(refusalText(failure, 'The answers could not be sent.'));
      },
    });
  }

  /** Puts the agent's questions away and goes to the message box, to write something else. */
  putAwayQuestions(asked: Asked): void {
    this.putAway$.next(asked.id);
    this.focusMessageBox();
  }

  /** Sends what was typed: to the open thread, or as the first message of a new one. */
  send(view: WorkspaceView): void {
    const text = this.draft$.value.trim();
    if (!text || !view.project || view.running || this.sending$.value) return;

    const thread$ = view.thread
      ? of(view.thread)
      : this.api.createThread(view.project.id, startingChoice(this.newThreads$.value)).pipe(
          tap((thread) => this.selection$.next({ projectId: thread.project_id, threadId: thread.id })),
        );

    this.sending$.next(true);
    const to = view.thread?.reviewer ? this.to$.value : 'author';
    thread$.pipe(switchMap((thread) => this.api.sendMessage(thread.id, text, to))).subscribe({
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

  stop(thread: Thread): void {
    this.api.stop(thread.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The agent could not be stopped.')),
    });
  }

  /**
   * Asks for the change's description, starting from what the agent wrote, then applies the change.
   *
   * The description becomes the one entry the change adds to the project's
   * history, so the person gets the last word on it.
   */
  applyChange(thread: Thread): void {
    this.describe(thread, {
      header: 'Apply to the project',
      message: 'Describe the change. It becomes one entry in the project’s history.',
      button: 'Apply',
      failure: 'The change could not be applied.',
      then: (description) => this.sendApply(thread, description),
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

  private sendApply(thread: Thread, description: string): void {
    this.api.applyChange(thread.id, description).subscribe({
      next: () => this.alerts.success('Applied to the project.'),
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The change could not be applied.')),
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

  /** Adds a project and opens it. A refused one leaves the menu open, to be corrected. */
  addProject({ name, directory }: NewProject): void {
    this.api.createProject(name, directory).subscribe({
      next: (project) => {
        this.selectProject(project);
        this.screen$.next('thread');
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

  /** Goes back to the thread with a text in the message box, ready to be finished and sent. */
  private draftAndReturn(draft: string): void {
    this.draft$.next(draft);
    this.screen$.next('thread');
    this.focusMessageBox();
  }

  /** Asks for the cursor at the end of the message box. */
  private focusMessageBox(): void {
    this.typingWanted$.next();
  }

  private removeProject(project: Project): void {
    this.api.removeProject(project.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The project could not be removed.')),
    });
  }
}
