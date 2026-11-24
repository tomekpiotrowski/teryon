import { CommonModule } from '@angular/common';
import { TestBed, type ComponentFixture } from '@angular/core/testing';
import { BehaviorSubject } from 'rxjs';
import { vi } from 'vitest';

import { homeQueue } from './home.model';
import { HomeCardComponent } from './home-card.component';
import { MarkdownPipe } from './markdown.pipe';
import { QuestionsComponent } from './questions.component';
import { groupThreads, type Project, type Thread, type ThreadEntry } from './workspace.model';
import { WorkspaceStore } from './workspace.store';

const project: Project = { id: 'a', name: 'Project a', directory: '/work/a', created_at: '2026-11-01T10:00:00' };

function thread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't',
    project_id: 'a',
    title: 'Fix it',
    agent: 'claude',
    status: 'idle',
    unread: true,
    commands: [],
    change: null,
    finished: null,
    reviewer: null,
    created_at: '2026-11-03T10:00:00',
    ...overrides,
  };
}

function entry(kind: ThreadEntry['kind'], body: Record<string, unknown>, position: number): ThreadEntry {
  return { id: `e${position}`, thread_id: 't', position, kind, body, created_at: '2026-11-03T10:00:00' };
}

describe('HomeCardComponent', () => {
  let entries: BehaviorSubject<ThreadEntry[]>;
  let fixture: ComponentFixture<HomeCardComponent>;

  beforeEach(() => {
    entries = new BehaviorSubject<ThreadEntry[]>([]);
    TestBed.configureTestingModule({
      imports: [CommonModule],
      declarations: [HomeCardComponent, QuestionsComponent, MarkdownPipe],
      providers: [{ provide: WorkspaceStore, useValue: { entries$: () => entries.asObservable() } }],
    });
    fixture = TestBed.createComponent(HomeCardComponent);
  });

  function show(item: Thread, expanded = false): HTMLElement {
    const card = homeQueue(groupThreads([project], [item])).cards[0];
    fixture.componentInstance.card = card;
    fixture.componentInstance.expanded = expanded;
    fixture.componentInstance.workflows = { default: 'default', workflows: [{ name: 'default', description: '', stages: ['talking', 'implementing', 'done'] }], labels: [] };
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  }

  it('shows a collapsed card and expands it on a click without opening the thread', () => {
    const opened = vi.fn();
    const toggled = vi.fn();
    fixture.componentInstance.opened.subscribe(opened);
    fixture.componentInstance.toggled.subscribe(toggled);
    const element = show(thread({ ended: 'failed', unread: true, stage: 'talking' }));

    expect(element.textContent).toContain('Fix it');
    expect(element.textContent).toContain('The run failed');
    expect(element.querySelector('.home-card-open')).toBeNull();
    element.querySelector('button')?.click();

    expect(toggled).toHaveBeenCalledTimes(1);
    expect(opened).not.toHaveBeenCalled();
  });

  it('shows the latest turn, and earlier messages only after the toggle', () => {
    entries.next([
      entry('user_message', { text: 'earlier ask' }, 1),
      entry('agent_message', { text: 'earlier answer' }, 2),
      entry('user_message', { text: 'latest ask' }, 3),
      entry('agent_message', { text: 'latest answer' }, 4),
    ]);
    const element = show(thread({ unread: true, stage: 'implementing' }), true);

    expect(element.textContent).toContain('latest answer');
    expect(element.textContent).not.toContain('earlier answer');
    const toggle = Array.from(element.querySelectorAll('button')).find((button) => button.textContent?.includes('Earlier in this thread'));
    toggle?.click();
    fixture.detectChanges();
    expect(element.textContent).toContain('earlier answer');
    expect(element.textContent).toContain('Mark read');
  });

  it('offers Stop while the thread is working, and a reply once it is idle', () => {
    const running = show(thread({ status: 'running', unread: true, stage: 'talking' }), true);
    expect(running.textContent).toContain('is working');
    expect(running.textContent).toContain('Stop');
    expect(running.querySelector('[data-home-reply]')).toBeNull();

    const stopped = vi.fn();
    fixture.componentInstance.stopped.subscribe(stopped);
    running.querySelector('.stop-action')?.dispatchEvent(new Event('click'));
    expect(stopped).toHaveBeenCalledTimes(1);
  });

  it('shows the agent’s questions in the card', () => {
    entries.next([
      entry('user_message', { text: 'please ask questions' }, 1),
      entry('questions', {
        turn: 'turn-1',
        questions: [{
          question: 'Where should answers be kept?',
          answers: [{ label: 'In a file of its own' }, { label: 'In the thread' }],
          choosing: { kind: 'one', recommended: 0 },
        }],
      }, 2),
    ]);
    const element = show(thread({ asking: true, unread: true }), true);

    const questions = element.querySelector('[aria-label="The agent’s questions"]');
    expect(questions?.textContent).toContain('Where should answers be kept?');
    expect(element.querySelector('[data-home-reply]')).toBeTruthy();
  });

  it('sends a reply from the card', () => {
    const replied: { text: string; to: string }[] = [];
    fixture.componentInstance.replied.subscribe((reply) => replied.push(reply));
    const element = show(thread({ unread: true, stage: 'talking' }), true);
    const box = element.querySelector('[data-home-reply]') as HTMLTextAreaElement;
    box.value = 'Ship it';
    box.dispatchEvent(new Event('input'));
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));

    expect(replied).toEqual([{ text: 'Ship it', to: 'author' }]);
  });

  it('leaves Shift+Enter to the reply box, which starts a new line itself', () => {
    const replied: { text: string; to: string }[] = [];
    fixture.componentInstance.replied.subscribe((reply) => replied.push(reply));
    const element = show(thread({ unread: true, stage: 'talking' }), true);
    const box = element.querySelector('[data-home-reply]') as HTMLTextAreaElement;
    box.value = 'Line one';
    box.dispatchEvent(new Event('input'));
    const event = new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, cancelable: true });
    box.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
    expect(replied).toEqual([]);
  });

  it.each([{ ctrlKey: true }, { ctrlKey: true, shiftKey: true }])('starts a new line in the reply box at the caret on Enter with %j', (modifiers) => {
    const replied: { text: string; to: string }[] = [];
    const drafts: string[] = [];
    fixture.componentInstance.replied.subscribe((reply) => replied.push(reply));
    fixture.componentInstance.draftChanged.subscribe((draft) => drafts.push(draft));
    const element = show(thread({ unread: true, stage: 'talking' }), true);
    const box = element.querySelector('[data-home-reply]') as HTMLTextAreaElement;
    box.value = 'hello world';
    box.dispatchEvent(new Event('input'));
    box.setSelectionRange(5, 5);
    const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, ...modifiers });
    box.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(replied).toEqual([]);
    expect(box.value).toBe('hello\n world');
    expect(drafts.at(-1)).toBe('hello\n world');
  });

  it('replaces selected text with a new line in the reply box on Control+Enter', () => {
    const element = show(thread({ unread: true, stage: 'talking' }), true);
    const box = element.querySelector('[data-home-reply]') as HTMLTextAreaElement;
    box.value = 'hello big world';
    box.dispatchEvent(new Event('input'));
    box.setSelectionRange(5, 9);
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, cancelable: true }));

    expect(box.value).toBe('hello\n world');
  });

  it('sends a multiline reply whole', () => {
    const replied: { text: string; to: string }[] = [];
    fixture.componentInstance.replied.subscribe((reply) => replied.push(reply));
    const element = show(thread({ unread: true, stage: 'talking' }), true);
    const box = element.querySelector('[data-home-reply]') as HTMLTextAreaElement;
    box.value = 'First line\nSecond line';
    box.dispatchEvent(new Event('input'));
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));

    expect(replied).toEqual([{ text: 'First line\nSecond line', to: 'author' }]);
  });

  it('sends the reply to the reviewer once that is who was picked', () => {
    const replied: { text: string; to: string }[] = [];
    const switched: string[] = [];
    fixture.componentInstance.replied.subscribe((reply) => replied.push(reply));
    fixture.componentInstance.toChanged.subscribe((to) => switched.push(to));
    const element = show(thread({
      unread: true,
      reviewer: { agent: 'codex', reviewing: false, verdict: null },
    }), true);

    const target = Array.from(element.querySelectorAll('button')).find((button) => button.textContent?.trim() === 'to author');
    target?.click();
    expect(switched).toEqual(['reviewer']);

    fixture.componentInstance.to = 'reviewer';
    fixture.detectChanges();
    const box = element.querySelector('[data-home-reply]') as HTMLTextAreaElement;
    box.value = 'Is the second one real?';
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));

    expect(replied).toEqual([{ text: 'Is the second one real?', to: 'reviewer' }]);
  });

  it('takes the highlight class while the thread is fresh', () => {
    const element = show(thread({ status: 'running' }));
    expect(element.querySelector('.home-card-fresh')).toBeNull();
    fixture.componentInstance.fresh = true;
    fixture.detectChanges();
    expect(element.querySelector('.home-card-fresh')).toBeTruthy();
  });

  it('turns file citations in agent messages into code links', () => {
    entries.next([
      entry('user_message', { text: 'where is it?' }, 1),
      entry('agent_message', { text: 'See [the parser](file:///work/a/src/parser.rs#L10-L20).' }, 2),
    ]);
    const element = show(thread({ unread: true, stage: 'talking' }), true);
    const link = element.querySelector<HTMLAnchorElement>('a.file-citation');

    expect(link?.textContent).toBe('the parser');
    expect(link?.getAttribute('href')).toBe('#open?path=src%2Fparser.rs&start=10&end=20');
  });

  describe('scrolling the conversation', () => {
    const scrolled = new WeakMap<Element, number>();

    /** Lays the conversation out as if each line were 100 pixels tall in a 320 pixel high card. */
    beforeEach(() => {
      vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockImplementation(function (this: Element) {
        return this.querySelectorAll('.thread-entry, .live-line, .home-earlier').length * 100;
      });
      vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(function (this: Element) {
        return Math.min(320, this.scrollHeight);
      });
      vi.spyOn(Element.prototype, 'scrollTop', 'get').mockImplementation(function (this: Element) {
        return scrolled.get(this) ?? 0;
      });
      vi.spyOn(Element.prototype, 'scrollTop', 'set').mockImplementation(function (this: Element, top: number) {
        scrolled.set(this, Math.max(0, Math.min(top, this.scrollHeight - this.clientHeight)));
      });
    });

    afterEach(() => vi.restoreAllMocks());

    /** One turn: the person's message followed by the agent's messages. */
    function conversation(count: number, from = 1): ThreadEntry[] {
      return Array.from({ length: count }, (_, index) => entry(index ? 'agent_message' : 'user_message', { text: `message ${from + index}` }, from + index));
    }

    function body(): HTMLElement {
      return (fixture.nativeElement as HTMLElement).querySelector('.home-card-body') as HTMLElement;
    }

    function bottom(): number {
      return body().scrollHeight - body().clientHeight;
    }

    function scrollTo(top: number): void {
      body().scrollTop = top;
      body().dispatchEvent(new Event('scroll'));
    }

    function setExpanded(expanded: boolean): void {
      fixture.componentInstance.expanded = expanded;
      fixture.detectChanges();
    }

    function arrive(next: ThreadEntry[]): void {
      entries.next(next);
      fixture.detectChanges();
    }

    it('starts at the bottom when the messages are already there', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);

      expect(bottom()).toBeGreaterThan(0);
      expect(body().scrollTop).toBe(bottom());
    });

    it('goes to the bottom once messages arrive after the card opened', () => {
      show(thread({ unread: true, stage: 'talking' }), true);
      expect(body().scrollTop).toBe(0);

      arrive(conversation(6));
      expect(bottom()).toBeGreaterThan(0);
      expect(body().scrollTop).toBe(bottom());
    });

    it('starts at the bottom when a collapsed card is opened', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }));
      setExpanded(true);

      expect(body().scrollTop).toBe(bottom());
    });

    it('follows new messages while at the bottom', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      const before = bottom();

      arrive(conversation(8));
      expect(bottom()).toBeGreaterThan(before);
      expect(body().scrollTop).toBe(bottom());
    });

    it('follows the working line appearing while at the bottom', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      const before = bottom();

      show(thread({ unread: true, stage: 'talking', status: 'running' }), true);
      expect(bottom()).toBeGreaterThan(before);
      expect(body().scrollTop).toBe(bottom());
    });

    it('stays at the bottom when the last message changes in place', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(0);
      scrollTo(bottom());
      body().scrollTop = 0;

      arrive([...conversation(5), entry('agent_message', { text: 'message 6, longer now' }, 6)]);
      expect(body().scrollTop).toBe(bottom());
    });

    it('leaves the position alone after scrolling up, and follows again back at the bottom', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(50);

      arrive(conversation(8));
      expect(body().scrollTop).toBe(50);

      scrollTo(bottom());
      arrive(conversation(10));
      expect(body().scrollTop).toBe(bottom());
    });

    it('counts being within a few pixels of the bottom as being at the bottom', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(bottom() - 16);

      arrive(conversation(8));
      expect(body().scrollTop).toBe(bottom());
    });

    it('keeps the position when earlier messages are opened', () => {
      entries.next([...conversation(6), ...conversation(6, 7)]);
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(0);
      const toggle = body().querySelector('.home-earlier') as HTMLButtonElement;
      toggle.click();
      fixture.detectChanges();

      expect(body().textContent).toContain('message 1');
      expect(body().scrollTop).toBe(0);
    });

    it('does not follow new messages while earlier messages are being read', () => {
      entries.next([...conversation(6), ...conversation(6, 7)]);
      show(thread({ unread: true, stage: 'talking' }), true);
      (body().querySelector('.home-earlier') as HTMLButtonElement).click();
      fixture.detectChanges();
      body().scrollTop = 0;

      arrive([...conversation(6), ...conversation(8, 7)]);
      expect(body().scrollTop).toBe(0);
    });

    it('keeps the position while a reply is typed or other inputs change', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(50);
      const box = (fixture.nativeElement as HTMLElement).querySelector('[data-home-reply]') as HTMLTextAreaElement;
      box.value = 'Typing';
      box.dispatchEvent(new Event('input'));
      fixture.detectChanges();
      fixture.componentInstance.draft = 'Typing more';
      fixture.componentInstance.fresh = true;
      fixture.componentInstance.commands = [];
      fixture.componentInstance.putAwayId = 'something';
      fixture.detectChanges();

      expect(body().scrollTop).toBe(50);
    });

    it('goes back to the bottom after collapsing and opening the card again', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(50);
      setExpanded(false);
      setExpanded(true);

      expect(body().scrollTop).toBe(bottom());
    });

    it('follows the bottom again after a reply is sent', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(50);
      const box = (fixture.nativeElement as HTMLElement).querySelector('[data-home-reply]') as HTMLTextAreaElement;
      box.value = 'Go on';
      box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));

      arrive([...conversation(6), ...conversation(6, 7)]);
      expect(bottom()).toBeGreaterThan(0);
      expect(body().scrollTop).toBe(bottom());
    });

    it('starts at the bottom of another thread shown in the same card', () => {
      entries.next(conversation(6));
      show(thread({ unread: true, stage: 'talking' }), true);
      scrollTo(50);

      entries.next(conversation(8));
      show(thread({ id: 'other', unread: true, stage: 'talking' }), true);
      expect(body().scrollTop).toBe(bottom());
    });
  });

  it('bolds the title of an unread new card', () => {
    const element = show(thread({ stage: 'done', unread: true }));

    expect(element.querySelector('.home-card-unread')?.textContent).toContain('Fix it');
  });

  it('does not bold the title of an unread reply card', () => {
    const element = show(thread({ asking: true, unread: true }));

    expect(element.querySelector('.home-card')?.getAttribute('data-section')).toBe('waiting');
    expect(element.querySelector('.home-card-unread')).toBeNull();
  });
});
