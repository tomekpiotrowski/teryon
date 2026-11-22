import { inject, Injectable, InjectionToken, OnDestroy } from '@angular/core';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { BehaviorSubject, map } from 'rxjs';

/** A local setup problem, independent of whether the service is connected. */
export interface SetupNotice {
  readonly kind: 'conflict' | 'repair' | 'path' | 'shadowed' | 'failed' | 'diagnostic';
  readonly message: string;
  readonly detail: string | null;
  /** Identifies the existing file the person can choose to replace. */
  readonly replacement: string | null;
}

export interface SetupReport {
  readonly pending: boolean;
  readonly notices: readonly SetupNotice[];
}

export type SetupRequest = { readonly kind: 'status' } | { readonly kind: 'install'; readonly replacement: string | null };

export const TERMINAL_COMMAND_SETUP = new InjectionToken<(request: SetupRequest) => Promise<SetupReport>>('terminal command setup', {
  providedIn: 'root',
  factory: () => async (request) => {
    if (!isTauri()) return { pending: false, notices: [] };
    return request.kind === 'status'
      ? invoke<SetupReport>('terminal_command_status')
      : invoke<SetupReport>('install_terminal_command', { replacement: request.replacement });
  },
});

/**
 * Shows AppImage command setup problems without interrupting work.
 *
 * Setup runs in the desktop app at startup. The interface reads that result
 * until the background worker finishes, and only retries or replaces a command when asked.
 */
@Injectable({ providedIn: 'root' })
export class TerminalCommand implements OnDestroy {
  private readonly setup = inject(TERMINAL_COMMAND_SETUP);
  private started = false;
  private stopped = false;
  private poll: ReturnType<typeof setTimeout> | null = null;
  readonly state$ = new BehaviorSubject<{ readonly report: SetupReport; readonly busy: boolean; readonly dismissed: boolean }>({
    report: { pending: false, notices: [] }, busy: false, dismissed: false,
  });

  readonly retryable$ = this.state$.pipe(map(({ report }) => report.notices.some((notice) =>
    notice.kind === 'failed' || notice.kind === 'conflict' || notice.kind === 'repair',
  )));

  ngOnDestroy(): void {
    this.stopped = true;
    if (this.poll !== null) clearTimeout(this.poll);
  }

  start(): void {
    if (this.started) return;
    this.started = true;
    void this.run({ kind: 'status' });
  }

  retry(): void {
    void this.run({ kind: 'install', replacement: null });
  }

  replace(replacement: string): void {
    if (!this.state$.value.report.notices.some((notice) => notice.replacement === replacement)) return;
    void this.run({ kind: 'install', replacement });
  }

  dismiss(): void {
    this.state$.next({ ...this.state$.value, dismissed: true });
  }

  private async run(request: SetupRequest, polling = false): Promise<void> {
    if (this.stopped || (this.state$.value.busy && !polling)) return;
    this.state$.next({ ...this.state$.value, busy: true });
    let report: SetupReport;
    try {
      report = await this.setup(request);
    } catch (problem) {
      const reason = problem instanceof Error ? problem.message : typeof problem === 'string' ? problem : 'Try again.';
      report = { pending: false, notices: [{ kind: 'failed', message: `Could not set up the terminal command. ${reason}`, detail: null, replacement: null }] };
    }
    if (this.stopped) return;
    this.state$.next({ ...this.state$.value, report, busy: report.pending });
    if (report.pending) {
      this.poll = setTimeout(() => {
        this.poll = null;
        void this.run({ kind: 'status' }, true);
      }, 100);
    }
  }
}
