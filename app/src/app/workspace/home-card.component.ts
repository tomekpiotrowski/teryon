import { ChangeDetectionStrategy, Component, ElementRef, EventEmitter, inject, Input, OnDestroy, Output, ViewChild } from '@angular/core';
import { BehaviorSubject, combineLatest, distinctUntilChanged, map, type Observable, of, shareReplay, Subscription, switchMap } from 'rxjs';
import type { Command } from './commands.model';
import { keyHint } from './commands.model';
import type { HomeCard } from './home.model';
import { splitLatestTurn } from './home.model';
import { isRecommended, type Asked } from './questions.model';
import { QuestionsComponent } from './questions.component';
import {
  agentLabel,
  changeCard,
  choiceLabel,
  citationRoots,
  composerAction,
  type EntryLine,
  insertNewline,
  NO_MODELS,
  NO_WORKFLOWS,
  pendingQuestions,
  reviewerLabel,
  reviewerOf,
  stageText,
  threadChoice,
  threadLabel,
  threadLines,
  type Voice,
  type ModelLists,
  type Workflows,
} from './workspace.model';
import { WorkspaceStore } from './workspace.store';

/** What an open card shows at one moment. */
interface CardView {
  readonly card: HomeCard;
  readonly expanded: boolean;
  readonly title: string;
  readonly who: string;
  readonly stage: string;
  readonly earlier: readonly EntryLine[];
  readonly latest: readonly EntryLine[];
  readonly showEarlier: boolean;
  readonly asked: Asked | null;
  readonly running: boolean;
  readonly reviewing: boolean;
  readonly reviewer: string;
  readonly canMarkRead: boolean;
  readonly actions: readonly { command: Command; hint: string | null }[];
  readonly labelled: boolean;
  readonly changeTitle: string;
  readonly changeNote: string;
  readonly tone: string;
  readonly plan: boolean;
  readonly hasChange: boolean;
  readonly showStrip: boolean;
  readonly to: Voice;
  readonly draft: string;
  readonly roots: readonly string[];
}

/**
 * One thread on Home, collapsed to a line or open in place.
 *
 * Opening a card is how a person reads and answers without leaving Home, so
 * the card follows the thread's conversation only while it is open. The
 * latest turn is shown first; earlier messages stay one toggle away.
 */
@Component({
  selector: 'app-home-card',
  templateUrl: './home-card.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class HomeCardComponent implements OnDestroy {
  private readonly store = inject(WorkspaceStore);

  @Output() readonly toggled = new EventEmitter<void>();
  @Output() readonly opened = new EventEmitter<void>();
  @Output() readonly markedRead = new EventEmitter<void>();
  @Output() readonly replied = new EventEmitter<{ text: string; to: Voice }>();
  @Output() readonly answered = new EventEmitter<string>();
  @Output() readonly putAway = new EventEmitter<string>();
  @Output() readonly restored = new EventEmitter<void>();
  @Output() readonly stopped = new EventEmitter<void>();
  @Output() readonly planOpened = new EventEmitter<void>();
  @Output() readonly commandRun = new EventEmitter<Command>();
  @Output() readonly moreOpened = new EventEmitter<void>();
  @Output() readonly draftChanged = new EventEmitter<string>();
  @Output() readonly toChanged = new EventEmitter<Voice>();
  @Output() readonly linesChange = new EventEmitter<readonly EntryLine[]>();

  private readonly card$ = new BehaviorSubject<HomeCard | null>(null);
  private readonly expanded$ = new BehaviorSubject(false);
  private readonly fresh$ = new BehaviorSubject(false);
  private readonly draft$ = new BehaviorSubject('');
  private readonly to$ = new BehaviorSubject<Voice>('author');
  private readonly commands$ = new BehaviorSubject<readonly Command[]>([]);
  private readonly putAwayId$ = new BehaviorSubject<string | null>(null);
  private readonly workflows$ = new BehaviorSubject<Workflows>(NO_WORKFLOWS);
  private readonly models$ = new BehaviorSubject<ModelLists>(NO_MODELS);
  private readonly showEarlier$ = new BehaviorSubject(false);
  private shownRoots: readonly string[] = [];

  /** The thread this card shows, for Home to find it among the others. */
  get threadId(): string | null {
    return this.card$.value?.thread.id ?? null;
  }

  readonly threadLabel = threadLabel;
  readonly agentLabel = agentLabel;
  readonly reviewerLabel = reviewerLabel;
  readonly reviewerOf = reviewerOf;
  readonly isRecommended = isRecommended;

  @ViewChild('reply') private reply?: ElementRef<HTMLTextAreaElement>;
  @ViewChild(QuestionsComponent) private questions?: QuestionsComponent;

  @Input({ required: true }) set card(card: HomeCard) {
    const previous = this.card$.value;
    if (previous && previous.thread.id !== card.thread.id) this.showEarlier$.next(false);
    this.card$.next(card);
  }

  @Input() set expanded(expanded: boolean) {
    if (!expanded) this.showEarlier$.next(false);
    this.expanded$.next(expanded);
  }

  @Input() set fresh(fresh: boolean) {
    this.fresh$.next(fresh);
  }

  @Input() set draft(draft: string | null | undefined) {
    const text = draft ?? '';
    if (text !== this.draft$.value) this.draft$.next(text);
  }

  @Input() set to(to: Voice | null | undefined) {
    const voice = to === 'reviewer' ? 'reviewer' : 'author';
    if (voice !== this.to$.value) this.to$.next(voice);
  }

  @Input() set commands(commands: readonly Command[]) {
    this.commands$.next(commands);
  }

  @Input() set putAwayId(id: string | null) {
    this.putAwayId$.next(id);
  }

  @Input() set workflows(workflows: Workflows) {
    this.workflows$.next(workflows);
  }

  @Input() set models(models: ModelLists) {
    this.models$.next(models);
  }

  private readonly lines$: Observable<readonly EntryLine[]> = combineLatest([
    this.card$.pipe(distinctUntilChanged((before, after) => before?.thread.id === after?.thread.id)),
    this.expanded$.pipe(distinctUntilChanged()),
  ]).pipe(
    switchMap(([card, expanded]) => (card && expanded ? this.store.entries$(card.thread.id).pipe(map((entries) => threadLines(entries))) : of<EntryLine[]>([]))),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  private readonly publishing: Subscription = this.lines$.subscribe((lines) => this.linesChange.emit(lines));

  readonly freshClass$ = this.fresh$.pipe(distinctUntilChanged());

  readonly view$: Observable<CardView | null> = combineLatest([
    this.card$,
    this.expanded$,
    this.lines$,
    this.showEarlier$,
    this.draft$,
    this.to$,
    this.commands$,
    this.putAwayId$,
    this.workflows$,
    this.models$,
  ]).pipe(
    map(([card, expanded, lines, showEarlier, draft, to, commands, putAwayId, workflows, models]) => {
      if (!card) return null;
      const thread = card.thread;
      const split = splitLatestTurn(lines);
      const pending = pendingQuestions(lines, thread.status === 'running');
      const asked = pending && pending.id !== putAwayId ? pending : null;
      const change = changeCard(thread, card.project);
      const actions = (change?.next ?? [])
        .map((id) => commands.find((command) => command.id === id))
        .filter((command): command is Command => command !== undefined)
        .map((command) => ({ command, hint: keyHint(command) }));
      const choice = threadChoice(thread, 'author');
      return {
        card,
        expanded,
        title: threadLabel(thread),
        who: choice ? choiceLabel(choice, models) : agentLabel(thread.agent),
        stage: stageText(workflows, thread),
        earlier: split.earlier,
        latest: split.latest,
        showEarlier,
        asked,
        running: thread.status === 'running',
        reviewing: thread.reviewer?.reviewing === true,
        reviewer: agentLabel(reviewerOf(thread)),
        canMarkRead: thread.unread && thread.asking !== true,
        actions,
        labelled: change?.labelled === true,
        changeTitle: change?.title ?? '',
        changeNote: change?.note ?? '',
        tone: change?.tone ?? '',
        plan: thread.plan != null,
        hasChange: thread.change != null,
        showStrip: change !== null || thread.plan != null,
        to: thread.reviewer ? to : 'author',
        draft,
        roots: this.rootsFor(card.project, thread),
      };
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  ngOnDestroy(): void {
    this.publishing.unsubscribe();
  }

  private rootsFor(project: HomeCard['project'], thread: HomeCard['thread']): readonly string[] {
    const next = citationRoots(project, thread);
    if (next.length === this.shownRoots.length && next.every((root, index) => root === this.shownRoots[index])) return this.shownRoots;
    this.shownRoots = next;
    return next;
  }

  toggleEarlier(show: boolean): void {
    this.showEarlier$.next(show);
  }

  updateDraft(event: Event): void {
    const text = (event.target as HTMLTextAreaElement).value;
    if (text === this.draft$.value) return;
    this.draft$.next(text);
    this.draftChanged.emit(text);
  }

  /** Sends the reply on Enter, starts a new line on Shift+Enter or Control+Enter, and leaves the box on Escape. */
  handleReplyKey(event: KeyboardEvent): void {
    const action = composerAction(event);
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    if (action === 'leave') {
      (event.target as HTMLElement).blur();
      return;
    }
    if (action === 'newline') {
      const text = insertNewline(event.target as HTMLTextAreaElement);
      if (text !== this.draft$.value) {
        this.draft$.next(text);
        this.draftChanged.emit(text);
      }
      return;
    }
    this.send();
  }

  send(): void {
    const text = (this.reply?.nativeElement.value ?? this.draft$.value).trim();
    const card = this.card$.value;
    if (!text || !card || card.thread.status === 'running') return;
    const to = card.thread.reviewer ? this.to$.value : 'author';
    this.replied.emit({ text, to });
  }

  switchTo(): void {
    this.toChanged.emit(this.to$.value === 'author' ? 'reviewer' : 'author');
  }

  /** Puts the cursor in the reply, once the box is on screen. */
  focusReply(): void {
    setTimeout(() => {
      const box = this.reply?.nativeElement;
      box?.focus();
      box?.setSelectionRange(box.value.length, box.value.length);
    });
  }

  /**
   * Puts the keys on the questions.
   *
   * Questions the person had put away come back first, since `q` is how they
   * are reached again, the same as on the thread screen.
   */
  focusQuestions(): void {
    const card = this.card$.value;
    if (!card) return;
    if (card.thread.asking === true && !this.questions) this.restored.emit();
    setTimeout(() => this.questions?.focus());
  }
}
