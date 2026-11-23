import { inject, Injectable } from '@angular/core';
import {
  BehaviorSubject,
  catchError,
  defer,
  distinctUntilChanged,
  EMPTY,
  filter,
  finalize,
  map,
  Observable,
  of,
  scan,
  share,
  switchMap,
} from 'rxjs';
import { ServiceConnection } from '../service/service-connection';
import { applyEvent, NO_LISTS, withEntries, type WorkspaceLists } from '../service/workspace-events';
import type { ModelLists, Opens, Project, Starting, Thread, ThreadEntry, Workflows } from './workspace.model';

/**
 * The workspace as the screen sees it: live lists of projects, threads and entries.
 *
 * Everything comes from the local service, which sends every project and
 * thread when the connection is made and every change after that. The lists
 * are kept from the moment the store exists, so a screen opened later still
 * starts from the latest snapshot. A thread's entries are only sent while
 * someone follows it, which `entries$` does for as long as it is watched.
 */
@Injectable({ providedIn: 'root' })
export class WorkspaceStore {
  private readonly connection = inject(ServiceConnection);
  private readonly lists$ = new BehaviorSubject<WorkspaceLists>(NO_LISTS);
  private readonly following = new Map<string, Observable<ThreadEntry[]>>();

  readonly projects$: Observable<readonly Project[]> = this.lists$.pipe(
    map((lists) => lists.projects),
    distinctUntilChanged(),
  );
  readonly threads$: Observable<readonly Thread[]> = this.lists$.pipe(
    map((lists) => lists.threads),
    distinctUntilChanged(),
  );
  /** What a thread's copy can be opened in, besides the file manager. */
  readonly opens$: Observable<Opens> = this.lists$.pipe(
    map((lists) => lists.opens),
    distinctUntilChanged(),
  );
  /** The models each agent offers besides its default. */
  readonly models$: Observable<ModelLists> = this.lists$.pipe(
    map((lists) => lists.models),
    distinctUntilChanged(),
  );
  /** The workflows threads can follow, and what each stage is called. */
  readonly workflows$: Observable<Workflows> = this.lists$.pipe(
    map((lists) => lists.workflows),
    distinctUntilChanged(),
  );
  /** Who the next new thread starts with, when the workflow's first stage chooses. */
  readonly starting$: Observable<Starting | null> = this.lists$.pipe(
    map((lists) => lists.starting),
    distinctUntilChanged(),
  );
  /** How the connection to the service stands, for the screen to explain a wait. */
  readonly connection$ = this.connection.state$.asObservable();

  constructor() {
    this.connection.events$.subscribe((event) => this.lists$.next(applyEvent(this.lists$.value, event)));
  }

  /** Connects to the service. Safe to call every time the screen opens. */
  start(): void {
    this.connection.start();
  }

  /** Restarts a service this window cannot talk to. It stops the agents that are working. */
  restartService(): void {
    void this.connection.restart();
  }

  /**
   * A thread's entries, in order, as they arrive.
   *
   * Following starts with the first watcher and stops with the last. Each time
   * the connection is made again the thread is followed again, because the
   * service forgets what a closed connection followed.
   */
  entries$(threadId: string): Observable<ThreadEntry[]> {
    let entries = this.following.get(threadId);
    if (!entries) {
      entries = this.connection.state$.pipe(
        filter((state) => state.kind === 'connected'),
        switchMap(() => this.follow(threadId)),
        finalize(() => {
          this.following.delete(threadId);
          this.connection
            .request({ type: 'unfollow_thread', thread_id: threadId })
            .pipe(catchError(() => EMPTY))
            .subscribe();
        }),
        share({ resetOnRefCountZero: true }),
      );
      this.following.set(threadId, entries);
    }
    return entries;
  }

  private follow(threadId: string): Observable<ThreadEntry[]> {
    const added$ = this.connection.events$.pipe(
      filter((event) => event.type === 'entry_added' && event.entry.thread_id === threadId),
      map((event) => (event.type === 'entry_added' ? [event.entry as ThreadEntry] : [])),
    );
    const first$ = defer(() => this.connection.request({ type: 'follow_thread', thread_id: threadId })).pipe(
      map((answer) => (answer.type === 'entries' ? (answer.entries as ThreadEntry[]) : [])),
      catchError(() => of([] as ThreadEntry[])),
    );
    // Entries that arrive while the first list is on its way are kept and merged in.
    return new Observable<ThreadEntry[]>((subscriber) => {
      let early: ThreadEntry[] = [];
      let ready = false;
      const listening = added$.subscribe((added) => {
        if (ready) subscriber.next(added);
        else early = early.concat(added);
      });
      const reading = first$.subscribe((all) => {
        ready = true;
        subscriber.next(withEntries(all, early));
      });
      return () => {
        listening.unsubscribe();
        reading.unsubscribe();
      };
    }).pipe(
      scan((entries, change) => withEntries(entries, change), [] as ThreadEntry[]),
    );
  }
}
