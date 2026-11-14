import { DOCUMENT } from '@angular/common';
import { inject, Injectable } from '@angular/core';
import { liveQuery, type Table } from 'dexie';
import { ErnoAuthService, ErnoSyncService, type SyncDeltaItem } from 'erno-angular';
import { from, type Observable } from 'rxjs';
import { WorkspaceDatabase } from './workspace.database';
import type { Project, Thread, ThreadEntry } from './workspace.model';

/** Where the browser remembers whose workspace its copy holds. */
const OWNER_KEY = 'teryon-workspace-owner';

/** The synchronised tables, by the name the server knows them by. */
const TABLES = ['projects', 'threads', 'thread_entries'] as const;

/**
 * The workspace as the screen sees it: live lists of projects, threads and entries.
 *
 * The lists come from the browser's copy and change whenever synchronisation
 * delivers something, so the screen never asks for a one-time answer.
 *
 * The copy belongs to one person. Erno keeps a position per table saying how
 * far synchronisation has got, and does not reset it at sign-out, so when
 * someone else signs in the copy is emptied and the positions start over.
 */
@Injectable({ providedIn: 'root' })
export class WorkspaceStore {
  private readonly database = inject(WorkspaceDatabase);
  private readonly sync = inject(ErnoSyncService);
  private readonly auth = inject(ErnoAuthService);
  private readonly storage = inject(DOCUMENT).defaultView?.localStorage;
  private started: Promise<void> | null = null;

  readonly projects$: Observable<Project[]> = from(liveQuery(() => this.database.projects.toArray()));
  readonly threads$: Observable<Thread[]> = from(liveQuery(() => this.database.threads.toArray()));

  entries$(threadId: string): Observable<ThreadEntry[]> {
    return from(liveQuery(() => this.database.entries.where('thread_id').equals(threadId).sortBy('position')));
  }

  /** Starts synchronisation. Safe to call every time the screen opens. */
  start(): Promise<void> {
    this.started ??= this.begin();
    return this.started;
  }

  /**
   * Puts rows the server just answered with into the copy.
   *
   * Synchronisation would deliver them a moment later; this makes what the
   * person just did show up at once.
   */
  async remember(rows: { project?: Project; thread?: Thread; entry?: ThreadEntry }): Promise<void> {
    if (rows.project) await this.database.projects.put(rows.project);
    if (rows.thread) await this.database.threads.put(rows.thread);
    if (rows.entry) await this.database.entries.put(rows.entry);
  }

  private async begin(): Promise<void> {
    this.sync.register('projects', '/api/projects/sync', (item) => this.apply(this.database.projects, item));
    this.sync.register('threads', '/api/threads/sync', (item) => this.apply(this.database.threads, item));
    this.sync.register('thread_entries', '/api/thread-entries/sync', (item) => this.apply(this.database.entries, item));
    await this.sync.start();

    // One change of hands at a time: emptying the copy must not overlap with
    // refilling it for the next person.
    let settled = Promise.resolve();
    this.auth.currentUser$.subscribe((user) => {
      if (user) settled = settled.then(() => this.adopt(user.id));
    });
    this.auth.authState$.subscribe((state) => {
      if (state !== 'anonymous') return;
      this.storage?.removeItem(OWNER_KEY);
      settled = settled.then(() => this.empty());
    });
  }

  /** Makes the copy the given person's, starting over when it held someone else's. */
  private async adopt(owner: string): Promise<void> {
    if (owner === this.storage?.getItem(OWNER_KEY)) return;
    await this.empty();
    for (const table of TABLES) await this.sync.resetCursor(table);
    this.storage?.setItem(OWNER_KEY, owner);
  }

  private async apply<Row extends { id: string }>(table: Table<Row, string>, item: SyncDeltaItem): Promise<void> {
    const row = item.data as (Row & { deleted_at?: string | null }) | null | undefined;
    if (item.deleted || row?.deleted_at) {
      await table.delete(item.id);
    } else if (row) {
      await table.put(row);
    }
  }

  private async empty(): Promise<void> {
    await Promise.all([this.database.projects.clear(), this.database.threads.clear(), this.database.entries.clear()]);
  }
}
