import { TestBed, type ComponentFixture } from '@angular/core/testing';
import { vi } from 'vitest';

import { HomeComponent, type ContinueRequest, type StartRequest } from './home.component';
import { type AgentName, groupThreads, NO_WORKFLOWS, type Project, type Thread, type Workflows } from './workspace.model';

const project = (id: string): Project => ({ id, name: `Project ${id}`, directory: `/work/${id}`, created_at: '2026-11-01T10:00:00' });

function thread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't',
    project_id: 'a',
    title: 'A thread',
    agent: 'claude',
    status: 'idle',
    unread: false,
    commands: [],
    change: null,
    finished: null,
    reviewer: null,
    created_at: '2026-11-03T10:00:00',
    ...overrides,
  };
}

const TWO_WORKFLOWS: Workflows = {
  ...NO_WORKFLOWS,
  workflows: [
    { name: 'default', description: '', stages: ['talking', 'ready', 'done'] },
    { name: 'small', description: '', stages: ['talking', 'ready', 'done'] },
  ],
};

interface Scene {
  readonly fixture: ComponentFixture<HomeComponent>;
  readonly element: HTMLElement;
  readonly started: StartRequest[];
  readonly continued: ContinueRequest[];
  readonly opened: Thread[];
  readonly texts: string[];
  readonly pressed: string[];
}

/** Home on projects `a` and `b`, with the given threads, and a record of what it emits. */
function scene(threads: Thread[] = [], inputs: { installed?: string[]; text?: string; workflows?: Workflows; picked?: AgentName | null } = {}): Scene {
  const fixture = TestBed.createComponent(HomeComponent);
  const home = fixture.componentInstance;
  const started: StartRequest[] = [];
  const continued: ContinueRequest[] = [];
  const opened: Thread[] = [];
  const texts: string[] = [];
  const pressed: string[] = [];
  home.agentPressed.subscribe((agent) => pressed.push(agent));
  home.started.subscribe((request) => started.push(request));
  home.continued.subscribe((request) => continued.push(request));
  home.opened.subscribe((item) => opened.push(item));
  home.textChanged.subscribe((text) => texts.push(text));
  home.groups = groupThreads([project('a'), project('b')], threads);
  home.installed = inputs.installed ?? ['claude', 'codex', 'grok'];
  home.workflows = inputs.workflows ?? TWO_WORKFLOWS;
  home.pickedAgent = inputs.picked ?? null;
  home.starting = { type: 'provider', agent: 'claude', model: null };
  home.project = project('a');
  if (inputs.text !== undefined) home.text = inputs.text;
  fixture.detectChanges();
  return { fixture, element: fixture.nativeElement as HTMLElement, started, continued, opened, texts, pressed };
}

function prompt(of: Scene): HTMLInputElement {
  return of.element.querySelector('input.home-input') as HTMLInputElement;
}

function type(of: Scene, text: string): void {
  const box = prompt(of);
  box.value = text;
  box.dispatchEvent(new Event('input'));
  of.fixture.detectChanges();
}

function key(of: Scene, target: Element, name: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  of.fixture.detectChanges();
  return event;
}

function button(of: Scene, text: string): HTMLButtonElement {
  const found = Array.from(of.element.querySelectorAll('button')).find((candidate) => candidate.textContent?.includes(text));
  if (!found) throw new Error(`no button with ${text}`);
  return found as HTMLButtonElement;
}

function rows(of: Scene): HTMLElement[] {
  return Array.from(of.element.querySelectorAll<HTMLElement>('.home-row'));
}

/** A browser's storage that lives as long as the test, since the test environment has none. */
function stubStorage(): void {
  const kept = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => kept.get(key) ?? null,
    setItem: (key: string, value: string) => void kept.set(key, value),
    removeItem: (key: string) => void kept.delete(key),
    clear: () => kept.clear(),
  });
}

describe('HomeComponent', () => {
  beforeEach(() => {
    stubStorage();
    TestBed.configureTestingModule({ declarations: [HomeComponent] });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  describe('the list', () => {
    it('says nothing needs the person when nothing does', () => {
      const home = scene([thread({ id: 'x' })]);
      expect(home.element.textContent).toContain('Nothing needs you.');
      expect(rows(home)).toHaveLength(0);
    });

    it('lists a thread with its word, and opens it on a click', () => {
      const waiting = thread({ id: 'w', title: 'Fix it', ended: 'failed' });
      const home = scene([waiting]);
      expect(rows(home)).toHaveLength(1);
      expect(rows(home)[0].textContent).toContain('Fix it');
      expect(rows(home)[0].querySelector('.home-word')?.textContent?.trim()).toBe('failed');
      rows(home)[0].click();
      expect(home.opened).toEqual([waiting]);
    });

    it('shows the project name only when threads come from several projects', () => {
      const one = scene([thread({ id: '1', asking: true })]);
      expect(rows(one)[0].querySelectorAll('.home-dim')).toHaveLength(0);
      TestBed.resetTestingModule();
      TestBed.configureTestingModule({ declarations: [HomeComponent] });
      const several = scene([thread({ id: '1', asking: true }), thread({ id: '2', asking: true, project_id: 'b' })]);
      expect(rows(several).map((row) => row.querySelectorAll('.home-dim').length)).toEqual([1, 1]);
    });

    it('cuts the list at seven and points to the board', () => {
      const many = Array.from({ length: 9 }, (_, index) => thread({ id: `t${index}`, asking: true }));
      const home = scene(many);
      expect(rows(home)).toHaveLength(7);
      expect(home.element.querySelector('.home-more')?.textContent?.trim()).toBe('+2 more');
      let board = 0;
      home.fixture.componentInstance.boardOpened.subscribe(() => board++);
      (home.element.querySelector('.home-more') as HTMLElement).click();
      expect(board).toBe(1);
    });

    it('names what runs under the list', () => {
      const home = scene([thread({ id: 'r', title: 'Busy', status: 'running' })]);
      const line = home.element.querySelector('.home-running');
      expect(line?.textContent).toContain('Busy');
      expect(line?.textContent).toContain('Project a');
      expect(line?.textContent).toContain('working');
    });
  });

  describe('the keys of the list', () => {
    const waiting = [thread({ id: 'a', title: 'First', asking: true, created_at: '2026-11-05T10:00:00' }), thread({ id: 'b', title: 'Second', asking: true, created_at: '2026-11-04T10:00:00' })];

    it('hands the keys to the list on Tab, marking the first row', () => {
      const home = scene(waiting);
      const event = key(home, prompt(home), 'Tab');
      expect(event.defaultPrevented).toBe(true);
      expect(rows(home)[0].classList).toContain('home-marked');
    });

    it('leaves Tab alone when there is no list, and Shift+Tab always', () => {
      const empty = scene([]);
      expect(key(empty, prompt(empty), 'Tab').defaultPrevented).toBe(false);
      TestBed.resetTestingModule();
      TestBed.configureTestingModule({ declarations: [HomeComponent] });
      const home = scene(waiting);
      expect(key(home, prompt(home), 'Tab', { shiftKey: true }).defaultPrevented).toBe(false);
    });

    it('moves with j and k, not past either end, and opens with Enter', () => {
      const home = scene(waiting);
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      key(home, list, 'j');
      expect(rows(home)[1].classList).toContain('home-marked');
      key(home, list, 'j');
      expect(rows(home)[1].classList).toContain('home-marked');
      key(home, list, 'k');
      key(home, list, 'k');
      expect(rows(home)[0].classList).toContain('home-marked');
      key(home, list, 'j');
      key(home, list, 'Enter');
      expect(home.opened.map((item) => item.id)).toEqual(['b']);
    });

    it('moves and opens even when keys come before the screen is drawn', () => {
      const home = scene(waiting);
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      list.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', bubbles: true, cancelable: true }));
      list.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
      expect(home.opened.map((item) => item.id)).toEqual(['b']);
    });

    it('goes back to the prompt on Escape', () => {
      const home = scene(waiting);
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      key(home, list, 'Escape');
      expect(rows(home).some((row) => row.classList.contains('home-marked'))).toBe(false);
    });
  });

  describe('starting a thread', () => {
    it('does nothing with nothing typed, or only spaces', () => {
      const home = scene();
      key(home, prompt(home), 'Enter');
      type(home, '   ');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([]);
      expect(button(home, 'Run').disabled).toBe(true);
    });

    it('asks to start in the open project on the default workflow', () => {
      const home = scene();
      type(home, ' Fix the parser ');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([{ projectId: 'a', workflow: 'default', text: 'Fix the parser' }]);
    });

    it('keeps the text, so a refused request loses nothing', () => {
      const home = scene();
      type(home, 'Fix the parser');
      key(home, prompt(home), 'Enter');
      expect(prompt(home).value).toBe('Fix the parser');
      expect(home.texts.at(-1)).toBe('Fix the parser');
      key(home, prompt(home), 'Enter');
      expect(home.started).toHaveLength(2);
    });

    it('asks for the project and workflow chosen', () => {
      const home = scene();
      button(home, 'default').click();
      home.fixture.detectChanges();
      (Array.from(home.element.querySelectorAll('.home-popover button')).find((item) => item.textContent?.trim() === 'small') as HTMLElement).click();
      home.fixture.detectChanges();
      button(home, 'Project a').click();
      home.fixture.detectChanges();
      (Array.from(home.element.querySelectorAll('.home-popover button')).find((item) => item.textContent?.trim() === 'Project b') as HTMLElement).click();
      home.fixture.detectChanges();
      type(home, 'hello');
      button(home, 'Run').click();
      expect(home.started).toEqual([{ projectId: 'b', workflow: 'small', text: 'hello' }]);
    });

    it('sends the workflow shown, not one that was removed since it was chosen', () => {
      const home = scene();
      button(home, 'default').click();
      home.fixture.detectChanges();
      (Array.from(home.element.querySelectorAll('.home-popover button')).find((item) => item.textContent?.trim() === 'small') as HTMLElement).click();
      home.fixture.detectChanges();
      home.fixture.componentInstance.workflows = { ...TWO_WORKFLOWS, workflows: TWO_WORKFLOWS.workflows.filter((workflow) => workflow.name !== 'small') };
      home.fixture.detectChanges();
      expect(button(home, 'default')).toBeDefined();
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([{ projectId: 'a', workflow: 'default', text: 'hello' }]);
    });

    it('sends the workflow it names after a rename too', () => {
      const home = scene([], { workflows: { ...TWO_WORKFLOWS, default: 'main', workflows: [{ ...TWO_WORKFLOWS.workflows[0], name: 'main' }] } });
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started[0].workflow).toBe('main');
    });

    it('shows the agent the first stage chooses as pressed, and the picked one in its place', () => {
      const staged = scene();
      expect(button(staged, 'Claude').getAttribute('aria-pressed')).toBe('true');
      TestBed.resetTestingModule();
      TestBed.configureTestingModule({ declarations: [HomeComponent] });
      const picked = scene([], { picked: 'codex' });
      expect(button(picked, 'Claude').getAttribute('aria-pressed')).toBe('false');
      expect(button(picked, 'Codex').getAttribute('aria-pressed')).toBe('true');
    });

    it('shows no agent pressed for a workflow whose first stage is not the one shown', () => {
      const home = scene();
      button(home, 'default').click();
      home.fixture.detectChanges();
      (Array.from(home.element.querySelectorAll('.home-popover button')).find((item) => item.textContent?.trim() === 'small') as HTMLElement).click();
      home.fixture.detectChanges();
      expect(Array.from(home.element.querySelectorAll('.home-agent')).map((agent) => agent.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'false']);
    });

    it('keeps a picked agent pressed on any workflow', () => {
      const home = scene([], { picked: 'grok' });
      button(home, 'default').click();
      home.fixture.detectChanges();
      (Array.from(home.element.querySelectorAll('.home-popover button')).find((item) => item.textContent?.trim() === 'small') as HTMLElement).click();
      home.fixture.detectChanges();
      expect(button(home, 'Grok').getAttribute('aria-pressed')).toBe('true');
    });

    it('tells the workspace which agent was pressed, and holds no pick of its own', () => {
      const home = scene();
      button(home, 'Codex').click();
      home.fixture.detectChanges();
      expect(home.pressed).toEqual(['codex']);
      expect(button(home, 'Codex').getAttribute('aria-pressed')).toBe('false');
      home.fixture.componentInstance.pickedAgent = 'codex';
      home.fixture.detectChanges();
      expect(button(home, 'Codex').getAttribute('aria-pressed')).toBe('true');
      home.fixture.componentInstance.pickedAgent = 'grok';
      home.fixture.detectChanges();
      expect(button(home, 'Grok').getAttribute('aria-pressed')).toBe('true');
      expect(button(home, 'Codex').getAttribute('aria-pressed')).toBe('false');
    });

    it('offers only the agents installed', () => {
      const home = scene([], { installed: ['codex'] });
      const names = Array.from(home.element.querySelectorAll('.home-agent')).map((agent) => agent.textContent?.replace(/\s+/g, ' ').trim());
      expect(names).toEqual(['CCodex']);
    });

    it('is not sent twice while a request is on its way', () => {
      const home = scene();
      home.fixture.componentInstance.sending = true;
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([]);
    });

    it('remembers the project and workflow for the next visit', () => {
      const home = scene();
      button(home, 'default').click();
      home.fixture.detectChanges();
      (Array.from(home.element.querySelectorAll('.home-popover button')).find((item) => item.textContent?.trim() === 'small') as HTMLElement).click();
      home.fixture.detectChanges();
      TestBed.resetTestingModule();
      TestBed.configureTestingModule({ declarations: [HomeComponent] });
      const next = scene();
      expect(button(next, 'small')).toBeDefined();
    });

    it('shows text the workspace puts in the prompt', () => {
      const home = scene([], { text: 'About `README.md`: ' });
      expect(prompt(home).value).toBe('About `README.md`: ');
    });
  });

  describe('continuing a thread with @', () => {
    const threads = [
      thread({ id: 'p', title: 'Fix the parser', created_at: '2026-11-01T10:00:00' }),
      thread({ id: 's', title: 'Add sensor node', created_at: '2026-11-02T10:00:00' }),
    ];

    it('searches threads after an @, and says when none matches', () => {
      const home = scene(threads);
      type(home, '@sen');
      expect(Array.from(home.element.querySelectorAll('.home-match .home-title')).map((title) => title.textContent)).toEqual(['Add sensor node']);
      type(home, '@zzz');
      expect(home.element.textContent).toContain('No thread matches.');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([]);
      expect(home.continued).toEqual([]);
    });

    it('moves through the matches with the arrows, going round', () => {
      const home = scene(threads);
      type(home, '@');
      const marked = () => Array.from(home.element.querySelectorAll('.home-match')).findIndex((match) => match.classList.contains('home-marked'));
      expect(marked()).toBe(0);
      key(home, prompt(home), 'ArrowDown');
      expect(marked()).toBe(1);
      key(home, prompt(home), 'ArrowDown');
      expect(marked()).toBe(0);
      key(home, prompt(home), 'ArrowUp');
      expect(marked()).toBe(1);
    });

    it('chooses a match with Enter, then sends the rest to that thread', () => {
      const home = scene(threads);
      type(home, '@pars');
      key(home, prompt(home), 'Enter');
      expect(home.element.querySelector('.home-chip')?.textContent).toContain('Fix the parser');
      expect(prompt(home).value).toBe('');
      type(home, 'try again');
      key(home, prompt(home), 'Enter');
      expect(home.continued).toEqual([{ thread: threads[0], text: 'try again' }]);
      expect(home.started).toEqual([]);
    });

    it('keeps the chosen thread and the text, so a refused request loses nothing', () => {
      const home = scene(threads);
      type(home, '@pars');
      key(home, prompt(home), 'Enter');
      type(home, 'try again');
      key(home, prompt(home), 'Enter');
      expect(home.element.querySelector('.home-chip')?.textContent).toContain('Fix the parser');
      expect(prompt(home).value).toBe('try again');
      key(home, prompt(home), 'Enter');
      expect(home.continued).toHaveLength(2);
    });

    it('chooses a match with a click', () => {
      const home = scene(threads);
      type(home, '@');
      (home.element.querySelectorAll('.home-match')[1] as HTMLElement).click();
      home.fixture.detectChanges();
      expect(home.element.querySelector('.home-chip')?.textContent).toContain('Fix the parser');
    });

    it('drops the chip with Backspace on an empty prompt, but not with text', () => {
      const home = scene(threads);
      type(home, '@pars');
      key(home, prompt(home), 'Enter');
      type(home, 'x');
      key(home, prompt(home), 'Backspace');
      expect(home.element.querySelector('.home-chip')).not.toBeNull();
      type(home, '');
      key(home, prompt(home), 'Backspace');
      expect(home.element.querySelector('.home-chip')).toBeNull();
    });

    it('drops the search on Escape, and the chip too', () => {
      const home = scene(threads);
      type(home, '@pars');
      key(home, prompt(home), 'Escape');
      expect(home.texts.at(-1)).toBe('');
      expect(home.element.querySelector('.home-matches')).toBeNull();
    });

    it('takes an @ pressed outside the prompt as the start of a search', () => {
      const home = scene(threads);
      document.body.appendChild(home.element);
      const event = new KeyboardEvent('keydown', { key: '@', bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      home.fixture.detectChanges();
      expect(event.defaultPrevented).toBe(true);
      expect(home.texts.at(-1)).toBe('@');
      home.element.remove();
    });

    it('leaves an @ typed in a text field alone', () => {
      const home = scene(threads);
      document.body.appendChild(home.element);
      const event = new KeyboardEvent('keydown', { key: '@', bubbles: true, cancelable: true });
      prompt(home).dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
      home.element.remove();
    });
  });

  describe('the project and workflow lists', () => {
    it('close on Escape without leaving the prompt\'s text', () => {
      const home = scene();
      type(home, 'keep me');
      button(home, 'Project a').click();
      home.fixture.detectChanges();
      expect(home.element.querySelector('.home-popover')).not.toBeNull();
      key(home, prompt(home), 'Escape');
      expect(home.element.querySelector('.home-popover')).toBeNull();
      expect(prompt(home).value).toBe('keep me');
    });
  });
});
