import { TestBed } from '@angular/core/testing';
import { BehaviorSubject, firstValueFrom, type Observable, of, Subject } from 'rxjs';
import type { Answer, Entry, Event, Request } from '../service/protocol';
import { ServiceConnection } from '../service/service-connection';
import { CONNECTED, type ThreadEntry } from './workspace.model';
import { WorkspaceStore } from './workspace.store';

/**
 * A connection that is already up, and answers a follow with the entries the test handed it.
 *
 * Events are pushed by the test, the way the service would after the first list.
 */
class FakeConnection {
  readonly state$ = new BehaviorSubject(CONNECTED);
  private readonly events = new Subject<Event>();
  readonly events$ = this.events.asObservable();
  readonly asked: Request[] = [];

  constructor(private readonly entries: ThreadEntry[]) {}

  request(request: Request): Observable<Answer> {
    this.asked.push(request);
    if (request.type === 'follow_thread') return of({ type: 'entries', entries: this.entries as unknown as Entry[] });
    return of({ type: 'nothing' });
  }

  add(entry: ThreadEntry): void {
    this.events.next({ type: 'entry_added', entry: entry as unknown as Entry });
  }
}

function entry(id: string, position: number): ThreadEntry {
  return { id, thread_id: 't', position, kind: 'notice', body: { text: id }, created_at: '2026-11-18T10:00:00Z' };
}

describe('WorkspaceStore entries', () => {
  let connection: FakeConnection;
  let store: WorkspaceStore;

  beforeEach(() => {
    connection = new FakeConnection([entry('e1', 1)]);
    TestBed.configureTestingModule({ providers: [{ provide: ServiceConnection, useValue: connection }] });
    store = TestBed.inject(WorkspaceStore);
  });

  it('gives a watcher who joins later the entries already read, and follows again once everyone has left', async () => {
    const seen: ThreadEntry[][] = [];
    const watching = store.entries$('t').subscribe((entries) => seen.push(entries));
    await firstValueFrom(store.entries$('t'));

    expect(connection.asked.filter((request) => request.type === 'follow_thread')).toHaveLength(1);
    expect(seen[0]?.map((item) => item.id)).toEqual(['e1']);

    connection.add(entry('e2', 2));
    const joined = await firstValueFrom(store.entries$('t'));
    expect(joined.map((item) => item.id)).toEqual(['e1', 'e2']);
    expect(connection.asked.filter((request) => request.type === 'follow_thread')).toHaveLength(1);

    watching.unsubscribe();
    expect(connection.asked.some((request) => request.type === 'unfollow_thread')).toBe(true);

    const again = await firstValueFrom(store.entries$('t'));
    expect(again.map((item) => item.id)).toEqual(['e1']);
    expect(connection.asked.filter((request) => request.type === 'follow_thread')).toHaveLength(2);
  });
});
