import { afterNextRender, ChangeDetectionStrategy, Component, ElementRef, EventEmitter, HostListener, inject, Injector, Input, type OnChanges, Output, ViewChild } from '@angular/core';
import { BehaviorSubject, combineLatest, map } from 'rxjs';
import {
  AGENTS, type AgentName, agentLabel, type Choice, choiceLabel, defaultModelOption, type Effort, effortLabel,
  FIRST_NEW_THREADS, type ModelLists, type ModelOption, type NewThreads, NO_MODELS, rememberedChoice, resolveChoice,
} from './workspace.model';

/** The open list, keyed by identifiers so it follows catalog updates. */
type Step = { level: 'agent' } | { level: 'model'; agent: AgentName } | { level: 'effort'; agent: AgentName; model: string };

/** A row inside the active list. */
interface Option {
  readonly id: string;
  readonly label: string;
  readonly next: boolean;
  readonly selected: boolean;
  /** Whether a line sets it apart from the options after it. */
  readonly separated: boolean;
  readonly choose: () => void;
}

/** The identifier of the Automatic option, which no agent shares. */
const AUTOMATIC = 'automatic';

/**
 * Chooses an installed provider, its versioned model, and an effort in one menu.
 *
 * On Home it also offers Automatic, which lets the default workflow's first
 * stage choose new threads' agent. While Automatic is on, the button reads
 * `Auto · ` and the pick the next thread starts with.
 */
@Component({
  selector: 'app-agent-chooser',
  templateUrl: './agent-chooser.component.html',
  styleUrl: './agent-chooser.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class AgentChooserComponent implements OnChanges {
  @Input() installed: readonly string[] = [];
  @Input() models: ModelLists = NO_MODELS;
  @Input() choice: Choice | null = null;
  @Input() remembered: NewThreads = FIRST_NEW_THREADS;
  @Input() allowedAgents: readonly AgentName[] = AGENTS;
  @Input() placement: 'down' | 'up' = 'down';
  @Input() disabled = false;
  /** Whether Automatic is offered above the agents. */
  @Input() allowAutomatic = false;
  /** Whether Automatic is on, so `choice` is the first stage's pick rather than the person's. */
  @Input() automatic = false;
  /** Why the first stage can pick no agent, shown on hover while Automatic is on. */
  @Input() unavailableReason: string | null = null;
  @Output() readonly chosen = new EventEmitter<Choice>();
  /** The person chose Automatic. */
  @Output() readonly automaticChosen = new EventEmitter<void>();

  @ViewChild('trigger') private trigger?: ElementRef<HTMLButtonElement>;
  @ViewChild('list') private list?: ElementRef<HTMLElement>;
  private readonly injector = inject(Injector);
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  private readonly step$ = new BehaviorSubject<Step | null>(null);
  private readonly inputsChanged$ = new BehaviorSubject<void>(undefined);

  readonly view$ = combineLatest([this.step$, this.inputsChanged$]).pipe(map(([step]) => {
    const agents = AGENTS.filter((agent) => this.installed.includes(agent) && this.allowedAgents.includes(agent));
    const active = this.choice && resolveChoice(this.choice, this.models);
    const level = step?.level ?? 'agent';
    const model = step?.level === 'effort' ? this.models[step.agent].find((model) => model.id === step.model) : undefined;
    const label = this.label();
    return {
      open: step !== null,
      disabled: this.disabled || !agents.length,
      label,
      title: this.automatic && this.unavailableReason ? this.unavailableReason : label,
      listLabel: level === 'effort' ? 'Effort' : level === 'model' ? 'Model' : 'Agent',
      back: step?.level === 'model' ? 'Agents' : model?.label,
      options: this.options(step, agents, active, model),
    };
  }));

  /** Keeps the open level usable if discovery or running state changes. */
  ngOnChanges(): void {
    const step = this.step$.value;
    if (this.disabled && step) this.close(false);
    else if (step && step.level !== 'agent') {
      if (!this.installed.includes(step.agent) || !this.allowedAgents.includes(step.agent)) this.show({ level: 'agent' });
      else if (step.level === 'effort' && !this.models[step.agent].some((model) => model.id === step.model && model.efforts.length)) {
        this.show({ level: 'model', agent: step.agent });
      }
    }
    this.inputsChanged$.next();
  }

  /** Opens at the provider list, or closes an open menu. */
  toggle(): void {
    if (this.step$.value) this.close(true);
    else this.show({ level: 'agent' });
  }

  /** Returns to the previous list. */
  back(): void {
    const step = this.step$.value;
    if (step?.level === 'effort') this.show({ level: 'model', agent: step.agent }, step.model);
    else if (step?.level === 'model') this.show({ level: 'agent' }, step.agent);
  }

  /** Handles navigation while allowing native Enter and Space button activation. */
  key(event: KeyboardEvent): void {
    if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    const step = this.step$.value;
    const options = Array.from(this.list?.nativeElement.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? []);
    const current = options.indexOf(event.target as HTMLButtonElement);
    switch (event.key) {
      case 'Escape':
        if (!step) return;
        this.close(true);
        break;
      case 'Tab':
        this.close(false);
        return;
      case 'ArrowLeft':
      case 'Backspace':
        if (!step || step.level === 'agent') return;
        this.back();
        break;
      case 'ArrowRight':
        if (current < 0 || options[current].dataset['next'] !== 'true') return;
        options[current].click();
        break;
      case 'ArrowUp':
      case 'ArrowDown': {
        const direction = event.key === 'ArrowDown' ? 1 : -1;
        if (!step) this.show({ level: 'agent' });
        else if (options.length) {
          const next = current < 0 ? (direction > 0 ? 0 : options.length - 1) : (current + direction + options.length) % options.length;
          options[next].focus();
        }
        break;
      }
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  /** Dismisses an open menu when the person clicks outside it. */
  @HostListener('document:click', ['$event'])
  dismiss(event: MouseEvent): void {
    if (!this.step$.value || !(event.target instanceof Node)) return;
    if (!this.host.nativeElement.contains(event.target)) {
      this.close(this.list?.nativeElement.contains(document.activeElement) ?? false);
    }
  }

  /** The button's words: the choice, with `Auto · ` in front while the first stage chooses. */
  private label(): string {
    const choice = this.choice ? choiceLabel(this.choice, this.models) : null;
    if (!this.allowAutomatic || !this.automatic) return choice ?? 'No agent';
    if (choice) return `Auto · ${choice}`;
    return this.unavailableReason ? 'Auto · Unavailable' : 'Auto';
  }

  /** Projects only the active list, including the current choice or catalog default. */
  private options(step: Step | null, agents: readonly AgentName[], active: Choice | null, model?: ModelOption): Option[] {
    if (!step || step.level === 'agent') {
      const automatic = this.allowAutomatic && this.automatic;
      const first: Option[] = this.allowAutomatic ? [{
        id: AUTOMATIC,
        label: 'Automatic',
        next: false,
        selected: automatic,
        separated: true,
        choose: () => this.chooseAutomatic(),
      }] : [];
      return [...first, ...agents.map((agent) => ({
        id: agent,
        label: agentLabel(agent),
        next: this.models[agent].length > 0,
        selected: !automatic && agent === active?.agent,
        separated: false,
        choose: () => this.models[agent].length
          ? this.show({ level: 'model', agent })
          : this.pick({ agent, model: null, effort: null }),
      }))];
    }
    const agent = step.agent;
    if (step.level === 'model') {
      const options = this.models[agent];
      const preferred = [active?.agent === agent ? active.model : null, this.remembered.models[agent]]
        .find((id) => options.some((option) => option.id === id)) ?? defaultModelOption(agent, this.models)?.id;
      return options.map((option) => ({
        id: option.id,
        label: option.label,
        next: option.efforts.length > 0,
        selected: option.id === preferred,
        separated: false,
        choose: () => option.efforts.length
          ? this.show({ level: 'effort', agent, model: option.id })
          : this.pick({ agent, model: option.id, effort: null }),
      }));
    }
    if (!model) return [];
    const actual = this.choice?.agent === agent && active?.model === model.id ? active.effort : null;
    const preferred = [actual, rememberedChoice(this.remembered, agent, model.id).effort, model.default_effort]
      .find((effort): effort is Effort => effort !== null && model.efforts.includes(effort));
    return model.efforts.map((effort) => ({
      id: effort,
      label: effortLabel(effort),
      next: false,
      selected: effort === preferred,
      separated: false,
      choose: () => this.pick({ agent, model: model.id, effort }),
    }));
  }

  /** Moves focus to the selected option once the new list has been drawn. */
  private show(step: Step, focusId?: string): void {
    this.step$.next(step);
    afterNextRender(() => {
      if (this.step$.value !== step) return;
      const options = Array.from(this.list?.nativeElement.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? []);
      (options.find((option) => option.dataset['id'] === focusId)
        ?? options.find((option) => option.getAttribute('aria-selected') === 'true')
        ?? options[0])?.focus();
    }, { injector: this.injector });
  }

  /** Reports a complete choice and returns keyboard focus to the trigger. */
  private pick(choice: Choice): void {
    this.close(true);
    this.chosen.emit(choice);
  }

  /** Goes back to letting the first stage choose, and returns keyboard focus to the trigger. */
  private chooseAutomatic(): void {
    this.close(true);
    this.automaticChosen.emit();
  }

  /** Dismisses the menu and optionally returns focus to the trigger. */
  private close(restore: boolean): void {
    this.step$.next(null);
    if (restore) this.trigger?.nativeElement.focus();
  }
}
