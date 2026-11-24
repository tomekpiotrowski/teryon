import { TestBed, type ComponentFixture } from '@angular/core/testing';
import { vi } from 'vitest';

import { HomeComponent, type StartRequest } from './home.component';
import { type Choice, groupThreads, type ModelLists, type NewThreads, NO_MODELS, NO_WORKFLOWS, type Project, type Starting, type Thread, type Workflows } from './workspace.model';

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

const TEST_WORKFLOWS: Workflows = {
  ...NO_WORKFLOWS,
  workflows: [
    { name: 'default', description: '', stages: ['talking', 'ready', 'done'] },
    { name: 'small', description: '', stages: ['talking', 'ready', 'done'] },
    { name: 'planned', description: '', stages: ['planning', 'ready', 'done'] },
  ],
};

interface Scene {
  readonly fixture: ComponentFixture<HomeComponent>;
  readonly element: HTMLElement;
  readonly started: StartRequest[];
  readonly opened: Thread[];
  readonly texts: string[];
  readonly chosen: Choice[];
}

interface SceneInputs {
  installed?: string[];
  text?: string;
  workflows?: Workflows;
  starting?: Starting | null;
  models?: ModelLists;
  pickedChoice?: Choice;
  rememberedModels?: NewThreads['models'];
}

/** Home on projects `a` and `b`, with the given threads, and a record of what it emits. */
function scene(threads: Thread[] = [], inputs: SceneInputs = {}): Scene {
  const fixture = TestBed.createComponent(HomeComponent);
  const home = fixture.componentInstance;
  const started: StartRequest[] = [];
  const opened: Thread[] = [];
  const texts: string[] = [];
  const chosen: Choice[] = [];
  home.choiceChosen.subscribe((choice) => chosen.push(choice));
  home.started.subscribe((request) => started.push(request));
  home.opened.subscribe((item) => opened.push(item));
  home.textChanged.subscribe((text) => texts.push(text));
  home.groups = groupThreads([project('a'), project('b')], threads);
  home.installed = inputs.installed ?? ['claude', 'codex', 'grok'];
  home.workflows = inputs.workflows ?? TEST_WORKFLOWS;
  home.starting = inputs.starting === undefined ? { type: 'provider', agent: 'claude', model: null } : inputs.starting;
  home.models = inputs.models ?? NO_MODELS;
  home.pickedChoice = inputs.pickedChoice ?? null;
  home.rememberedModels = inputs.rememberedModels ?? null;
  home.project = project('a');
  if (inputs.text !== undefined) home.text = inputs.text;
  fixture.detectChanges();
  return { fixture, element: fixture.nativeElement as HTMLElement, started, opened, texts, chosen };
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
  const buttons = Array.from(of.element.querySelectorAll('button'));
  const found = buttons.find((candidate) => candidate.textContent?.trim() === text) ??
    buttons.find((candidate) => candidate.textContent?.includes(text));
  if (!found) throw new Error(`no button with ${text}`);
  return found as HTMLButtonElement;
}

function agentChooser(of: Scene): HTMLButtonElement {
  return of.element.querySelector<HTMLButtonElement>('.home-agent')!;
}

function clickButton(of: Scene, label: string): void {
  button(of, label).click();
  of.fixture.detectChanges();
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

    it('starts in the chosen project on the default workflow', () => {
      const home = scene();
      clickButton(home, 'Project a');
      clickButton(home, 'Project b');
      type(home, 'hello');
      button(home, 'Run').click();
      expect(home.started).toEqual([{ projectId: 'b', workflow: 'default', text: 'hello' }]);
    });

    it('sends the workflow it names after a rename too', () => {
      const home = scene([], { workflows: { ...TEST_WORKFLOWS, default: 'main', workflows: [{ ...TEST_WORKFLOWS.workflows[0], name: 'main' }] } });
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started[0].workflow).toBe('main');
    });

    it('shows the default workflow stage model', () => {
      const home = scene([], { starting: { type: 'provider', agent: 'grok', model: 'grok-4.6' } });
      expect(agentChooser(home).textContent?.trim()).toBe('Grok · Grok 4.6');
      expect(home.element.querySelector('[aria-label="Workflow"]')).toBeNull();
    });

    it('closes the model chooser for an outside click or Escape and keeps the choice and draft', async () => {
      const home = scene([], { text: 'keep this', models: { ...NO_MODELS, codex: ['gpt-6-astra'] } });
      const trigger = agentChooser(home);
      const outside = home.element.querySelector<HTMLElement>('.home-nothing')!;
      const openModels = async () => {
        clickButton(home, 'Claude · default');
        clickButton(home, 'Codex');
        await home.fixture.whenStable();
        expect(trigger.getAttribute('aria-expanded')).toBe('true');
      };

      await openModels();
      outside.click();
      home.fixture.detectChanges();
      expect(trigger.getAttribute('aria-expanded')).toBe('false');
      expect(document.activeElement).toBe(trigger);

      await openModels();
      expect(key(home, button(home, 'Default'), 'Escape').defaultPrevented).toBe(true);
      expect(trigger.getAttribute('aria-expanded')).toBe('false');
      expect(document.activeElement).toBe(trigger);
      expect(home.chosen).toEqual([]);
      expect(trigger.textContent?.trim()).toBe('Claude · default');
      expect(prompt(home).value).toBe('keep this');
    });

    it('closes the project chooser on outside click or Escape and keeps the draft', () => {
      const label = 'Project a';
      const home = scene([], { text: 'keep this' });
      document.body.appendChild(home.element);
      const trigger = button(home, label);
      const expanded = () => trigger.getAttribute('aria-expanded');
      const outside = home.element.querySelector<HTMLElement>('.home-nothing')!;
      clickButton(home, label);
      expect(expanded()).toBe('true');
      outside.click();
      home.fixture.detectChanges();
      expect(expanded()).toBe('false');
      clickButton(home, label);
      const option = home.element.querySelector<HTMLElement>('.home-popover button')!;
      option.focus();
      expect(key(home, option, 'Escape').defaultPrevented).toBe(true);
      expect(expanded()).toBe('false');
      expect(document.activeElement).toBe(trigger);
      clickButton(home, label);
      key(home, prompt(home), 'Escape');
      expect(expanded()).toBe('false');
      expect(prompt(home).value).toBe('keep this');
    });

    it('is not sent twice while a request is on its way', () => {
      const home = scene();
      home.fixture.componentInstance.sending = true;
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([]);
    });

    it('remembers the project and ignores an old workflow choice', () => {
      localStorage.setItem('teryon.home', JSON.stringify({ projectId: 'b', workflow: 'small' }));
      const home = scene();
      expect(button(home, 'Project b')).toBeDefined();
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started).toEqual([{ projectId: 'b', workflow: 'default', text: 'hello' }]);
    });

    it('shows text the workspace puts in the prompt', () => {
      const home = scene([], { text: 'About `README.md`: ' });
      expect(prompt(home).value).toBe('About `README.md`: ');
    });
  });

  describe('the agent and model chooser', () => {
    const models = { ...NO_MODELS, codex: ['gpt-6-astra', 'gpt-6-sol'] };
    const goToModels = (home: Scene) => {
      agentChooser(home).click();
      home.fixture.detectChanges();
      clickButton(home, 'Codex');
    };
    const options = (home: Scene, list: string) =>
      Array.from(home.element.querySelectorAll(`[aria-label="${list}"] [role="option"]`));

    it('lists the installed agents first, then names and versions for the chosen agent', () => {
      const home = scene([], { models, rememberedModels: { claude: null, codex: 'gpt-6-sol', grok: null, gemini: null } });
      clickButton(home, 'Claude · default');
      expect(options(home, 'Agent').map((option) => [option.textContent?.trim(), option.getAttribute('aria-selected')])).toEqual([
        ['Claude', 'true'],
        ['Codex', 'false'],
        ['Grok', 'false'],
      ]);
      clickButton(home, 'Codex');
      expect(options(home, 'Agent')).toHaveLength(0);
      expect(options(home, 'Model').map((option) => [option.textContent?.trim(), option.getAttribute('aria-selected'), option.getAttribute('title')])).toEqual([
        ['Default', 'false', null],
        ['GPT 6 astra', 'false', 'gpt-6-astra'],
        ['GPT 6 sol', 'true', 'gpt-6-sol'],
      ]);
      expect(home.chosen).toEqual([]);
      expect(agentChooser(home).textContent?.trim()).toBe('Claude · default');
      clickButton(home, '‹ Agents');
      expect(options(home, 'Agent')).toHaveLength(3);
      expect(options(home, 'Model')).toHaveLength(0);
      expect(home.chosen).toEqual([]);
    });

    it.each([
      ['GPT 6 astra', 'gpt-6-astra', 'GPT 6 astra'],
      ['Default', null, 'default'],
    ] as const)('emits the exact choice for %s, closes the list and returns focus', async (label, model, shown) => {
      const home = scene([], { models });
      goToModels(home);
      await home.fixture.whenStable();
      clickButton(home, label);
      const choice = { agent: 'codex' as const, model };
      expect(home.chosen).toEqual([choice]);
      expect(agentChooser(home).getAttribute('aria-expanded')).toBe('false');
      expect(document.activeElement).toBe(agentChooser(home));

      home.fixture.componentInstance.pickedChoice = choice;
      home.fixture.detectChanges();
      expect(agentChooser(home).textContent?.trim()).toBe(`Codex · ${shown}`);
    });

    it('marks the service model for the current agent while discovery updates the open list', () => {
      const home = scene([], { models, starting: { type: 'provider', agent: 'codex', model: 'gpt-6-other' } });
      expect(agentChooser(home).textContent?.trim()).toBe('Codex · GPT 6 other');
      goToModels(home);
      expect(options(home, 'Model').every((option) => option.getAttribute('aria-selected') === 'false')).toBe(true);

      home.fixture.componentInstance.models = { ...NO_MODELS, codex: ['gpt-6-sol', 'gpt-6-other'] };
      home.fixture.detectChanges();
      expect(options(home, 'Model').map((option) => option.textContent?.trim())).toEqual(['Default', 'GPT 6 sol', 'GPT 6 other']);
      expect(button(home, 'GPT 6 other').getAttribute('aria-selected')).toBe('true');
      expect(home.chosen).toEqual([]);
    });

    it('moves with arrows in both lists and goes back with Left or Backspace', async () => {
      const home = scene([], { models });
      key(home, agentChooser(home), 'ArrowDown');
      await home.fixture.whenStable();
      expect(document.activeElement).toBe(button(home, 'Claude'));
      key(home, button(home, 'Claude'), 'ArrowDown');
      expect(document.activeElement).toBe(button(home, 'Codex'));
      clickButton(home, 'Codex');
      await home.fixture.whenStable();
      expect(document.activeElement).toBe(button(home, 'Default'));
      key(home, button(home, 'Default'), 'ArrowUp');
      expect(document.activeElement).toBe(button(home, '‹ Agents'));
      key(home, button(home, '‹ Agents'), 'ArrowDown');
      key(home, button(home, 'Default'), 'ArrowDown');
      expect(document.activeElement).toBe(button(home, 'GPT 6 astra'));
      expect(key(home, button(home, 'GPT 6 astra'), 'ArrowLeft').defaultPrevented).toBe(true);
      await home.fixture.whenStable();
      expect(document.activeElement).toBe(button(home, 'Claude'));
      clickButton(home, 'Codex');
      expect(key(home, button(home, 'Default'), 'Backspace').defaultPrevented).toBe(true);
      expect(options(home, 'Agent')).toHaveLength(3);
      expect(home.chosen).toEqual([]);
    });

    it('selects an agent with only its default immediately', () => {
      const home = scene([], { installed: ['codex'], starting: null });
      expect(button(home, 'Project a').textContent?.trim()).toBe('Project a');
      expect(agentChooser(home).textContent?.trim()).toBe('Codex · default');
      clickButton(home, 'Codex · default');
      expect(options(home, 'Agent').map((option) => option.textContent?.trim())).toEqual(['Codex']);
      clickButton(home, 'Codex');
      expect(home.chosen).toEqual([{ agent: 'codex', model: null }]);
      expect(agentChooser(home).getAttribute('aria-expanded')).toBe('false');
    });

    it('disables the chooser when no agent is installed', () => {
      const home = scene([], { installed: [], models });
      expect(agentChooser(home).textContent?.trim()).toBe('No agent');
      expect(agentChooser(home).disabled).toBe(true);
    });
  });

  it('starts a new thread with text beginning with @', () => {
    const home = scene([thread({ title: 'Existing thread' })]);
    type(home, '@existing');
    key(home, prompt(home), 'Enter');
    expect(home.started).toEqual([{ projectId: 'a', workflow: 'default', text: '@existing' }]);
    expect(home.element.querySelector('[aria-label="Threads to continue"]')).toBeNull();
    expect(home.element.querySelector('.home-keys')?.textContent).not.toContain('continue a thread');
  });

  describe('the project list', () => {
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
