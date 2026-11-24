import { CommonModule } from '@angular/common';
import { AgentChooserComponent } from './agent-chooser.component';
import { HomeCardComponent } from './home-card.component';
import { modelOptionForTest as model } from './model.testing';
import { MarkdownPipe } from './markdown.pipe';
import { QuestionsComponent } from './questions.component';
import { TestBed, type ComponentFixture } from '@angular/core/testing';
import { of } from 'rxjs';
import { vi } from 'vitest';

import { HomeComponent, type StartRequest } from './home.component';
import { WorkspaceStore } from './workspace.store';
import { type Choice, groupThreads, type ModelLists, NO_MODELS, NO_WORKFLOWS, type Project, type Starting, type Thread, type Workflows } from './workspace.model';

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

/** The test workflows with the first stage choosing the agent of new threads. */
const RULED_WORKFLOWS: Workflows = {
  ...TEST_WORKFLOWS,
  labels: TEST_WORKFLOWS.labels.map((label) => (label.stage === 'talking' ? { ...label, chooses: 'Any · medium' } : label)),
};

interface Scene {
  readonly fixture: ComponentFixture<HomeComponent>;
  readonly element: HTMLElement;
  readonly started: StartRequest[];
  readonly opened: Thread[];
  readonly texts: string[];
  readonly chosen: Choice[];
  /** How many times Automatic was chosen. */
  readonly automatic: () => number;
}

interface SceneInputs {
  installed?: string[];
  text?: string;
  workflows?: Workflows;
  starting?: Starting | null;
  models?: ModelLists;
  pickedChoice?: Choice;
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
  let automatic = 0;
  home.automaticChosen.subscribe(() => automatic++);
  home.started.subscribe((request) => started.push(request));
  home.opened.subscribe((item) => opened.push(item));
  home.textChanged.subscribe((text) => texts.push(text));
  home.groups = groupThreads([project('a'), project('b')], threads);
  home.installed = inputs.installed ?? ['claude', 'codex', 'grok'];
  home.workflows = inputs.workflows ?? TEST_WORKFLOWS;
  home.starting = inputs.starting === undefined ? { type: 'provider', agent: 'claude', model: null, effort: null } : inputs.starting;
  home.models = inputs.models ?? NO_MODELS;
  home.pickedChoice = inputs.pickedChoice ?? null;
  home.project = project('a');
  if (inputs.text !== undefined) home.text = inputs.text;
  fixture.detectChanges();
  return { fixture, element: fixture.nativeElement as HTMLElement, started, opened, texts, chosen, automatic: () => automatic };
}

function prompt(of: Scene): HTMLTextAreaElement {
  return of.element.querySelector('textarea.home-input') as HTMLTextAreaElement;
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
  return of.element.querySelector<HTMLButtonElement>('.agent-trigger')!;
}

function clickButton(of: Scene, label: string): void {
  button(of, label).click();
  of.fixture.detectChanges();
}

function rows(of: Scene): HTMLElement[] {
  return Array.from(of.element.querySelectorAll<HTMLElement>('.home-card'));
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

function configure(): void {
  TestBed.configureTestingModule({
    imports: [CommonModule],
    declarations: [HomeComponent, HomeCardComponent, AgentChooserComponent, QuestionsComponent, MarkdownPipe],
    providers: [{ provide: WorkspaceStore, useValue: { entries$: () => of([]) } }],
  });
}

describe('HomeComponent', () => {
  beforeEach(() => {
    stubStorage();
    configure();
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

    it('lists a thread in Decide, and opens the card on a click', () => {
      const waiting = thread({ id: 'w', title: 'Fix it', ended: 'failed' });
      const home = scene([waiting]);
      expect(home.element.querySelector('[data-section="decide"]')).toBeTruthy();
      expect(rows(home)).toHaveLength(1);
      expect(rows(home)[0].textContent).toContain('Fix it');
      expect(rows(home)[0].textContent).toContain('The run failed');
      rows(home)[0].querySelector('button')?.click();
      home.fixture.detectChanges();
      expect(home.opened).toEqual([]);
      expect(rows(home)[0].classList).toContain('home-card-open');
    });

    it('shows the project name only when threads come from several projects', () => {
      const one = scene([thread({ id: '1', asking: true })]);
      expect(rows(one)[0].textContent).toContain('Claude');
      expect(rows(one)[0].textContent).not.toContain('Project a');
      TestBed.resetTestingModule();
      configure();
      const several = scene([thread({ id: '1', asking: true }), thread({ id: '2', asking: true, project_id: 'b' })]);
      expect(rows(several).map((row) => row.textContent)).toEqual([expect.stringContaining('Project a'), expect.stringContaining('Project b')]);
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

    it('names what runs under In progress, including its project', () => {
      const home = scene([thread({ id: 'r', title: 'Busy', status: 'running' })]);
      const line = home.element.querySelector('[data-section="progress"]');
      expect(line?.textContent).toContain('Busy');
      expect(line?.textContent).toContain('Project a');
      expect(line?.textContent).toContain('working');
      expect(home.element.textContent).toContain('Nothing needs you.');
      expect(home.element.querySelector('.home')?.classList.contains('home-empty')).toBe(false);
    });
  });

  describe('the keys of the list', () => {
    const waiting = [thread({ id: 'a', title: 'First', asking: true, created_at: '2026-11-05T10:00:00' }), thread({ id: 'b', title: 'Second', asking: true, created_at: '2026-11-04T10:00:00' })];

    it('hands the keys to the list on Tab, opening the first card', () => {
      const home = scene(waiting);
      const event = key(home, prompt(home), 'Tab');
      expect(event.defaultPrevented).toBe(true);
      expect(rows(home)[0].classList).toContain('home-card-open');
    });

    it('leaves Tab alone when there is no list, and Shift+Tab always', () => {
      const empty = scene([]);
      expect(key(empty, prompt(empty), 'Tab').defaultPrevented).toBe(false);
      TestBed.resetTestingModule();
      configure();
      const home = scene(waiting);
      expect(key(home, prompt(home), 'Tab', { shiftKey: true }).defaultPrevented).toBe(false);
    });

    it('moves with j and k, not past either end, and Enter stays on Home', () => {
      const home = scene(waiting);
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      key(home, list, 'j');
      expect(rows(home)[1].classList).toContain('home-card-open');
      key(home, list, 'j');
      expect(rows(home)[1].classList).toContain('home-card-open');
      key(home, list, 'k');
      key(home, list, 'k');
      expect(rows(home)[0].classList).toContain('home-card-open');
      key(home, list, 'j');
      key(home, list, 'Enter');
      expect(home.opened).toEqual([]);
      expect(rows(home)[1].classList).toContain('home-card-open');
    });

    it('moves even when keys come before the screen is drawn', () => {
      const home = scene(waiting);
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      list.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', bubbles: true, cancelable: true }));
      list.dispatchEvent(new KeyboardEvent('keydown', { key: 'o', bubbles: true, cancelable: true }));
      expect(home.opened.map((item) => item.id)).toEqual(['b']);
    });

    it('closes the card and goes back to the prompt on Escape', () => {
      const home = scene(waiting);
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      key(home, list, 'Escape');
      expect(rows(home).some((row) => row.classList.contains('home-card-open'))).toBe(false);
    });

    it('marks a read card read with m, and opens it with o', () => {
      const fresh = thread({ id: 'n', title: 'Done', unread: true, stage: 'implementing' });
      const home = scene([fresh]);
      const marked: Thread[] = [];
      home.fixture.componentInstance.markedRead.subscribe((item) => marked.push(item));
      key(home, prompt(home), 'Tab');
      const list = home.element.querySelector('[data-home-list]') as HTMLElement;
      key(home, list, 'm');
      expect(marked.map((item) => item.id)).toEqual(['n']);
      key(home, list, 'o');
      expect(home.opened.map((item) => item.id)).toEqual(['n']);
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
      expect(home.started).toEqual([{ projectId: 'a', workflow: 'default', text: 'Fix the parser', choice: { agent: 'claude', model: null, effort: null } }]);
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
      expect(home.started).toEqual([{ projectId: 'b', workflow: 'default', text: 'hello', choice: { agent: 'claude', model: null, effort: null } }]);
    });

    it('sends the workflow it names after a rename too', () => {
      const home = scene([], { workflows: { ...TEST_WORKFLOWS, default: 'main', workflows: [{ ...TEST_WORKFLOWS.workflows[0], name: 'main' }] } });
      type(home, 'hello');
      key(home, prompt(home), 'Enter');
      expect(home.started[0].workflow).toBe('main');
    });

    it('shows the default workflow stage model', () => {
      const home = scene([], { workflows: RULED_WORKFLOWS, starting: { type: 'provider', agent: 'grok', model: 'grok-4.6', effort: null } });
      expect(agentChooser(home).textContent?.trim()).toBe('Auto · Grok · Grok 4.6');
      expect(home.element.querySelector('[aria-label="Workflow"]')).toBeNull();
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

    it('leaves Shift+Enter to the box, which starts a new line itself', () => {
      const home = scene();
      type(home, 'hello');
      expect(key(home, prompt(home), 'Enter', { shiftKey: true }).defaultPrevented).toBe(false);
      expect(home.started).toEqual([]);
    });

    it.each([{ ctrlKey: true }, { ctrlKey: true, shiftKey: true }])('starts a new line at the caret on Enter with %j', (modifiers) => {
      const home = scene();
      type(home, 'hello world');
      prompt(home).setSelectionRange(5, 5);
      expect(key(home, prompt(home), 'Enter', modifiers).defaultPrevented).toBe(true);
      expect(prompt(home).value).toBe('hello\n world');
      expect(home.texts.at(-1)).toBe('hello\n world');
      expect(home.started).toEqual([]);
    });

    it('replaces the selected text with a new line on Control+Enter', () => {
      const home = scene();
      type(home, 'hello big world');
      prompt(home).setSelectionRange(5, 9);
      key(home, prompt(home), 'Enter', { ctrlKey: true });
      expect(prompt(home).value).toBe('hello\n world');
    });

    it('closes the project chooser when a new line is started', () => {
      const home = scene();
      const trigger = button(home, 'Project a');
      clickButton(home, 'Project a');
      expect(trigger.getAttribute('aria-expanded')).toBe('true');
      key(home, prompt(home), 'Enter', { ctrlKey: true });
      expect(trigger.getAttribute('aria-expanded')).toBe('false');
    });

    it('sends text of several lines whole', () => {
      const home = scene();
      type(home, 'Fix the parser\nand its tests');
      key(home, prompt(home), 'Enter');
      expect(home.started.map((started) => started.text)).toEqual(['Fix the parser\nand its tests']);
    });

    it('leaves Enter to the input method while a character is composed', () => {
      const home = scene();
      type(home, 'hello');
      expect(key(home, prompt(home), 'Enter', { isComposing: true }).defaultPrevented).toBe(false);
      expect(home.started).toEqual([]);
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
      expect(home.started).toEqual([{ projectId: 'b', workflow: 'default', text: 'hello', choice: { agent: 'claude', model: null, effort: null } }]);
    });

    it('shows text the workspace puts in the prompt', () => {
      const home = scene([], { text: 'About `README.md`: ' });
      expect(prompt(home).value).toBe('About `README.md`: ');
    });
  });

  describe('the agent chooser', () => {
    const models = { ...NO_MODELS, codex: [model('gpt-6-astra', 'GPT-6 Astra', ['low', 'high'], 'low')] };

    it('offers model and effort for an installed agent and preserves the draft', () => {
      const home = scene([], { models, text: 'keep this' });
      agentChooser(home).click();
      home.fixture.detectChanges();
      clickButton(home, 'Codex');
      clickButton(home, 'GPT-6 Astra');
      clickButton(home, 'High');
      expect(home.chosen).toEqual([{ agent: 'codex', model: 'gpt-6-astra', effort: 'high' }]);
      expect(prompt(home).value).toBe('keep this');
    });

    it('passes the discovered default model and effort when starting a thread', () => {
      const home = scene([], { installed: ['codex'], models, starting: null, text: 'hello' });
      expect(agentChooser(home).textContent?.trim()).toBe('Codex · GPT-6 Astra · Low');
      button(home, 'Run').click();
      expect(home.started[0].choice).toEqual({ agent: 'codex', model: 'gpt-6-astra', effort: 'low' });
    });

    describe('while the first stage chooses', () => {
      const grok: Starting = { type: 'provider', agent: 'grok', model: 'grok-4.6', effort: 'medium' };

      it('shows the first stage\'s pick behind Auto and lets the service start the thread on it', () => {
        const home = scene([], { workflows: RULED_WORKFLOWS, starting: grok, text: 'hello' });
        expect(agentChooser(home).textContent?.trim()).toBe('Auto · Grok · Grok 4.6 · Medium');
        button(home, 'Run').click();
        expect(home.started[0].choice).toBeNull();
      });

      it('shows and sends a manual pick without Auto', () => {
        const picked: Choice = { agent: 'codex', model: 'gpt-6-astra', effort: 'high' };
        const home = scene([], { workflows: RULED_WORKFLOWS, starting: grok, models, pickedChoice: picked, text: 'hello' });
        expect(agentChooser(home).textContent?.trim()).toBe('Codex · GPT-6 Astra · High');
        button(home, 'Run').click();
        expect(home.started[0].choice).toEqual(picked);
      });

      it('says why no agent can work, still letting the person pick one', () => {
        const home = scene([], { workflows: RULED_WORKFLOWS, starting: { type: 'unavailable', problem: 'All agents are at their limits.' }, text: 'hello' });
        expect(agentChooser(home).textContent?.trim()).toBe('Auto · Unavailable');
        expect(agentChooser(home).title).toBe('All agents are at their limits.');
        expect(agentChooser(home).disabled).toBe(false);
        button(home, 'Run').click();
        expect(home.started[0].choice).toBeNull();
      });

      it('reports Automatic chosen from the chooser', () => {
        const home = scene([], { workflows: RULED_WORKFLOWS, starting: grok, pickedChoice: { agent: 'codex', model: null, effort: null } });
        agentChooser(home).click();
        home.fixture.detectChanges();
        clickButton(home, 'Automatic');
        expect(home.automatic()).toBe(1);
        expect(home.chosen).toEqual([]);
      });

      it('follows Settings adding and removing the first stage\'s rule', () => {
        const home = scene([], { starting: grok, text: 'hello' });
        expect(agentChooser(home).textContent?.trim()).toBe('Claude');
        home.fixture.componentInstance.workflows = RULED_WORKFLOWS;
        home.fixture.detectChanges();
        expect(agentChooser(home).textContent?.trim()).toBe('Auto · Grok · Grok 4.6 · Medium');
        button(home, 'Run').click();
        home.fixture.componentInstance.workflows = TEST_WORKFLOWS;
        home.fixture.detectChanges();
        expect(agentChooser(home).textContent?.trim()).toBe('Claude');
        home.fixture.componentInstance.sending = false;
        type(home, 'again');
        button(home, 'Run').click();
        expect(home.started.map((request) => request.choice)).toEqual([null, { agent: 'claude', model: null, effort: null }]);
      });
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
    expect(home.started).toEqual([{ projectId: 'a', workflow: 'default', text: '@existing', choice: { agent: 'claude', model: null, effort: null } }]);
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
