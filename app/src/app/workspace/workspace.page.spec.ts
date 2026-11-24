import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { AlertController } from '@ionic/angular/alert-controller';
import { ErnoAlertsService } from 'erno-angular';
import { BehaviorSubject, EMPTY, type Observable, of, Subject, throwError } from 'rxjs';
import { vi } from 'vitest';

import { WorkspaceApi } from './workspace.api';
import { choosing, FIRST_NEW_THREADS, NO_MODELS, NO_WORKFLOWS, type Change, type Opens, type Project, type Screen, type Thread, type ThreadEntry } from './workspace.model';
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
  markSeen: () => Observable<undefined>;
  createThread: ReturnType<typeof vi.fn>;
  sendMessage: ReturnType<typeof vi.fn>;
}

describe('WorkspacePage', () => {
  let page: WorkspacePage;
  let store: StoreDouble;
  let api: ApiDouble;
  let alerts: { error: ReturnType<typeof vi.fn> };

  beforeEach(() => {
    store = new StoreDouble();
    api = { markSeen: () => of(undefined), createThread: vi.fn(), sendMessage: vi.fn() };
    alerts = { error: vi.fn() };
    TestBed.configureTestingModule({
      imports: [WorkspaceModule],
      providers: [
        provideRouter([]),
        { provide: WorkspaceStore, useValue: store },
        { provide: WorkspaceApi, useValue: api },
        { provide: ErnoAlertsService, useValue: alerts },
        { provide: AlertController, useValue: {} },
      ],
    }).overrideTemplate(WorkspacePage, ''); // Only what the page decides is looked at here, not how it shows it.
    page = TestBed.createComponent(WorkspacePage).componentInstance;
  });

  afterEach(() => page.ngOnDestroy());

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

    it.each(['textarea', 'input'])('leaves ? to the %s used for typing', (tag) => {
      TestBed.tick();
      const target = document.createElement(tag);
      if (tag === 'input') target.className = 'home-input';
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

  describe('starting a thread from Home', () => {
    const created = changed('new', 'new-thread', null);
    const request = { projectId: 'p', workflow: 'small', text: 'Fix the parser' };

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
      expect(page.selection$.value.threadId).toBe('new');
      expect(page.sending$.value).toBe(false);
    });

    it('starts on the agent picked, on the model remembered for it', () => {
      api.createThread.mockReturnValue(of(created));
      api.sendMessage.mockReturnValue(of({}));
      page.newThreads$.next(choosing(FIRST_NEW_THREADS, { agent: 'codex', model: 'gpt-6-astra' }));

      page.startThread(request);

      expect(api.createThread).toHaveBeenCalledWith('p', { agent: 'codex', model: 'gpt-6-astra' }, 'small');
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

    it('keeps the text, now in the new thread\'s message box, when the message is refused', () => {
      api.createThread.mockReturnValue(of(created));
      api.sendMessage.mockReturnValue(throwError(() => new Error('no')));

      page.startThread(request);

      expect(page.draft$.value).toBe('Fix the parser');
      expect(page.selection$.value.threadId).toBe('new');
      expect(page.sending$.value).toBe(false);
      expect(alerts.error).toHaveBeenCalledTimes(1);
    });

    it('does not start a second thread while one is being started', () => {
      api.createThread.mockReturnValue(new Subject());

      page.startThread(request);
      page.startThread(request);

      expect(api.createThread).toHaveBeenCalledTimes(1);
    });
  });

  describe('the agent for new threads', () => {
    it('is shared by Home\'s dropdown and the menu\'s commands', () => {
      const picked = () => {
        let value: string | null = 'unset';
        page.pickedAgent$.subscribe((agent) => (value = agent)).unsubscribe();
        return value;
      };
      expect(picked()).toBeNull();

      page.chooseAgent('codex');
      expect(picked()).toBe('codex');

      page.run({ id: 'x', title: 'x', detail: '', group: 'action', key: null, leader: null, live: false, asking: false, badge: '', action: { kind: 'useChoice', choice: { agent: 'grok', model: null } } });
      expect(picked()).toBe('grok');

      page.chooseAgent('grok');
      expect(picked()).toBe('grok');
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
});
