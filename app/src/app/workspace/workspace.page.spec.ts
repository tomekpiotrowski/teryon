import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { AlertController } from '@ionic/angular/alert-controller';
import { ErnoAlertsService } from 'erno-angular';
import { BehaviorSubject, EMPTY, type Observable, of, Subject, throwError } from 'rxjs';
import { vi } from 'vitest';

import { WorkspaceApi } from './workspace.api';
import {
  APPLY_RESOLVING_NOTICE,
  type Choice,
  FIRST_NEW_THREADS,
  NO_MODELS,
  NO_WORKFLOWS,
  REFUSAL,
  type Change,
  type Opens,
  type Project,
  type Screen,
  type Thread,
  type ThreadEntry,
} from './workspace.model';
import { WorkspaceModule } from './workspace.module';
import { WorkspacePage } from './workspace.page';
import { WorkspaceStore } from './workspace.store';

/** The synchronised lists, with each thread's conversation answered when the test says so. */
class StoreDouble {
  readonly projects$ = new BehaviorSubject<readonly Project[]>([]);
  readonly threads$ = new BehaviorSubject<readonly Thread[]>([]);
  readonly opens$ = new BehaviorSubject<Opens>({ editor: false, terminal: false });
  readonly models$ = of(NO_MODELS);
  readonly workflows$ = of(NO_WORKFLOWS);
  readonly installed$ = of(['claude', 'codex']);
  readonly starting$ = of(null);
  readonly connection$ = EMPTY;
  private readonly conversations = new Map<string, Subject<ThreadEntry[]>>();

  start(): void {}

  restartService(): void {}

  entries$(threadId: string): Observable<ThreadEntry[]> {
    let conversation = this.conversations.get(threadId);
    if (!conversation) {
      conversation = new Subject<ThreadEntry[]>();
      this.conversations.set(threadId, conversation);
    }
    return conversation.asObservable();
  }

  answer(threadId: string): void {
    this.conversations.get(threadId)?.next([]);
  }
}

const project: Project = { id: 'p', name: 'App', directory: '/work/app', created_at: '2026-11-01T10:00:00' };

function changed(id: string, name: string, branch: string | null): Thread {
  const change: Change = {
    name,
    path: `/work/app/.teryon/worktrees/${name}`,
    base: 'main',
    start: 'abc',
    branch,
    state: null,
    review: null,
  };
  return {
    id,
    project_id: 'p',
    title: name,
    agent: 'claude',
    status: 'idle',
    unread: false,
    commands: [],
    change,
    finished: null,
    reviewer: null,
    created_at: '2026-11-02T10:00:00',
  };
}

/** What the page asks the service for, answered as each test says. */
interface ApiDouble {
  markSeen: ReturnType<typeof vi.fn>;
  createThread: ReturnType<typeof vi.fn>;
  sendMessage: ReturnType<typeof vi.fn>;
  catchUp: ReturnType<typeof vi.fn>;
  applyChange: ReturnType<typeof vi.fn>;
  draftDescription: ReturnType<typeof vi.fn>;
  sendForReview: ReturnType<typeof vi.fn>;
}

describe('WorkspacePage', () => {
  let page: WorkspacePage;
  let store: StoreDouble;
  let api: ApiDouble;
  let alerts: { error: ReturnType<typeof vi.fn>; info: ReturnType<typeof vi.fn>; success: ReturnType<typeof vi.fn> };
  let alertController: { create: ReturnType<typeof vi.fn> };

  beforeEach(() => {
    const stored = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => void stored.set(key, value),
    });
    store = new StoreDouble();
    api = {
      markSeen: vi.fn(() => of(undefined)),
      createThread: vi.fn(),
      sendMessage: vi.fn(),
      catchUp: vi.fn(() => of(false)),
      applyChange: vi.fn(() => of(false)),
      draftDescription: vi.fn(() => of('Draft description')),
      sendForReview: vi.fn(() => of(undefined)),
    };
    alerts = { error: vi.fn(), info: vi.fn(), success: vi.fn() };
    alertController = { create: vi.fn(() => Promise.resolve({ present: vi.fn(() => Promise.resolve()) })) };
    TestBed.configureTestingModule({
      imports: [WorkspaceModule],
      providers: [
        provideRouter([]),
        { provide: WorkspaceStore, useValue: store },
        { provide: WorkspaceApi, useValue: api },
        { provide: ErnoAlertsService, useValue: alerts },
        { provide: AlertController, useValue: alertController },
      ],
    }).overrideTemplate(WorkspacePage, ''); // Only what the page decides is looked at here, not how it shows it.
    page = TestBed.createComponent(WorkspacePage).componentInstance;
  });

  afterEach(() => {
    page.ngOnDestroy();
    vi.unstubAllGlobals();
  });

  describe('Shortcuts', () => {
    it.each(['Escape', '?'])('opens with ? and closes with %s without leaving the screen', (key) => {
      TestBed.tick();
      page.screen$.next('board');
      page.handleShortcut(new KeyboardEvent('keydown', { key: '?' }));
      expect(page.shortcutsOpen$.value).toBe(true);

      page.handleShortcut(new KeyboardEvent('keydown', { key }));
      expect(page.shortcutsOpen$.value).toBe(false);
      expect(page.screen$.value).toBe('board');
    });

    it('leaves ? to the box used for typing', () => {
      TestBed.tick();
      const target = document.createElement('textarea');
      const event = new KeyboardEvent('keydown', { key: '?', cancelable: true });
      target.addEventListener('keydown', () => page.handleShortcut(event));
      target.dispatchEvent(event);

      expect(page.shortcutsOpen$.value).toBe(false);
      expect(event.defaultPrevented).toBe(false);
    });

    it.each([{ key: '/' }, { key: 'k', ctrlKey: true }])('switches from shortcuts to the menu with %j', (press) => {
      page.openShortcuts();
      page.handleShortcut(new KeyboardEvent('keydown', press));
      expect(page.shortcutsOpen$.value).toBe(false);
      expect(page.menuOpen$.value).toBe(true);
    });
  });

  describe('keys in the message box', () => {
    const view = {} as Parameters<WorkspacePage['send']>[0];
    let send: ReturnType<typeof vi.spyOn>;
    let box: HTMLTextAreaElement;

    beforeEach(() => {
      send = vi.spyOn(page, 'send').mockImplementation(() => undefined);
      box = document.createElement('textarea');
      box.value = 'hello world';
      box.setSelectionRange(5, 5);
      page.draft$.next(box.value);
    });

    function press(init: KeyboardEventInit): KeyboardEvent {
      const event = new KeyboardEvent('keydown', { cancelable: true, ...init });
      box.addEventListener('keydown', () => page.handleComposerKeydown(event, view), { once: true });
      box.dispatchEvent(event);
      return event;
    }

    it('sends on Enter', () => {
      expect(press({ key: 'Enter' }).defaultPrevented).toBe(true);
      expect(send).toHaveBeenCalledWith(view);
    });

    it('leaves Shift+Enter to the box, which starts a new line itself', () => {
      expect(press({ key: 'Enter', shiftKey: true }).defaultPrevented).toBe(false);
      expect(send).not.toHaveBeenCalled();
      expect(page.draft$.value).toBe('hello world');
    });

    it.each([{ ctrlKey: true }, { ctrlKey: true, shiftKey: true }])('starts a new line at the caret on Enter with %j', (modifiers) => {
      expect(press({ key: 'Enter', ...modifiers }).defaultPrevented).toBe(true);
      expect(send).not.toHaveBeenCalled();
      expect(box.value).toBe('hello\n world');
      expect(page.draft$.value).toBe('hello\n world');
    });

    it('replaces the selected text with a new line on Control+Enter', () => {
      box.setSelectionRange(0, 6);
      press({ key: 'Enter', ctrlKey: true });
      expect(page.draft$.value).toBe('\nworld');
    });

    it('leaves the box on Escape', () => {
      document.body.appendChild(box);
      box.focus();
      try {
        expect(press({ key: 'Escape' }).defaultPrevented).toBe(true);
        expect(document.activeElement).not.toBe(box);
        expect(send).not.toHaveBeenCalled();
      } finally {
        box.remove();
      }
    });
  });

  describe('Escape', () => {
    const screens: readonly Screen[] = ['files', 'history', 'usage'];

    describe.each(screens)('from %s', (screen) => {
      beforeEach(() => page.screen$.next(screen));

      it.each(['div', 'input', 'textarea', 'select', 'button'])('returns to the thread with a %s focused', (tag) => {
        const target = document.createElement(tag);
        const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
        target.addEventListener('keydown', (event) => page.handleShortcut(event as KeyboardEvent));

        target.dispatchEvent(event);

        expect(page.screen$.value).toBe('thread');
        expect(event.defaultPrevented).toBe(true);
      });

      it('closes the menu first, then returns to the thread on the next Escape', () => {
        page.openMenu();
        page.handleShortcut(new KeyboardEvent('keydown', { key: 'Escape' }));

        expect(page.menuOpen$.value).toBe(false);
        expect(page.screen$.value).toBe(screen);

        page.handleShortcut(new KeyboardEvent('keydown', { key: 'Escape' }));
        expect(page.screen$.value).toBe('thread');
      });

      it('clears an unfinished shortcut and keeps the draft and repository location', () => {
        page.leader$.next('w');
        page.draft$.next('Keep this reply');
        const place = { projectId: 'p', threadId: null, branch: 'main', commit: 'abc' };
        page.place$.next(place);

        page.handleShortcut(new KeyboardEvent('keydown', { key: 'Escape' }));

        expect(page.screen$.value).toBe('thread');
        expect(page.leader$.value).toBeNull();
        expect(page.draft$.value).toBe('Keep this reply');
        expect(page.place$.value).toEqual(place);
      });

      it.each(['handled', 'composing'])('leaves a %s Escape alone', (kind) => {
        const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true, isComposing: kind === 'composing' });
        if (kind === 'handled') event.preventDefault();

        page.handleShortcut(event);

        expect(page.screen$.value).toBe(screen);
      });

      it.each(['document', 'alert'])('leaves Escape from the %s to an open confirmation', (origin) => {
        const alert = document.createElement('ion-alert');
        document.body.append(alert);
        try {
          const target = origin === 'alert' ? alert : document;
          const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
          target.dispatchEvent(event);
          page.handleShortcut(event);

          expect(page.screen$.value).toBe(screen);
          expect(event.defaultPrevented).toBe(false);
        } finally {
          alert.remove();
        }

        page.handleShortcut(new KeyboardEvent('keydown', { key: 'Escape' }));
        expect(page.screen$.value).toBe('thread');
      });

      it('does not return for an unrelated key', () => {
        page.handleShortcut(new KeyboardEvent('keydown', { key: 'F8' }));
        expect(page.screen$.value).toBe(screen);
      });
    });

    it('leaves Escape alone when already on the thread', () => {
      const event = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
      page.handleShortcut(event);
      expect(page.screen$.value).toBe('thread');
      expect(event.defaultPrevented).toBe(false);
    });
  });

  describe('showCode', () => {
    const first = changed('t1', 'fix-login', 'fix-login');
    const second = changed('t2', 'rename-users', null);

    beforeEach(() => {
      store.projects$.next([project]);
      store.threads$.next([first, second]);
    });

    it('shows the copy of a thread opened a moment ago, before its conversation arrives', () => {
      // The screen watches the view the whole time, as the template does.
      const watching = page.view$.subscribe();
      page.selectThread(first);
      store.answer('t1');
      page.selectThread(second);

      page.showCode();

      expect(page.place$.value).toEqual({ projectId: 'p', threadId: 't2', branch: null, commit: null });
      expect(page.screen$.value).toBe('files');
      watching.unsubscribe();
    });

    it('goes back to the thread from the code, keeping what the code showed', () => {
      page.selectThread(first);
      page.showCode();
      page.showCode();

      expect(page.screen$.value).toBe('thread');
      expect(page.place$.value).toEqual({ projectId: 'p', threadId: 't1', branch: 'fix-login', commit: null });
    });

    it('keeps the place chosen in the history when the code is asked for from there', () => {
      page.selectThread(first);
      page.chooseBranch(project, 'main');
      page.screen$.next('history');

      page.showCode();

      expect(page.place$.value).toEqual({ projectId: 'p', threadId: null, branch: 'main', commit: null });
      expect(page.screen$.value).toBe('files');
    });
  });

  describe('opening a citation', () => {
    const first = changed('t1', 'fix-login', 'fix-login');

    beforeEach(() => {
      store.projects$.next([project]);
      store.threads$.next([first]);
      page.selectThread(first);
      page.place$.next({ projectId: 'p', threadId: null, branch: 'main', commit: 'abc' });
    });

    /** A click on a citation link, as the page's handler receives it. */
    function clickCitation(href: string, init: MouseEventInit = {}): MouseEvent {
      const link = document.createElement('a');
      link.className = 'file-citation';
      link.setAttribute('href', href);
      const event = new MouseEvent('click', { bubbles: true, cancelable: true, ...init });
      link.dispatchEvent(event);
      page.openCitation(event);
      return event;
    }

    it('opens the cited file in the copy, forgetting a commit the history was showing', () => {
      const event = clickCitation('#open?path=src%2Fparser.rs&start=748&end=826');

      expect(event.defaultPrevented).toBe(true);
      expect(page.screen$.value).toBe('files');
      expect(page.place$.value).toEqual({ projectId: 'p', threadId: 't1', branch: 'fix-login', commit: null });
      expect(page.citation$.value).toEqual({ path: 'src/parser.rs', start: 748, end: 826 });
    });

    it.each([
      ['Ctrl', { ctrlKey: true }],
      ['Shift', { shiftKey: true }],
      ['Meta', { metaKey: true }],
      ['the middle button', { button: 1 }],
    ] as const)('leaves a click with %s to the browser', (_, init) => {
      const event = clickCitation('#open?path=src%2Fparser.rs', init);

      expect(event.defaultPrevented).toBe(false);
      expect(page.screen$.value).toBe('thread');
      expect(page.citation$.value).toBeNull();
    });

    it('does nothing with an address the file API would refuse', () => {
      const event = clickCitation('#open?path=.git%2Fconfig');

      expect(event.defaultPrevented).toBe(false);
      expect(page.screen$.value).toBe('thread');
      expect(page.place$.value.commit).toBe('abc');
    });

    it('drops the citation when the code is left, and keeps the place', () => {
      clickCitation('#open?path=src%2Fparser.rs');
      page.screen$.next('thread');

      expect(page.citation$.value).toBeNull();
      expect(page.place$.value).toEqual({ projectId: 'p', threadId: 't1', branch: 'fix-login', commit: null });
    });

    it('opens a citation from an open Home card in that card’s project and copy', () => {
      const other: Project = { id: 'q', name: 'Other', directory: '/other', created_at: '2026-11-01T10:00:00' };
      const cardThread = { ...changed('t2', 'card-fix', 'card-fix'), project_id: 'q' };
      store.projects$.next([project, other]);
      store.threads$.next([cardThread]);
      page.selectProject(project);
      page.homeThreadId$.next('t2');

      const event = clickCitation('#open?path=src%2Flib.rs&start=4&end=4');

      expect(event.defaultPrevented).toBe(true);
      expect(page.screen$.value).toBe('files');
      expect(page.selection$.value).toEqual({ projectId: 'q', threadId: null });
      expect(page.place$.value).toEqual({ projectId: 'q', threadId: 't2', branch: 'card-fix', commit: null });
      expect(page.citation$.value).toEqual({ path: 'src/lib.rs', start: 4, end: 4 });
    });
  });

  describe('starting a thread from Home', () => {
    const created = changed('new', 'new-thread', null);
    const request = { projectId: 'p', workflow: 'small', text: 'Fix the parser', choice: null };

    beforeEach(() => {
      store.projects$.next([project]);
      page.draft$.next('Fix the parser');
    });

    it('creates the thread on the workflow asked for, sends the message and clears the draft', () => {
      api.createThread.mockReturnValue(of(created));
      api.sendMessage.mockReturnValue(of({}));

      page.startThread(request);

      expect(api.createThread).toHaveBeenCalledWith('p', null, 'small');
      expect(api.sendMessage).toHaveBeenCalledWith('new', 'Fix the parser', 'author');
      expect(page.draft$.value).toBe('');
      expect(page.selection$.value.threadId).toBeNull();
      expect(page.highlightedId$.value).toBe('new');
      expect(page.sending$.value).toBe(false);
    });

    it.each(['gpt-6-astra', null])('starts with the exact Home selection (%s)', (model) => {
      api.createThread.mockReturnValue(of(created));
      api.sendMessage.mockReturnValue(of({}));
      const choice = { agent: 'codex' as const, model, effort: 'high' as const };
      page.startThread({ ...request, choice });

      expect(api.createThread).toHaveBeenCalledWith('p', choice, 'small');
    });

    it('keeps the text and says so when the thread cannot be created', () => {
      api.createThread.mockReturnValue(throwError(() => new Error('no')));

      page.startThread(request);

      expect(api.sendMessage).not.toHaveBeenCalled();
      expect(page.draft$.value).toBe('Fix the parser');
      expect(page.selection$.value.threadId).toBeNull();
      expect(page.sending$.value).toBe(false);
      expect(alerts.error).toHaveBeenCalledTimes(1);
    });

    it('keeps the text in the prompt when the message is refused, and reuses the thread on the next try', () => {
      api.createThread.mockReturnValue(of(created));
      api.sendMessage.mockReturnValueOnce(throwError(() => new Error('no'))).mockReturnValueOnce(of({}));

      page.startThread(request);

      expect(page.draft$.value).toBe('Fix the parser');
      expect(page.selection$.value.threadId).toBeNull();
      expect(page.highlightedId$.value).toBeNull();
      expect(page.sending$.value).toBe(false);
      expect(alerts.error).toHaveBeenCalledTimes(1);

      page.startThread(request);

      expect(api.createThread).toHaveBeenCalledTimes(1);
      expect(api.sendMessage).toHaveBeenCalledTimes(2);
      expect(page.highlightedId$.value).toBe('new');
    });

    it('does not start a second thread while one is being started', () => {
      api.createThread.mockReturnValue(new Subject());

      page.startThread(request);
      page.startThread(request);

      expect(api.createThread).toHaveBeenCalledTimes(1);
    });

    it('stops highlighting the new thread after a moment', () => {
      vi.useFakeTimers();
      api.createThread.mockReturnValue(of(created));
      api.sendMessage.mockReturnValue(of({}));

      page.startThread(request);
      expect(page.highlightedId$.value).toBe('new');
      vi.advanceTimersByTime(2000);

      expect(page.highlightedId$.value).toBeNull();
      vi.useRealTimers();
    });
  });

  describe('a card open on Home', () => {
    const planned = {
      ...changed('t1', 'fix-login', 'fix-login'),
      plan: { title: 'The plan', fingerprint: 'x', too_large: false },
      unread: true,
    };

    beforeEach(() => {
      store.projects$.next([project]);
      store.threads$.next([planned]);
      page.homeThreadId$.next(planned.id);
    });

    it('comes back from the plan to Home with the card still open', () => {
      page.showPlanFor(planned);
      expect(page.screen$.value).toBe('plan');
      expect(page.selection$.value.threadId).toBeNull();

      page.handleShortcut(new KeyboardEvent('keydown', { key: 'Escape' }));

      expect(page.screen$.value).toBe('thread');
      expect(page.homeThreadId$.value).toBe(planned.id);
    });

    it('offers the card’s plan among the commands', () => {
      const commands = page.commands$.subscribe();
      TestBed.tick();
      let plan = false;
      page.commands$.subscribe((list) => {
        plan = list.some((command) => command.action.kind === 'showScreen' && command.action.screen === 'plan');
      }).unsubscribe();
      expect(plan).toBe(true);
      commands.unsubscribe();
    });

    it('does not offer the card’s plan on the board', () => {
      page.screen$.next('board');
      const commands = page.commands$.subscribe();
      TestBed.tick();
      let plan = true;
      page.commands$.subscribe((list) => {
        plan = list.some((command) => command.action.kind === 'showScreen' && command.action.screen === 'plan');
      }).unsubscribe();
      expect(plan).toBe(false);
      commands.unsubscribe();
    });

    it('closes the card when a new thread is started, so the prompt takes the message', () => {
      page.selection$.next({ projectId: project.id, threadId: planned.id });
      const commands = page.commands$.subscribe();
      TestBed.tick();
      let started: Parameters<WorkspacePage['run']>[0] | undefined;
      page.commands$.subscribe((list) => {
        started = list.find((command) => command.id === 'action:newThread');
      }).unsubscribe();

      page.run(started!);

      expect(page.selection$.value).toEqual({ projectId: project.id, threadId: null });
      expect(page.homeThreadId$.value).toBeNull();
      expect(page.screen$.value).toBe('thread');
      commands.unsubscribe();
    });

    it('puts a drafted review into the card and leaves the prompt alone', () => {
      page.homeLines$.next([
        { kind: 'review', text: 'Review', reviewer: false },
        { kind: 'agent', text: 'A finding\n\nFindings: 1', reviewer: true },
      ]);
      const commands = page.commands$.subscribe();
      TestBed.tick();

      page.passFindings();

      expect(page.draft$.value).toBe('');
      expect(page.cardNote$.value?.threadId).toBe(planned.id);
      expect(page.cardNote$.value?.text).toContain('A finding');
      expect(page.selection$.value.threadId).toBeNull();
      commands.unsubscribe();
    });

    it('adds an asked-about file to the card reply instead of the prompt text', () => {
      page.draft$.next('prompt text');

      page.askAbout('src/a.rs');

      expect(page.draft$.value).toBe('prompt text');
      expect(page.cardNote$.value).toMatchObject({ threadId: planned.id, add: true });
      expect(page.cardNote$.value?.text).toBe('About `src/a.rs`: ');
    });

    it('puts a catch-up request into the card when the change overlaps', () => {
      api.catchUp.mockReturnValue(of(true));

      page.catchUp(planned);

      expect(api.catchUp).toHaveBeenCalledWith(planned.id);
      expect(page.draft$.value).toBe('');
      expect(page.cardNote$.value?.text).toContain('newer work');
    });

    it('does not mark the card read just because Home is showing it', () => {
      const seen = vi.fn(() => of(undefined));
      api.markSeen = seen;
      const watching = page.view$.subscribe();
      TestBed.tick();

      expect(seen).not.toHaveBeenCalled();
      watching.unsubscribe();
    });
  });

  describe('the model for new threads', () => {
    it('uses the same state for Home and menu choices and remembers each agent’s model', () => {
      const picked = vi.fn();
      const subscription = page.pickedChoice$.subscribe(picked);
      expect(picked).toHaveBeenLastCalledWith(null);

      const codex: Choice = { agent: 'codex', model: 'gpt-6-astra', effort: 'high' };
      page.chooseForNewThreads(codex);
      expect(picked).toHaveBeenLastCalledWith(codex);

      const grok: Choice = { agent: 'grok', model: 'grok-4.6', effort: null };
      page.run({ id: 'x', title: 'x', detail: '', group: 'action', key: null, leader: null, live: false, asking: false, badge: '', action: { kind: 'useChoice', choice: grok } });
      expect(picked).toHaveBeenLastCalledWith(grok);
      expect(page.newThreads$.value).toEqual({
        agent: 'grok',
        models: { ...FIRST_NEW_THREADS.models, codex: codex.model, grok: grok.model },
        efforts: { 'codex:gpt-6-astra': 'high' },
        picked: true,
      });
      expect(JSON.parse(localStorage.getItem('teryon.newThreads')!)).toEqual(page.newThreads$.value);
      subscription.unsubscribe();
    });

    it.each(['Home', 'menu'])('goes back to automatic selection from the %s, keeping remembered models and efforts', (from) => {
      const picked = vi.fn();
      const subscription = page.pickedChoice$.subscribe(picked);
      const codex: Choice = { agent: 'codex', model: 'gpt-6-astra', effort: 'high' };
      page.chooseForNewThreads(codex);
      const manual = page.newThreads$.value;

      if (from === 'Home') page.chooseAutomaticForNewThreads();
      else page.run({ id: 'action:use:automatic', title: 'x', detail: '', group: 'action', key: null, leader: null, live: false, asking: false, badge: '', action: { kind: 'useAutomatic' } });

      expect(picked).toHaveBeenLastCalledWith(null);
      expect(page.newThreads$.value).toEqual({ ...manual, picked: false });
      expect(JSON.parse(localStorage.getItem('teryon.newThreads')!)).toEqual({ ...manual, picked: false });
      subscription.unsubscribe();
    });
  });

  describe('Home', () => {
    const first = changed('t1', 'fix-login', null);

    beforeEach(() => {
      store.projects$.next([project]);
      store.threads$.next([first]);
      page.selectThread(first);
      // The page offers commands once the open thread's conversation has arrived.
      store.answer('t1');
    });

    it.each<Screen>(['files', 'history', 'usage', 'settings', 'board', 'thread'])('is reached from %s by G then H', (screen) => {
      page.screen$.next(screen);
      const watching = page.view$.subscribe();
      const commands = page.commands$.subscribe();
      TestBed.tick();
      store.answer('t1');

      page.handleShortcut(new KeyboardEvent('keydown', { key: 'g' }));
      expect(page.leader$.value).toBe('g');
      page.handleShortcut(new KeyboardEvent('keydown', { key: 'h' }));

      expect(page.leader$.value).toBeNull();
      expect(page.selection$.value.threadId).toBeNull();
      expect(page.screen$.value).toBe('thread');
      watching.unsubscribe();
      commands.unsubscribe();
    });

    it('is reached from the menu, leaving the draft behind', () => {
      page.draft$.next('half written');
      const commands = page.commands$.subscribe();
      TestBed.tick();
      store.answer('t1');
      page.openMenu();
      let home: Parameters<WorkspacePage['run']>[0] | undefined;
      page.commands$.subscribe((list) => (home = list.find((command) => command.id === 'action:home'))).unsubscribe();

      expect(home).toBeDefined();
      page.run(home as Parameters<WorkspacePage['run']>[0]);

      expect(page.menuOpen$.value).toBe(false);
      expect(page.selection$.value.threadId).toBeNull();
      expect(page.draft$.value).toBe('');
      commands.unsubscribe();
    });

    it('lets the board keep H for itself until a G is waiting', () => {
      page.screen$.next('board');
      const event = new KeyboardEvent('keydown', { key: 'h', cancelable: true });
      page.handleShortcut(event);
      expect(page.screen$.value).toBe('board');
      expect(event.defaultPrevented).toBe(false);
    });
  });

  describe('applying and sending for review', () => {
    const target = changed('t1', 'fix-login', 'fix-login');

    it('applies directly without opening an alert dialog and shows success when applied immediately', () => {
      api.applyChange.mockReturnValue(of(false));

      page.applyChange(target);

      expect(api.draftDescription).not.toHaveBeenCalled();
      expect(alertController.create).not.toHaveBeenCalled();
      expect(api.applyChange).toHaveBeenCalledWith('t1');
      expect(alerts.success).toHaveBeenCalledWith('Applied to the project.');
      expect(alerts.info).not.toHaveBeenCalled();
      expect(alerts.error).not.toHaveBeenCalled();
    });

    it('shows an info toast without opening a dialog when applying starts resolving an overlap', () => {
      api.applyChange.mockReturnValue(of(true));

      page.applyChange(target);

      expect(alertController.create).not.toHaveBeenCalled();
      expect(api.applyChange).toHaveBeenCalledWith('t1');
      expect(alerts.info).toHaveBeenCalledWith(APPLY_RESOLVING_NOTICE);
      expect(alerts.success).not.toHaveBeenCalled();
    });

    it('shows the refusal reason when applying fails', () => {
      const refusal = Object.assign(new Error('No files have changed, so there is nothing to apply.'), { name: REFUSAL });
      api.applyChange.mockReturnValue(throwError(() => refusal));

      page.applyChange(target);

      expect(alertController.create).not.toHaveBeenCalled();
      expect(alerts.error).toHaveBeenCalledWith('No files have changed, so there is nothing to apply.');
    });

    it('keeps the description dialog for sending for review on GitHub', async () => {
      const present = vi.fn(() => Promise.resolve());
      alertController.create.mockResolvedValue({ present });

      page.sendForReview(target);
      await Promise.resolve();

      expect(api.draftDescription).toHaveBeenCalledWith('t1');
      expect(alertController.create).toHaveBeenCalledTimes(1);
      expect(alertController.create.mock.calls[0][0].header).toBe('Send for review on GitHub');
      expect(present).toHaveBeenCalledTimes(1);
    });
  });
});
