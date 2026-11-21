import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { AlertController } from '@ionic/angular/alert-controller';
import { ErnoAlertsService } from 'erno-angular';
import { BehaviorSubject, EMPTY, type Observable, of, Subject } from 'rxjs';

import { WorkspaceApi } from './workspace.api';
import { NO_MODELS, type Change, type Opens, type Project, type Screen, type Thread, type ThreadEntry } from './workspace.model';
import { WorkspaceModule } from './workspace.module';
import { WorkspacePage } from './workspace.page';
import { WorkspaceStore } from './workspace.store';

/** The synchronised lists, with each thread's conversation answered when the test says so. */
class StoreDouble {
  readonly projects$ = new BehaviorSubject<readonly Project[]>([]);
  readonly threads$ = new BehaviorSubject<readonly Thread[]>([]);
  readonly opens$ = new BehaviorSubject<Opens>({ editor: false, terminal: false });
  readonly models$ = of(NO_MODELS);
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

describe('WorkspacePage', () => {
  let page: WorkspacePage;
  let store: StoreDouble;

  beforeEach(() => {
    store = new StoreDouble();
    TestBed.configureTestingModule({
      imports: [WorkspaceModule],
      providers: [
        provideRouter([]),
        { provide: WorkspaceStore, useValue: store },
        { provide: WorkspaceApi, useValue: { markSeen: () => of(undefined) } },
        { provide: ErnoAlertsService, useValue: {} },
        { provide: AlertController, useValue: {} },
      ],
    }).overrideTemplate(WorkspacePage, ''); // Only what the page decides is looked at here, not how it shows it.
    page = TestBed.createComponent(WorkspacePage).componentInstance;
  });

  afterEach(() => page.ngOnDestroy());

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
});
