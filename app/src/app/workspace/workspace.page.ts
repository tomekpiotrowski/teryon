import { ChangeDetectionStrategy, Component, ElementRef, HostListener, inject, OnDestroy, ViewChild } from '@angular/core';
import { AlertController } from '@ionic/angular/alert-controller';
import { ErnoAlertsService, ErnoAuthService } from 'erno-angular';
import {
  BehaviorSubject,
  combineLatest,
  distinctUntilChanged,
  map,
  type Observable,
  of,
  shareReplay,
  Subscription,
  switchMap,
  tap,
} from 'rxjs';
import { ThemeService } from '../theme.service';
import { askDraft } from './files.model';
import { type Commit, revertDraft } from './history.model';
import { WorkspaceApi } from './workspace.api';
import {
  type AgentName,
  agentLabel,
  type EntryLine,
  entryLine,
  groupThreads,
  NO_PLACE,
  NOTHING_SELECTED,
  otherAgent,
  type Place,
  placeIn,
  type Project,
  type ProjectGroup,
  refusalText,
  resolveSelection,
  type Screen,
  screenAfter,
  type Selection,
  shortcutScreen,
  type Thread,
  threadLabel,
} from './workspace.model';
import { WorkspaceStore } from './workspace.store';

/** Everything the template needs about the workspace at one moment. */
interface WorkspaceView {
  readonly groups: readonly ProjectGroup[];
  readonly project: Project | null;
  readonly thread: Thread | null;
  readonly lines: readonly EntryLine[];
  readonly running: boolean;
  /** How many agents are running in the selected project. */
  readonly liveAgents: number;
  /** The selected project's threads, for the history to name where a commit came from. */
  readonly projectThreads: readonly Thread[];
  /** The part of the selected project's repository the files and the history show. */
  readonly place: Place;
}

/**
 * The workspace screen: projects and their threads on the side, one thread in the middle.
 *
 * Everything shown is derived from observables: the synchronised lists in
 * `WorkspaceStore` and a handful of subjects holding what the person has
 * selected and typed. Handlers only push into those subjects or send a request.
 */
@Component({
  selector: 'app-workspace',
  templateUrl: './workspace.page.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class WorkspacePage implements OnDestroy {
  readonly auth = inject(ErnoAuthService);
  readonly theme = inject(ThemeService);
  private readonly store = inject(WorkspaceStore);
  private readonly api = inject(WorkspaceApi);
  private readonly alerts = inject(ErnoAlertsService);
  private readonly alertController = inject(AlertController);
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);

  readonly threadLabel = threadLabel;
  readonly agentLabel = agentLabel;

  readonly selection$ = new BehaviorSubject<Selection>(NOTHING_SELECTED);
  readonly draft$ = new BehaviorSubject('');
  readonly agent$ = new BehaviorSubject<AgentName>('claude');
  readonly navigationOpen$ = new BehaviorSubject(false);
  readonly settingsOpen$ = new BehaviorSubject(false);
  readonly projectFormOpen$ = new BehaviorSubject(false);
  /** True while a request is on its way, so a second press does not send it twice. */
  readonly sending$ = new BehaviorSubject(false);
  /** Whether the selected project's threads, files or history fill the screen. */
  readonly screen$ = new BehaviorSubject<Screen>('thread');
  /** The branch or commit the files and the history were last asked to show. */
  readonly place$ = new BehaviorSubject<Place>(NO_PLACE);

  readonly view$: Observable<WorkspaceView> = combineLatest([
    combineLatest([this.store.projects$, this.store.threads$]).pipe(
      map(([projects, threads]) => groupThreads(projects, threads)),
    ),
    this.selection$,
    this.place$,
  ]).pipe(
    switchMap(([groups, selection, place]) => {
      const { project, thread } = resolveSelection(groups, selection);
      const projectThreads = groups.find((group) => group.project.id === project?.id)?.threads ?? [];
      const entries$ = thread ? this.store.entries$(thread.id) : of([]);
      return entries$.pipe(
        map((entries) => ({
          groups,
          project,
          thread,
          lines: entries.map(entryLine),
          running: thread?.status === 'running',
          liveAgents: projectThreads.filter((candidate) => candidate.status === 'running').length,
          projectThreads,
          place: project ? placeIn(place, project.id) : NO_PLACE,
        })),
      );
    }),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  @ViewChild('thread') private thread?: ElementRef<HTMLElement>;

  /** Keeps the newest line in sight as a thread grows. */
  private readonly following: Subscription = this.view$
    .pipe(
      map((view) => `${view.thread?.id}:${view.lines.length}:${view.running}`),
      distinctUntilChanged(),
    )
    .subscribe(() => requestAnimationFrame(() => this.thread?.nativeElement.scrollTo({ top: Number.MAX_SAFE_INTEGER })));

  constructor() {
    void this.store.start();
  }

  ngOnDestroy(): void {
    this.following.unsubscribe();
  }

  /** `C` shows the files and `H` the history, as the hints on screen say. Pressed again, each goes back to the thread. */
  @HostListener('document:keydown', ['$event'])
  handleShortcut(event: KeyboardEvent): void {
    const target = event.target instanceof HTMLElement ? event.target : null;
    const press = {
      key: event.key,
      ctrlKey: event.ctrlKey,
      metaKey: event.metaKey,
      altKey: event.altKey,
      element: target?.tagName ?? '',
      editable: target?.isContentEditable ?? false,
    };
    const asked = shortcutScreen(press);
    if (!asked || this.settingsOpen$.value) return;
    event.preventDefault();
    this.screen$.next(screenAfter(this.screen$.value, asked));
  }

  /** Goes back to the thread with the file named in the message box, ready to be asked about. */
  askAbout(path: string): void {
    this.draftAndReturn(askDraft(this.draft$.value, path));
  }

  /**
   * Goes back to the thread with a request to revert a commit in the message box.
   *
   * Teryon never changes a repository itself. Reverting is work for an
   * agent, and the person still decides whether to send the request.
   */
  askToRevert(commit: Commit): void {
    this.draftAndReturn(revertDraft(this.draft$.value, commit));
  }

  /** Shows a branch of a project in the files and the history. Nothing means the directory as it is. */
  chooseBranch(project: Project, branch: string | null): void {
    this.place$.next({ projectId: project.id, branch, commit: null });
  }

  /** Shows the files of a project as a commit left them. */
  browseAt(project: Project, commit: string): void {
    this.place$.next({ ...placeIn(this.place$.value, project.id), commit });
    this.screen$.next('files');
  }

  /** Shows the thread a commit came from. */
  openThread(thread: Thread): void {
    this.selectThread(thread);
    this.screen$.next('thread');
  }

  selectProject(project: Project): void {
    this.selection$.next({ projectId: project.id, threadId: null });
    this.draft$.next('');
    this.navigationOpen$.next(false);
  }

  selectThread(thread: Thread): void {
    this.selection$.next({ projectId: thread.project_id, threadId: thread.id });
    this.draft$.next('');
    this.navigationOpen$.next(false);
  }

  toggleAgent(): void {
    this.agent$.next(otherAgent(this.agent$.value));
  }

  updateDraft(event: Event): void {
    this.draft$.next((event.target as HTMLTextAreaElement).value);
  }

  handleComposerKeydown(event: KeyboardEvent, view: WorkspaceView): void {
    if (event.key !== 'Enter' || event.shiftKey) return;
    event.preventDefault();
    this.send(view);
  }

  /** Sends what was typed: to the open thread, or as the first message of a new one. */
  send(view: WorkspaceView): void {
    const text = this.draft$.value.trim();
    if (!text || !view.project || view.running || this.sending$.value) return;

    const thread$ = view.thread
      ? of(view.thread)
      : this.api.createThread(view.project.id, this.agent$.value).pipe(
          tap((thread) => {
            void this.store.remember({ thread });
            this.selection$.next({ projectId: thread.project_id, threadId: thread.id });
          }),
        );

    this.sending$.next(true);
    thread$.pipe(switchMap((thread) => this.api.sendMessage(thread.id, text))).subscribe({
      next: (entry) => {
        void this.store.remember({ entry });
        this.draft$.next('');
        this.sending$.next(false);
      },
      error: (failure: unknown) => {
        this.sending$.next(false);
        this.alerts.error(refusalText(failure, 'The message could not be sent.'));
      },
    });
  }

  stop(thread: Thread): void {
    this.api.stop(thread.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The agent could not be stopped.')),
    });
  }

  addProject(event: Event, name: string, directory: string): void {
    event.preventDefault();
    this.api.createProject(name, directory).subscribe({
      next: (project) => {
        void this.store.remember({ project });
        this.projectFormOpen$.next(false);
        this.selectProject(project);
      },
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The project could not be added.')),
    });
  }

  async confirmRemoveProject(project: Project): Promise<void> {
    const alert = await this.alertController.create({
      header: 'Remove project',
      message: `Remove ${project.name} and its threads from Teryon? The directory itself is left untouched.`,
      buttons: [
        { text: 'Cancel', role: 'cancel' },
        { text: 'Remove', role: 'destructive', handler: () => this.removeProject(project) },
      ],
    });
    await alert.present();
  }

  logout(): void {
    this.auth.logout().subscribe({ error: () => undefined });
  }

  async confirmDeleteAccount(): Promise<void> {
    const alert = await this.alertController.create({
      header: 'Delete account',
      message: 'This permanently deletes your account and all your data. Enter your password to confirm.',
      inputs: [{ name: 'password', type: 'password', placeholder: 'Current password' }],
      buttons: [
        { text: 'Cancel', role: 'cancel' },
        {
          text: 'Delete',
          role: 'destructive',
          handler: (data: { password: string }) => this.deleteAccount(data.password),
        },
      ],
    });
    await alert.present();
  }

  /** Goes back to the thread with a text in the message box, ready to be finished and sent. */
  private draftAndReturn(draft: string): void {
    this.draft$.next(draft);
    this.screen$.next('thread');
    requestAnimationFrame(() => {
      const box = this.host.nativeElement.querySelector('textarea');
      box?.focus();
      box?.setSelectionRange(box.value.length, box.value.length);
    });
  }

  private removeProject(project: Project): void {
    this.api.removeProject(project.id).subscribe({
      error: (failure: unknown) => this.alerts.error(refusalText(failure, 'The project could not be removed.')),
    });
  }

  private deleteAccount(password: string): void {
    this.auth.deleteAccount(password).subscribe({
      error: (error) =>
        this.alerts.error(error?.status === 403 ? 'Incorrect password.' : 'Could not delete account.'),
    });
  }
}

