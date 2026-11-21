import { AfterViewInit, ChangeDetectionStrategy, Component, ElementRef, EventEmitter, inject, Input, OnDestroy, Output } from '@angular/core';
import { BehaviorSubject, distinctUntilChanged, ReplaySubject, type Subscription } from 'rxjs';
import type { Question } from '../service/protocol';
import {
  answerMessage,
  type Answering,
  type Asked,
  complete,
  isChosen,
  isRecommended,
  ownChosen,
  ownNumber,
  picked,
  pressed,
  startAnswering,
  stopTyping,
  typed,
} from './questions.model';

/**
 * The questions the thread's agent asked, above the message box, answered from the keyboard.
 *
 * The agent's recommendations start chosen, so Enter alone agrees with all
 * of them. Number keys choose, the arrows or `j` and `k` move between
 * questions, and the last number of each question takes the person's own
 * words. Escape puts the questions away so a normal message can be written;
 * they stay in the thread and come back from the menu.
 *
 * The keys it uses go no further, so the screen's own keys don't run too;
 * `/`, Ctrl+K and the letters still reach the screen.
 */
@Component({
  selector: 'app-questions',
  templateUrl: './questions.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class QuestionsComponent implements AfterViewInit, OnDestroy {
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);

  /** The questions, as asked. New ones start the answering afresh; the same ones again keep what was chosen. */
  @Input({ required: true }) set asked(asked: Asked) {
    this.asked$.next(asked);
  }

  /** The message carrying the answers, once the person sends them. */
  @Output() readonly answered = new EventEmitter<string>();
  /** The person put the questions away, to write a message instead. */
  @Output() readonly putAway = new EventEmitter<void>();

  private readonly asked$ = new ReplaySubject<Asked>(1);
  readonly state$ = new BehaviorSubject<Answering | null>(null);

  private readonly starting: Subscription = this.asked$
    .pipe(distinctUntilChanged((before, after) => before.id === after.id))
    .subscribe((asked) => this.state$.next(startAnswering(asked.questions)));

  readonly isChosen = isChosen;
  readonly isRecommended = isRecommended;
  readonly ownNumber = ownNumber;
  readonly complete = complete;

  /**
   * Takes the keyboard on arrival, unless the person is busy elsewhere.
   *
   * Questions arrive when an agent finishes, which may be in the middle of
   * anything. Taking the keyboard then would send Enter, meant for the menu
   * or a message, to the questions instead, answering them unasked. So they
   * take it only when nothing else holds it: no dialog is open, and no text
   * is being written. The empty message box counts as free, since that is
   * where the keyboard rests after sending.
   */
  ngAfterViewInit(): void {
    if (keyboardFree()) this.focusList();
  }

  ngOnDestroy(): void {
    this.starting.unsubscribe();
  }

  /** Answers a key pressed on the questions, outside the field for own words. */
  handleKey(event: KeyboardEvent): void {
    const state = this.state$.value;
    if (!state || event.target instanceof HTMLInputElement || event.ctrlKey || event.metaKey || event.altKey) return;
    const outcome = pressed(state, event.key);
    if (!outcome) return;
    event.preventDefault();
    event.stopPropagation();
    this.state$.next(outcome.state);
    if (outcome.effect === 'send') this.answered.emit(answerMessage(outcome.state));
    if (outcome.effect === 'leave') this.putAway.emit();
    if (outcome.state.typing) this.focusField();
  }

  /** Confirms own words on Enter, and closes the field on Escape, keeping the keys from the screen. */
  handleFieldKey(event: KeyboardEvent): void {
    const state = this.state$.value;
    if (!state || event.isComposing || (event.key !== 'Enter' && event.key !== 'Escape')) return;
    event.preventDefault();
    event.stopPropagation();
    const field = event.target as HTMLInputElement;
    this.state$.next(event.key === 'Enter' ? typed(state, field.value) : stopTyping(state));
    this.focusList();
  }

  /** Chooses an answer on offer by a click. */
  pick(question: number, answer: number): void {
    const state = this.state$.value;
    if (!state) return;
    this.state$.next(picked(state, question, answer));
    this.focusList();
  }

  /** Opens the field for own words by a click, or drops them in a question taking several. */
  pickOwn(question: number): void {
    const state = this.state$.value;
    if (!state) return;
    const after = ownChosen(state, question);
    this.state$.next(after);
    if (after.typing) this.focusField();
    else this.focusList();
  }

  /**
   * Sends the answers by the button, once every question has one.
   *
   * Own words still in their field are taken first, as Enter in it would,
   * so a click doesn't send the answer they were meant to replace.
   */
  send(): void {
    let state = this.state$.value;
    if (!state) return;
    if (state.typing) {
      const field = this.host.nativeElement.querySelector<HTMLInputElement>('.own-field');
      state = field ? typed(state, field.value) : stopTyping(state);
      this.state$.next(state);
    }
    if (complete(state)) this.answered.emit(answerMessage(state));
  }

  /** What marks an answer on offer as chosen: `▸` in a question taking one, `[x]` or `[ ]` in one taking several. */
  mark(question: Question, chosen: boolean): string {
    if (question.choosing.kind === 'several') return chosen ? '[x]' : '[ ]';
    return chosen ? '▸' : '';
  }

  private focusList(): void {
    requestAnimationFrame(() => this.host.nativeElement.querySelector<HTMLElement>('.questions')?.focus());
  }

  private focusField(): void {
    requestAnimationFrame(() => {
      const field = this.host.nativeElement.querySelector<HTMLInputElement>('.own-field');
      field?.focus();
      field?.select();
    });
  }
}

/** Whether the keyboard is free to take: no dialog open, and no text being written anywhere. */
function keyboardFree(): boolean {
  if (document.querySelector('[aria-modal="true"]')) return false;
  const active = document.activeElement;
  if (active instanceof HTMLTextAreaElement) return active.value.trim() === '';
  return !(active instanceof HTMLInputElement || (active instanceof HTMLElement && active.isContentEditable));
}
