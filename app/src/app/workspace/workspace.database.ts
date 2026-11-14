import { Injectable } from '@angular/core';
import Dexie, { type Table } from 'dexie';
import { registerDevtoolsDatabase } from 'erno-angular';
import type { Project, Thread, ThreadEntry } from './workspace.model';

/**
 * The copy of the workspace kept in the browser.
 *
 * Erno's synchronisation delivers every change to projects, threads and
 * entries; this is where they land and what the screen reads. Keeping a copy
 * means the screen has something to show at once on the next visit.
 */
@Injectable({ providedIn: 'root' })
export class WorkspaceDatabase extends Dexie {
  readonly projects!: Table<Project, string>;
  readonly threads!: Table<Thread, string>;
  readonly entries!: Table<ThreadEntry, string>;

  constructor() {
    super('teryon-workspace');
    this.version(1).stores({
      projects: 'id',
      threads: 'id, project_id',
      entries: 'id, thread_id',
    });
    registerDevtoolsDatabase(this);
  }
}
