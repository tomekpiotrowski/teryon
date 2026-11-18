import { HttpClient } from '@angular/common/http';
import { inject, Injectable } from '@angular/core';
import { ERNO_CONFIG } from 'erno-angular';
import type { Observable } from 'rxjs';
import type { FileView, Listing } from './files.model';
import type { Changes, History } from './history.model';
import type { AgentName, Project, Thread, ThreadEntry } from './workspace.model';

/**
 * The requests that change the workspace.
 *
 * Reading goes through synchronisation instead, see `WorkspaceStore`. The
 * exception is a project's files and history, which live in its repository
 * rather than in the database and are therefore asked for.
 */
@Injectable({ providedIn: 'root' })
export class WorkspaceApi {
  private readonly http = inject(HttpClient);
  private readonly api = `${inject(ERNO_CONFIG).baseUrl}/api`;

  createProject(name: string, directory: string): Observable<Project> {
    return this.http.post<Project>(`${this.api}/projects`, { name, directory });
  }

  removeProject(projectId: string): Observable<void> {
    return this.http.delete<void>(`${this.api}/projects/${projectId}`);
  }

  createThread(projectId: string, agent: AgentName): Observable<Thread> {
    return this.http.post<Thread>(`${this.api}/projects/${projectId}/threads`, { agent });
  }

  sendMessage(threadId: string, text: string): Observable<ThreadEntry> {
    return this.http.post<ThreadEntry>(`${this.api}/threads/${threadId}/messages`, { text });
  }

  /** Tells the server the person has looked at a thread, which clears its unread mark on every device. */
  markSeen(threadId: string): Observable<void> {
    return this.http.post<void>(`${this.api}/threads/${threadId}/seen`, {});
  }

  stop(threadId: string): Observable<void> {
    return this.http.post<void>(`${this.api}/threads/${threadId}/stop`, {});
  }

  /** Every file of a branch or a commit. Without either, the project's directory as it is. */
  files(projectId: string, branch: string | null, commit: string | null): Observable<Listing> {
    return this.http.get<Listing>(`${this.api}/projects/${projectId}/files`, { params: given({ branch, commit }) });
  }

  /** One file of a branch or a commit, line by line, with its changes marked. */
  file(projectId: string, branch: string | null, commit: string | null, path: string): Observable<FileView> {
    return this.http.get<FileView>(`${this.api}/projects/${projectId}/file`, { params: given({ branch, commit, path }) });
  }

  /** The commits of a branch. Without a branch, of whatever the project's directory is on. */
  history(projectId: string, branch: string | null): Observable<History> {
    return this.http.get<History>(`${this.api}/projects/${projectId}/history`, { params: given({ branch }) });
  }

  /** The files a commit changed. With `from`, what changed between that commit and this one. */
  changes(projectId: string, commit: string, from: string | null): Observable<Changes> {
    return this.http.get<Changes>(`${this.api}/projects/${projectId}/changes`, { params: given({ commit, from }) });
  }

  /** One file a commit changed, line by line, with its changes marked. */
  change(projectId: string, commit: string, from: string | null, path: string): Observable<FileView> {
    return this.http.get<FileView>(`${this.api}/projects/${projectId}/change`, { params: given({ commit, from, path }) });
  }
}

/** The parts of a request that were given, leaving out the ones that were not. */
function given(parts: Readonly<Record<string, string | null>>): Record<string, string> {
  const present: Record<string, string> = {};
  for (const [name, value] of Object.entries(parts)) {
    if (value !== null) present[name] = value;
  }
  return present;
}
