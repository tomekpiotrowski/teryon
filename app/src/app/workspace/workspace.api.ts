import { inject, Injectable } from '@angular/core';
import { map, type Observable } from 'rxjs';
import type { Answer, Opener, Request } from '../service/protocol';
import { ServiceConnection } from '../service/service-connection';
import type { FileView, Listing } from './files.model';
import type { Changes, History } from './history.model';
import type { AgentName, Project, Thread, ThreadEntry } from './workspace.model';

/**
 * The requests the screens make to the local service.
 *
 * Changes made here come back as events too, and the store applies them,
 * so the answers are mostly for the screen to know at once what was made.
 * A refusal arrives as an error carrying a message meant for the person.
 */
@Injectable({ providedIn: 'root' })
export class WorkspaceApi {
  private readonly connection = inject(ServiceConnection);

  createProject(name: string, directory: string): Observable<Project> {
    return this.ask({ type: 'add_project', name, directory }, (answer) =>
      answer.type === 'project' ? (answer.project as Project) : undefined,
    );
  }

  removeProject(projectId: string): Observable<void> {
    return this.ask({ type: 'remove_project', project_id: projectId }, () => undefined as void);
  }

  createThread(projectId: string, agent: AgentName): Observable<Thread> {
    return this.ask({ type: 'start_thread', project_id: projectId, agent, title: '' }, (answer) =>
      answer.type === 'thread' ? (answer.thread as Thread) : undefined,
    );
  }

  sendMessage(threadId: string, text: string): Observable<ThreadEntry> {
    return this.ask({ type: 'send_message', thread_id: threadId, text }, (answer) =>
      answer.type === 'entry' ? (answer.entry as ThreadEntry) : undefined,
    );
  }

  /** Tells the service the person has looked at a thread, which clears its unread mark in every window. */
  markSeen(threadId: string): Observable<void> {
    return this.ask({ type: 'mark_seen', thread_id: threadId }, () => undefined as void);
  }

  stop(threadId: string): Observable<void> {
    return this.ask({ type: 'stop', thread_id: threadId }, () => undefined as void);
  }

  /** Opens a thread's copy in the file manager, or the editor or terminal set up for it. */
  openCopy(threadId: string, opener: Opener): Observable<void> {
    return this.ask({ type: 'open_copy', thread_id: threadId, with: opener }, () => undefined as void);
  }

  /** Every file of a branch or a commit. Without either, the project's directory as it is. */
  files(projectId: string, branch: string | null, commit: string | null): Observable<Listing> {
    return this.ask({ type: 'files', project_id: projectId, branch, commit }, (answer) =>
      answer.type === 'listing' ? (answer.listing as Listing) : undefined,
    );
  }

  /** One file of a branch or a commit, line by line, with its changes marked. */
  file(projectId: string, branch: string | null, commit: string | null, path: string): Observable<FileView> {
    return this.ask({ type: 'file', project_id: projectId, branch, commit, path }, (answer) =>
      answer.type === 'file' ? (answer.file as FileView) : undefined,
    );
  }

  /** The commits of a branch. Without a branch, of whatever the project's directory is on. */
  history(projectId: string, branch: string | null): Observable<History> {
    return this.ask({ type: 'history', project_id: projectId, branch }, (answer) =>
      answer.type === 'history' ? (answer.history as History) : undefined,
    );
  }

  /** The files a commit changed. With `from`, what changed between that commit and this one. */
  changes(projectId: string, commit: string, from: string | null): Observable<Changes> {
    return this.ask({ type: 'changes', project_id: projectId, commit, from }, (answer) =>
      answer.type === 'changes' ? (answer.changes as Changes) : undefined,
    );
  }

  /** One file a commit changed, line by line, with its changes marked. */
  change(projectId: string, commit: string, from: string | null, path: string): Observable<FileView> {
    return this.ask({ type: 'change', project_id: projectId, commit, from, path }, (answer) =>
      answer.type === 'file' ? (answer.file as FileView) : undefined,
    );
  }

  /** Sends a request and picks the expected part of the answer. */
  private ask<Result>(request: Request, pick: (answer: Answer) => Result | undefined): Observable<Result> {
    return this.connection.request(request).pipe(
      map((answer) => {
        const picked = pick(answer);
        if (picked === undefined && answer.type !== 'nothing') {
          throw new Error(`Teryon answered with ${answer.type} where something else was expected.`);
        }
        return picked as Result;
      }),
    );
  }
}
