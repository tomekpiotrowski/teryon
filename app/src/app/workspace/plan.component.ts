import { ChangeDetectionStrategy, Component, EventEmitter, Input, Output, inject } from '@angular/core';
import { BehaviorSubject, catchError, distinctUntilChanged, filter, map, type Observable, of, startWith, switchMap } from 'rxjs';
import { WorkspaceApi } from './workspace.api';
import { refusalText, type Thread, threadLabel } from './workspace.model';

/** The plan as it stands, or why it could not be read. Neither while it is on its way. */
interface PlanView {
  readonly title: string;
  readonly text: string | null;
  readonly problem: string | null;
}

/**
 * The Plan screen: the open thread's plan, rendered as markdown.
 *
 * The plan is a file the thread's agent writes, see `crate::plans` in the
 * service. The screen reads it when it opens and again whenever the service
 * says the plan changed, so it can stay open while the agent reworks it.
 */
@Component({
  selector: 'app-plan',
  templateUrl: './plan.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class PlanComponent {
  @Output() readonly menuOpened = new EventEmitter<void>();

  private readonly api = inject(WorkspaceApi);
  private readonly thread$ = new BehaviorSubject<Thread | null>(null);

  @Input() set thread(thread: Thread | null) {
    this.thread$.next(thread);
  }

  readonly view$: Observable<PlanView> = this.thread$.pipe(
    filter((thread): thread is Thread => thread !== null),
    distinctUntilChanged((before, after) => before.id === after.id && before.plan?.fingerprint === after.plan?.fingerprint),
    switchMap((thread) => {
      const title = threadLabel(thread);
      return this.api.plan(thread.id).pipe(
        map((text): PlanView => ({ title, text, problem: null })),
        catchError((failure: unknown) => of<PlanView>({ title, text: null, problem: refusalText(failure, 'The plan could not be read.') })),
        startWith<PlanView>({ title, text: null, problem: null }),
      );
    }),
  );
}
