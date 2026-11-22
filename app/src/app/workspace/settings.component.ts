import { ChangeDetectionStrategy, Component, EventEmitter, inject, Output } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { BehaviorSubject, catchError, combineLatest, EMPTY, exhaustMap, map, type Observable, of, Subject } from 'rxjs';
import type { Configuration, Stage, Strength } from '../service/protocol';
import {
  chooseDefault,
  describeWorkflow,
  fromConfiguration,
  type ProviderMode,
  refusal,
  renameWorkflow,
  setAgent,
  setEditor,
  setLabel,
  setMode,
  setModel,
  setRounds,
  setStrength,
  setTerminal,
  type SettingsDraft,
  settingsView,
  type SettingsView,
  toggleAmong,
  toggleWorkflowStage,
  toConfiguration,
  withWorkflow,
  withoutWorkflow,
} from './settings.model';
import { WorkspaceApi } from './workspace.api';
import { type AgentName, refusalText } from './workspace.model';
import { WorkspaceStore } from './workspace.store';

/** The draft on screen, the last one the service confirmed, and why the last save was refused. */
interface EditorState {
  readonly draft: SettingsDraft | null;
  readonly saved: Configuration | null;
  readonly problem: string | null;
  readonly saving: boolean;
  readonly loadProblem: string | null;
}

const EMPTY_EDITOR: EditorState = { draft: null, saved: null, problem: null, saving: false, loadProblem: null };

/**
 * The settings screen: everyday choices and the workflows threads follow.
 *
 * It belongs to no project. The choices are this machine's, and a save
 * starts applying on the next turn. An agent already working finishes with
 * the program, model and instructions it was given.
 */
@Component({
  selector: 'app-settings',
  templateUrl: './settings.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class SettingsComponent {
  @Output() readonly closed = new EventEmitter<void>();
  @Output() readonly menuOpened = new EventEmitter<void>();

  private readonly api = inject(WorkspaceApi);
  private readonly store = inject(WorkspaceStore);
  private readonly editor$ = new BehaviorSubject<EditorState>(EMPTY_EDITOR);
  private readonly saveAsk$ = new Subject<void>();
  private nextKey = 1;

  readonly view$: Observable<SettingsView> = combineLatest([this.editor$, this.store.models$]).pipe(
    map(([editor, models]) => settingsView(editor.draft, editor.saved, editor.problem, editor.saving, editor.loadProblem, models)),
  );

  constructor() {
    this.api.configuration().pipe(takeUntilDestroyed()).subscribe({
      next: (configuration) => this.editor$.next({ draft: fromConfiguration(configuration), saved: configuration, problem: null, saving: false, loadProblem: null }),
      error: (failure: unknown) => this.editor$.next({ ...EMPTY_EDITOR, loadProblem: refusalText(failure, 'Could not read the settings.') }),
    });

    this.saveAsk$.pipe(
      exhaustMap(() => {
        const editor = this.editor$.value;
        if (!editor.draft || editor.saving) return EMPTY;
        const problem = refusal(editor.draft);
        if (problem) {
          this.editor$.next({ ...editor, problem });
          return EMPTY;
        }
        const draft = editor.draft;
        this.editor$.next({ ...editor, saving: true, problem: null });
        return this.api.saveConfiguration(toConfiguration(draft)).pipe(
          map((configuration) => ({ configuration, failure: null as unknown })),
          catchError((failure: unknown) => of({ configuration: null as Configuration | null, failure })),
        );
      }),
      takeUntilDestroyed(),
    ).subscribe((result) => {
      if (result.configuration) {
        this.editor$.next({
          draft: fromConfiguration(result.configuration),
          saved: result.configuration,
          problem: null,
          saving: false,
          loadProblem: null,
        });
        return;
      }
      this.editor$.next({
        ...this.editor$.value,
        saving: false,
        problem: refusalText(result.failure, 'Could not save the settings.'),
      });
    });
  }

  save(): void {
    this.saveAsk$.next();
  }

  discard(): void {
    const saved = this.editor$.value.saved;
    if (!saved || this.editor$.value.saving) return;
    this.editor$.next({ draft: fromConfiguration(saved), saved, problem: null, saving: false, loadProblem: null });
  }

  addWorkflow(): void {
    this.patch((draft) => withWorkflow(draft, `new-${this.nextKey++}`));
  }

  removeWorkflow(key: string): void {
    this.patch((draft) => withoutWorkflow(draft, key));
  }

  rename(key: string, event: Event): void {
    this.patch((draft) => renameWorkflow(draft, key, valueOf(event)));
  }

  describe(key: string, event: Event): void {
    this.patch((draft) => describeWorkflow(draft, key, valueOf(event)));
  }

  toggleStage(key: string, stage: Stage): void {
    this.patch((draft) => toggleWorkflowStage(draft, key, stage));
  }

  chooseWorkflow(event: Event): void {
    this.patch((draft) => chooseDefault(draft, valueOf(event)));
  }

  setRoundCount(event: Event): void {
    this.patch((draft) => setRounds(draft, valueOf(event)));
  }

  label(stage: Stage, event: Event): void {
    this.patch((draft) => setLabel(draft, stage, valueOf(event)));
  }

  chooseMode(stage: Stage, event: Event): void {
    this.patch((draft) => setMode(draft, stage, valueOf(event) as ProviderMode));
  }

  chooseAgent(stage: Stage, event: Event): void {
    this.patch((draft) => setAgent(draft, stage, valueOf(event)));
  }

  toggleProvider(stage: Stage, agent: AgentName): void {
    this.patch((draft) => toggleAmong(draft, stage, agent));
  }

  chooseStrength(stage: Stage, event: Event): void {
    this.patch((draft) => setStrength(draft, stage, valueOf(event) as Strength));
  }

  model(agent: AgentName, strength: Strength, event: Event): void {
    this.patch((draft) => setModel(draft, agent, strength, valueOf(event)));
  }

  editorCommand(event: Event): void {
    this.patch((draft) => setEditor(draft, valueOf(event)));
  }

  terminalCommand(event: Event): void {
    this.patch((draft) => setTerminal(draft, valueOf(event)));
  }

  private patch(change: (draft: SettingsDraft) => SettingsDraft): void {
    const editor = this.editor$.value;
    if (!editor.draft || editor.saving) return;
    this.editor$.next({ ...editor, draft: change(editor.draft) });
  }
}

function valueOf(event: Event): string {
  const target = event.target;
  return target instanceof HTMLInputElement || target instanceof HTMLSelectElement ? target.value : '';
}
