/**
 * Answering the questions a thread's agent asked: what is chosen, what the keys do, and the message that carries the answers.
 *
 * The agent offers two to four answers per question, its recommendation
 * chosen to start with, so agreeing with everything takes one key. A
 * question takes one answer or several; either way the person may answer
 * in their own words instead. Kept free of Angular so every key can be
 * tested on its own.
 */

import type { Question } from '../service/protocol';

/** Questions the agent asked in one call, as kept in the thread. */
export interface Asked {
  /** The entry that holds them, which tells one call from another. */
  readonly id: string;
  /** The turn they were asked in: a newer call in the same turn replaces an older one. */
  readonly turn: string;
  readonly questions: readonly Question[];
}

/** Where the person stands in answering. */
export interface Answering {
  readonly questions: readonly Question[];
  /** The question the keys act on. */
  readonly current: number;
  /** For each question, the positions of the answers on offer that are chosen, in order. */
  readonly chosen: readonly (readonly number[])[];
  /** For each question, the person's own words, or nothing when they gave none. */
  readonly own: readonly (string | null)[];
  /** Whether the field for the current question's own words is open. */
  readonly typing: boolean;
}

/** What a key did besides changing what is chosen. */
export type Effect = 'none' | 'send' | 'leave';

export interface Outcome {
  readonly state: Answering;
  readonly effect: Effect;
}

/** Answering from the start: the agent's recommendations chosen, on the first question. */
export function startAnswering(questions: readonly Question[]): Answering {
  return {
    questions,
    current: 0,
    chosen: questions.map((question) =>
      question.choosing.kind === 'one' ? [question.choosing.recommended] : [...question.choosing.recommended].sort((a, b) => a - b),
    ),
    own: questions.map(() => null),
    typing: false,
  };
}

/** The number of the row for the person's own words in a question: one after the last answer on offer. */
export function ownNumber(question: Question): number {
  return question.answers.length + 1;
}

/**
 * What a key does while no field is open.
 *
 * ↑ and ↓, or `k` and `j`, move between questions. A number chooses an
 * answer of the current question: in a question taking one answer it
 * replaces the choice and moves on, in one taking several it ticks or
 * unticks and stays. The last number opens the field for the person's own
 * words, or drops them in a question taking several. Enter sends once
 * every question has an answer, and otherwise moves to the first without
 * one. Escape leaves. Nothing means the key is not for the questions.
 */
export function pressed(state: Answering, key: string): Outcome | null {
  if (state.typing) return null;
  const question = state.questions[state.current];
  switch (key) {
    case 'ArrowDown':
    case 'j':
      return moved(state, state.current + 1);
    case 'ArrowUp':
    case 'k':
      return moved(state, state.current - 1);
    case 'Enter': {
      if (complete(state)) return { state, effect: 'send' };
      return moved(state, state.questions.findIndex((_, index) => !answered(state, index)));
    }
    case 'Escape':
      return { state, effect: 'leave' };
  }
  if (!/^[1-9]$/.test(key)) return null;
  const number = Number(key);
  if (number === ownNumber(question)) return { state: ownChosen(state, state.current), effect: 'none' };
  if (number > question.answers.length) return null;
  return { state: picked(state, state.current, number - 1), effect: 'none' };
}

/**
 * Chooses an answer on offer, as its number does: replacing the choice and moving on, or ticking and staying.
 *
 * Also what a click on the answer does, on whichever question it is in.
 */
export function picked(state: Answering, question: number, answer: number): Answering {
  const at = { ...state, current: question, typing: false };
  if (state.questions[question].choosing.kind === 'one') {
    return next({ ...at, chosen: replaced(state.chosen, question, [answer]), own: replaced(state.own, question, null) });
  }
  const before = state.chosen[question];
  const after = before.includes(answer) ? before.filter((chosen) => chosen !== answer) : [...before, answer].sort((a, b) => a - b);
  return { ...at, chosen: replaced(state.chosen, question, after) };
}

/**
 * What choosing the row for the person's own words does, by its number or a click.
 *
 * It opens the field, except in a question taking several answers that has
 * own words already, where it drops them, as unticking does.
 */
export function ownChosen(state: Answering, question: number): Answering {
  const several = state.questions[question].choosing.kind === 'several';
  if (several && state.own[question] !== null) return { ...state, current: question, own: replaced(state.own, question, null) };
  return { ...state, current: question, typing: true };
}

/**
 * The person's own words for the current question, confirmed in the field.
 *
 * In a question taking one answer they replace the chosen answer; in one
 * taking several they are one more answer. Then the next question comes.
 * Nothing written closes the field and changes nothing.
 */
export function typed(state: Answering, text: string): Answering {
  const words = text.trim();
  if (!words) return { ...state, typing: false };
  const question = state.current;
  const one = state.questions[question].choosing.kind === 'one';
  return next({
    ...state,
    typing: false,
    own: replaced(state.own, question, words),
    chosen: one ? replaced(state.chosen, question, []) : state.chosen,
  });
}

/** Closes the field for own words without taking them, as Escape in it does. */
export function stopTyping(state: Answering): Answering {
  return { ...state, typing: false };
}

/** Whether a question has an answer. One taking several always has: ticking none says none fit. */
export function answered(state: Answering, question: number): boolean {
  if (state.questions[question].choosing.kind === 'several') return true;
  return state.chosen[question].length > 0 || state.own[question] !== null;
}

/** Whether every question has an answer, so the answers can be sent. */
export function complete(state: Answering): boolean {
  return state.questions.every((_, index) => answered(state, index));
}

/** Whether an answer on offer is chosen. */
export function isChosen(state: Answering, question: number, answer: number): boolean {
  return state.chosen[question].includes(answer);
}

/** Whether an answer on offer is one the agent recommended. */
export function isRecommended(question: Question, answer: number): boolean {
  return question.choosing.kind === 'one' ? question.choosing.recommended === answer : question.choosing.recommended.includes(answer);
}

/**
 * The message that carries the answers to the agent, one line per question.
 *
 * For example:
 *
 *     Answers to your questions:
 *     1. Where should answers be kept? → In the thread
 *     2. Which screens? → Thread list, my own words
 *
 * The question is repeated so the agent doesn't have to match numbers to
 * what it asked, and kept on one line. Several answers are separated by
 * commas, own words last; none ticked says so.
 */
export function answerMessage(state: Answering): string {
  const lines = state.questions.map((question, index) => {
    const answers = [...state.chosen[index].map((answer) => question.answers[answer].label), ...(state.own[index] === null ? [] : [state.own[index]])];
    return `${index + 1}. ${oneLine(question.question)} → ${answers.length ? answers.map(oneLine).join(', ') : 'none of these'}`;
  });
  return ['Answers to your questions:', ...lines].join('\n');
}

function oneLine(text: string): string {
  return text.split(/\s+/).filter(Boolean).join(' ');
}

/** The next question, or the same one when it is the last. */
function next(state: Answering): Answering {
  return { ...state, current: Math.min(state.current + 1, state.questions.length - 1) };
}

function moved(state: Answering, to: number): Outcome {
  const current = Math.max(0, Math.min(to, state.questions.length - 1));
  return { state: { ...state, current }, effect: 'none' };
}

function replaced<T>(items: readonly T[], index: number, item: T): readonly T[] {
  return items.map((existing, position) => (position === index ? item : existing));
}
