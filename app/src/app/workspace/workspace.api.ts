import { inject, Injectable } from '@angular/core';
import { map, type Observable } from 'rxjs';
import type { Answer, Configuration, DirectoryListing, Opener, Request, UsageReport } from '../service/protocol';
import { ServiceConnection } from '../service/service-connection';
import type { FileView, Listing } from './files.model';
import type { Changes, History } from './history.model';
import type { Choice, Project, Thread, ThreadEntry, Voice } from './workspace.model';

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

  /** The directories inside a path, for choosing where a project lives. An empty path is the home directory. */
  directories(path: string): Observable<DirectoryListing> {
    return this.ask({ type: 'directories', path }, (answer) => (answer.type === 'directories' ? answer.listing : undefined));
  }

  createProject(name: string, directory: string): Observable<Project> {
    return this.ask({ type: 'add_project', name, directory }, (answer) =>
      answer.type === 'project' ? (answer.project as Project) : undefined,
    );
  }

  removeProject(projectId: string): Observable<void> {
    return this.ask({ type: 'remove_project', project_id: projectId }, () => undefined as void);
  }

  /**
   * Starts a thread on an agent and model, or, without a choice, on what the workflow's first stage chooses.
   *
   * The thread follows the named workflow, or the default one without a name.
   */
  createThread(projectId: string, choice: Choice | null, workflow: string | null = null): Observable<Thread> {
    return this.ask({ type: 'start_thread', project_id: projectId, agent: choice?.agent ?? null, model: choice?.model ?? null, effort: choice?.effort ?? null, title: '', workflow }, (answer) =>
      answer.type === 'thread' ? (answer.thread as Thread) : undefined,
    );
  }

  /** Sends a message to a thread's agent, or to its reviewer. */
  sendMessage(threadId: string, text: string, to: Voice): Observable<ThreadEntry> {
    return this.ask({ type: 'send_message', thread_id: threadId, text, to }, (answer) =>
      answer.type === 'entry' ? (answer.entry as ThreadEntry) : undefined,
    );
  }

  /**
   * Asks an agent other than the thread's own to review its change, on a model, in the same thread.
   *
   * Answers with the line that starts the review.
   */
  reviewChange(threadId: string, choice: Choice): Observable<ThreadEntry> {
    return this.ask({ type: 'review_change', thread_id: threadId, agent: choice.agent, model: choice.model, effort: choice.effort }, (answer) =>
      answer.type === 'entry' ? (answer.entry as ThreadEntry) : undefined,
    );
  }

  /** Puts a thread on another workflow. A stage the new workflow skips moves on to the next one it has. */
  chooseWorkflow(threadId: string, workflow: string): Observable<Thread> {
    return this.ask({ type: 'choose_workflow', thread_id: threadId, turn: null, workflow }, (answer) =>
      answer.type === 'thread' ? (answer.thread as Thread) : undefined,
    );
  }

  /** Switches a thread's agent, or its reviewer, to another agent or model, from the next message on. */
  changeAgent(threadId: string, to: Voice, choice: Choice): Observable<Thread> {
    return this.ask({ type: 'change_agent', thread_id: threadId, to, agent: choice.agent, model: choice.model, effort: choice.effort }, (answer) =>
      answer.type === 'thread' ? (answer.thread as Thread) : undefined,
    );
  }

  /** Tells the service the person has looked at a thread, which clears its unread mark in every window. */
  markSeen(threadId: string): Observable<void> {
    return this.ask({ type: 'mark_seen', thread_id: threadId }, () => undefined as void);
  }

  stop(threadId: string): Observable<void> {
    return this.ask({ type: 'stop', thread_id: threadId }, () => undefined as void);
  }

  /** A starting point for the description of a thread's change, for the person to edit before applying. */
  draftDescription(threadId: string): Observable<string> {
    return this.ask({ type: 'draft_description', thread_id: threadId }, (answer) =>
      answer.type === 'draft' ? answer.text : undefined,
    );
  }

  /**
   * Applies a thread's change to the project as one entry in its history.
   *
   * Returns true when the change overlaps with newer work and the thread's
   * agent has started catching up to apply it automatically when done.
   */
  applyChange(threadId: string): Observable<boolean> {
    return this.ask({ type: 'apply_change', thread_id: threadId, description: null }, (answer) =>
      answer.type === 'applied' ? answer.resolving : undefined,
    );
  }

  /** Brings a thread's change up to date with newer work in the project. True when the two overlap, so nothing changed. */
  catchUp(threadId: string): Observable<boolean> {
    return this.ask({ type: 'catch_up', thread_id: threadId }, (answer) =>
      answer.type === 'caught_up' ? answer.overlaps : undefined,
    );
  }

  /** Sends a thread's change for review on GitHub, described by `description`. */
  sendForReview(threadId: string, description: string): Observable<void> {
    return this.ask({ type: 'send_for_review', thread_id: threadId, description }, () => undefined as void);
  }

  /** Opens a thread's review on GitHub in the browser of the machine Teryon runs on. */
  openReview(threadId: string): Observable<void> {
    return this.ask({ type: 'open_review', thread_id: threadId }, () => undefined as void);
  }

  /**
   * Puts away every quiet thread in one project, or in every project.
   *
   * The answer is how many were newly marked. One that stopped being quiet
   * is left on the board.
   */
  putAwayQuiet(projectId: string | null): Observable<number> {
    return this.ask({ type: 'put_away_quiet', project_id: projectId }, (answer) => (answer.type === 'put_away' ? answer.count : undefined));
  }

  /** Marks finishes the board showed, so the next visit leaves them out. */
  acknowledgeDone(seen: readonly { readonly thread_id: string; readonly at: string }[]): Observable<void> {
    return this.ask({ type: 'acknowledge_done', seen: [...seen] }, () => undefined as void);
  }

  /** Throws a thread's change away, stopping its agent first. */
  discardChange(threadId: string): Observable<void> {
    return this.ask({ type: 'discard_change', thread_id: threadId }, () => undefined as void);
  }

  /** Opens a thread's copy in the file manager, or the editor or terminal set up for it. */
  openCopy(threadId: string, opener: Opener): Observable<void> {
    return this.ask({ type: 'open_copy', thread_id: threadId, with: opener }, () => undefined as void);
  }

  /** Every file of a thread's copy, a branch or a commit. Without any, the project's directory as it is. */
  files(projectId: string, threadId: string | null, branch: string | null, commit: string | null): Observable<Listing> {
    return this.ask({ type: 'files', project_id: projectId, thread_id: threadId, branch, commit }, (answer) =>
      answer.type === 'listing' ? (answer.listing as Listing) : undefined,
    );
  }

  /** One file of a thread's copy, a branch or a commit, line by line, with its changes marked. */
  file(projectId: string, threadId: string | null, branch: string | null, commit: string | null, path: string): Observable<FileView> {
    return this.ask({ type: 'file', project_id: projectId, thread_id: threadId, branch, commit, path }, (answer) =>
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

  /** Every agent's latest limits and the turns of the last 14 days. */
  /** A thread's plan, as markdown. */
  plan(threadId: string): Observable<string> {
    return this.ask({ type: 'plan', thread_id: threadId }, (answer) => (answer.type === 'plan' ? answer.text : undefined));
  }

  usage(): Observable<UsageReport> {
    return this.ask({ type: 'usage' }, (answer) => (answer.type === 'usage' ? answer.report : undefined));
  }

  /** The choices the settings screen shows, as this machine is using them. */
  configuration(): Observable<Configuration> {
    return this.ask({ type: 'configuration' }, (answer) => (answer.type === 'configuration' ? answer.configuration : undefined));
  }

  /**
   * Saves the settings screen and gives back what is now in use.
   *
   * A refusal is an error carrying the sentence, and leaves the file as it was.
   */
  saveConfiguration(configuration: Configuration): Observable<Configuration> {
    return this.ask({ type: 'save_configuration', configuration }, (answer) =>
      answer.type === 'configuration' ? answer.configuration : undefined,
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
