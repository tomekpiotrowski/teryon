import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { AlertController } from '@ionic/angular/alert-controller';
import { IonContent } from '@ionic/angular/ion-content';
import { ErnoAlertsService, ErnoAuthService } from 'erno-angular';
import { ThemeService } from '../theme.service';
import {
  appendTask,
  INITIAL_THREADS,
  PROJECTS,
  projectById,
  resolveAttention,
  type ThreadEntry,
} from './workspace.model';

@Component({
  selector: 'app-home',
  templateUrl: './home.page.html',
  imports: [IonContent],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HomePage {
  readonly auth = inject(ErnoAuthService);
  readonly theme = inject(ThemeService);
  readonly projects = PROJECTS;
  readonly selectedProjectId = signal(PROJECTS[0].id);
  readonly activeProject = computed(() => projectById(this.projects, this.selectedProjectId()));
  readonly threads = signal<Readonly<Record<string, readonly ThreadEntry[]>>>(INITIAL_THREADS);
  readonly activeThread = computed(() => this.threads()[this.selectedProjectId()] ?? []);
  readonly draft = signal('');
  readonly navigationOpen = signal(false);
  readonly settingsOpen = signal(false);
  readonly provider = signal<'claude' | 'grok'>('claude');
  readonly approval = signal<'always' | 'ask'>('always');

  private readonly alertController = inject(AlertController);
  private readonly alerts = inject(ErnoAlertsService);

  selectProject(projectId: string): void {
    this.selectedProjectId.set(projectId);
    this.draft.set('');
    this.navigationOpen.set(false);
  }

  startNewProject(): void {
    this.selectProject('docs');
  }

  updateDraft(event: Event): void {
    this.draft.set((event.target as HTMLTextAreaElement).value);
  }

  handleComposerKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Enter' || event.shiftKey) return;
    event.preventDefault();
    this.run();
  }

  run(): void {
    const projectId = this.selectedProjectId();
    const currentEntries = this.threads()[projectId] ?? [];
    const entries = appendTask(currentEntries, this.draft(), new Date());
    if (entries === currentEntries) return;

    this.threads.update((threads) => ({ ...threads, [projectId]: entries }));
    this.draft.set('');
  }

  resolve(index: number, approved: boolean): void {
    const projectId = this.selectedProjectId();
    const currentEntries = this.threads()[projectId] ?? [];
    const resolution = approved ? 'Approved by you — agents resumed' : 'Rejected by you — agents paused';
    const entries = resolveAttention(currentEntries, index, resolution);
    if (entries === currentEntries) return;
    this.threads.update((threads) => ({ ...threads, [projectId]: entries }));
  }

  toggleProvider(): void {
    this.provider.update((provider) => (provider === 'claude' ? 'grok' : 'claude'));
  }

  toggleApproval(): void {
    this.approval.update((approval) => (approval === 'always' ? 'ask' : 'always'));
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

  private deleteAccount(password: string): void {
    this.auth.deleteAccount(password).subscribe({
      error: (error) =>
        this.alerts.error(error?.status === 403 ? 'Incorrect password.' : 'Could not delete account.'),
    });
  }
}
