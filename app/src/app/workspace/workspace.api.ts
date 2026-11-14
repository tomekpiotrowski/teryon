import { HttpClient } from '@angular/common/http';
import { inject, Injectable } from '@angular/core';
import { ERNO_CONFIG } from 'erno-angular';
import type { Observable } from 'rxjs';
import type { AgentName, Project, Thread, ThreadEntry } from './workspace.model';

/**
 * The requests that change the workspace.
 *
 * Reading goes through synchronisation instead, see `WorkspaceStore`.
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

  stop(threadId: string): Observable<void> {
    return this.http.post<void>(`${this.api}/threads/${threadId}/stop`, {});
  }
}
