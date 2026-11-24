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
    const element = show(thread({ ended: 'failed', unread: false }));

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
    const running = show(thread({ status: 'running', unread: false }), true);
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
});
