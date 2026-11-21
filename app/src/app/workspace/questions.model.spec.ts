import type { Question } from '../service/protocol';
import {
  answered,
  answerMessage,
  type Answering,
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

/** A question taking one answer, recommending the given one. */
function one(question: string, labels: readonly string[], recommended = 0): Question {
  return { question, answers: labels.map((label) => ({ label, description: null })), choosing: { kind: 'one', recommended } };
}

/** A question taking several answers, recommending the given ones. */
function several(question: string, labels: readonly string[], recommended: readonly number[] = []): Question {
  return { question, answers: labels.map((label) => ({ label, description: null })), choosing: { kind: 'several', recommended: [...recommended] } };
}

const WHERE = one('Where should answers be kept?', ['In the thread', 'On the record']);
const REVIEWER = one('Should the reviewer ask?', ['Yes', 'No', 'Only when unsure'], 1);
const SCREENS = several('Which screens?', ['Thread list', 'Window title', 'Notification'], [2, 0]);
const EXTRAS = several('Anything else?', ['Sound', 'Badge']);

/** Presses keys in turn, keeping the state; fails on a key the questions don't take. */
function press(state: Answering, ...keys: string[]): Answering {
  return keys.reduce((current, key) => {
    const outcome = pressed(current, key);
    if (!outcome) throw new Error(`the questions did not take ${key}`);
    return outcome.state;
  }, state);
}

const START = startAnswering([WHERE, SCREENS]);

describe('startAnswering', () => {
  it('chooses the recommendations, several in order, on the first question with no own words', () => {
    expect(startAnswering([WHERE, REVIEWER, SCREENS, EXTRAS])).toEqual({
      questions: [WHERE, REVIEWER, SCREENS, EXTRAS],
      current: 0,
      chosen: [[0], [1], [0, 2], []],
      own: [null, null, null, null],
      typing: false,
    });
  });
});

describe('ownNumber', () => {
  it('is one after the last answer on offer', () => {
    expect(ownNumber(WHERE)).toBe(3);
    expect(ownNumber(SCREENS)).toBe(4);
  });
});

describe('pressed', () => {
  it('moves between questions with the arrows and with j and k, stopping at either end', () => {
    expect(press(START, 'ArrowDown').current).toBe(1);
    expect(press(START, 'j').current).toBe(1);
    expect(press(START, 'j', 'j').current).toBe(1);
    expect(press(START, 'j', 'ArrowUp').current).toBe(0);
    expect(press(START, 'j', 'k').current).toBe(0);
    expect(press(START, 'k').current).toBe(0);
  });

  it('replaces the choice of a question taking one answer and moves on', () => {
    const after = press(START, '2');
    expect(after.chosen).toEqual([[1], [0, 2]]);
    expect(after.current).toBe(1);
  });

  it('chooses the same answer again without unchoosing it', () => {
    expect(press(START, '1').chosen[0]).toEqual([0]);
  });

  it('stays on the last question after choosing in it', () => {
    const state = startAnswering([WHERE, REVIEWER]);
    expect(press(state, 'j', '3').current).toBe(1);
    expect(press(state, 'j', '3').chosen[1]).toEqual([2]);
  });

  it('ticks and unticks in a question taking several, staying on it', () => {
    const at = press(START, 'j');
    expect(press(at, '2').chosen[1]).toEqual([0, 1, 2]);
    expect(press(at, '1').chosen[1]).toEqual([2]);
    expect(press(at, '1', '3').chosen[1]).toEqual([]);
    expect(press(at, '1', '3', '1').chosen[1]).toEqual([0]);
    expect(press(at, '2').current).toBe(1);
  });

  it('opens the field for own words with the last number', () => {
    expect(press(START, '3').typing).toBe(true);
    expect(press(START, '3').current).toBe(0);
    expect(press(START, 'j', '4').typing).toBe(true);
  });

  it('drops own words in a question taking several with the last number, instead of opening the field', () => {
    const withWords = typed(press(START, 'j', '4'), 'Tray icon');
    const dropped = press(withWords, '4');
    expect(dropped.own[1]).toBeNull();
    expect(dropped.typing).toBe(false);
    expect(dropped.chosen[1]).toEqual([0, 2]);
  });

  it('opens the field again in a question taking one answer that has own words, to change them', () => {
    const withWords = typed(press(START, '3'), 'Somewhere else');
    expect(press(withWords, 'k', '3').typing).toBe(true);
  });

  it('leaves alone numbers past the own words, zero, letters and other keys', () => {
    expect(pressed(START, '4')).toBeNull();
    expect(pressed(START, '0')).toBeNull();
    expect(pressed(START, 'a')).toBeNull();
    expect(pressed(START, '/')).toBeNull();
    expect(pressed(START, 'Tab')).toBeNull();
  });

  it('leaves every key to the field while it is open', () => {
    const typing = press(START, '3');
    expect(pressed(typing, 'Enter')).toBeNull();
    expect(pressed(typing, 'Escape')).toBeNull();
    expect(pressed(typing, '1')).toBeNull();
    expect(pressed(typing, 'j')).toBeNull();
  });

  it('sends on Enter when every question has an answer, including straight away with the recommendations', () => {
    expect(pressed(START, 'Enter')).toEqual({ state: START, effect: 'send' });
  });

  it('sends with nothing ticked in a question taking several', () => {
    const state = startAnswering([EXTRAS]);
    expect(pressed(state, 'Enter')?.effect).toBe('send');
  });

  it('moves to the first question without an answer on Enter instead of sending', () => {
    const state: Answering = { ...startAnswering([WHERE, SCREENS, REVIEWER]), chosen: [[0], [], []], current: 0 };
    expect(pressed(state, 'Enter')).toEqual({ state: { ...state, current: 2 }, effect: 'none' });
  });

  it('leaves on Escape', () => {
    expect(pressed(START, 'Escape')).toEqual({ state: START, effect: 'leave' });
  });
});

describe('picked', () => {
  it('chooses an answer of any question by a click, going to that question and closing the field', () => {
    const typing = press(START, '3');
    const after = picked(typing, 1, 1);
    expect(after.chosen[1]).toEqual([0, 1, 2]);
    expect(after.current).toBe(1);
    expect(after.typing).toBe(false);
  });

  it('drops own words in a question taking one answer when an answer on offer is chosen', () => {
    const withWords = typed(press(START, '3'), 'Somewhere else');
    const after = picked(withWords, 0, 1);
    expect(after.own[0]).toBeNull();
    expect(after.chosen[0]).toEqual([1]);
  });

  it('keeps own words in a question taking several when an answer is ticked', () => {
    const withWords = typed(press(START, 'j', '4'), 'Tray icon');
    expect(picked(withWords, 1, 1).own[1]).toBe('Tray icon');
  });
});

describe('ownChosen', () => {
  it('opens the field for a clicked question', () => {
    expect(ownChosen(START, 1)).toEqual({ ...START, current: 1, typing: true });
  });
});

describe('typed', () => {
  it('replaces the chosen answer of a question taking one, and moves on', () => {
    const after = typed(press(START, '3'), '  Somewhere   else ');
    expect(after.own[0]).toBe('Somewhere   else');
    expect(after.chosen[0]).toEqual([]);
    expect(after.typing).toBe(false);
    expect(after.current).toBe(1);
  });

  it('adds to the ticked answers of a question taking several', () => {
    const after = typed(press(START, 'j', '4'), 'Tray icon');
    expect(after.own[1]).toBe('Tray icon');
    expect(after.chosen[1]).toEqual([0, 2]);
    expect(after.current).toBe(1);
  });

  it('changes nothing but closing the field when nothing was written', () => {
    const typing = press(START, '3');
    expect(typed(typing, '  ')).toEqual({ ...typing, typing: false });
  });
});

describe('stopTyping', () => {
  it('closes the field and keeps what was chosen', () => {
    const typing = press(START, '3');
    expect(stopTyping(typing)).toEqual({ ...typing, typing: false });
  });
});

describe('answered and complete', () => {
  it('counts a question taking one answer as answered by a choice or by own words', () => {
    const none: Answering = { ...START, chosen: [[], [0]] };
    expect(answered(none, 0)).toBe(false);
    expect(complete(none)).toBe(false);
    expect(answered({ ...none, own: ['Mine', null] }, 0)).toBe(true);
    expect(complete({ ...none, own: ['Mine', null] })).toBe(true);
  });

  it('counts a question taking several as always answered', () => {
    expect(answered({ ...START, chosen: [[0], []] }, 1)).toBe(true);
  });
});

describe('isChosen and isRecommended', () => {
  it('tells the chosen answers', () => {
    expect([0, 1, 2].map((answer) => isChosen(START, 1, answer))).toEqual([true, false, true]);
  });

  it('tells the recommended answers of both kinds of question', () => {
    expect([0, 1, 2].map((answer) => isRecommended(REVIEWER, answer))).toEqual([false, true, false]);
    expect([0, 1, 2].map((answer) => isRecommended(SCREENS, answer))).toEqual([true, false, true]);
    expect([0, 1].map((answer) => isRecommended(EXTRAS, answer))).toEqual([false, false]);
  });
});

describe('answerMessage', () => {
  it('repeats each question with its answer, numbered', () => {
    expect(answerMessage(START)).toBe(['Answers to your questions:', '1. Where should answers be kept? → In the thread', '2. Which screens? → Thread list, Notification'].join('\n'));
  });

  it('gives own words for a question taking one answer', () => {
    expect(answerMessage(typed(press(startAnswering([WHERE]), '3'), 'Somewhere else'))).toBe(
      'Answers to your questions:\n1. Where should answers be kept? → Somewhere else',
    );
  });

  it('puts own words after the ticked answers, and alone when nothing is ticked', () => {
    const withWords = typed(press(startAnswering([SCREENS]), '4'), 'Tray icon');
    expect(answerMessage(withWords)).toBe('Answers to your questions:\n1. Which screens? → Thread list, Notification, Tray icon');
    expect(answerMessage(press(withWords, '1', '3'))).toBe('Answers to your questions:\n1. Which screens? → Tray icon');
  });

  it('says none of these when nothing is ticked and no own words were given', () => {
    expect(answerMessage(startAnswering([EXTRAS]))).toBe('Answers to your questions:\n1. Anything else? → none of these');
  });

  it('puts questions and answers written over several lines on one line', () => {
    const question = one('Where should\nanswers   go?', ['Here', 'There']);
    expect(answerMessage(typed(press(startAnswering([question]), '3'), 'Over\nthere'))).toBe('Answers to your questions:\n1. Where should answers go? → Over there');
  });

  it('answers four questions of both kinds in order', () => {
    const state = press(startAnswering([WHERE, REVIEWER, SCREENS, EXTRAS]), '2', '3', '2', 'j', '2');
    expect(answerMessage(state)).toBe(
      [
        'Answers to your questions:',
        '1. Where should answers be kept? → On the record',
        '2. Should the reviewer ask? → Only when unsure',
        '3. Which screens? → Thread list, Window title, Notification',
        '4. Anything else? → Badge',
      ].join('\n'),
    );
  });
});
