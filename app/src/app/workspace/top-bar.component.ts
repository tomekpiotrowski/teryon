import { ChangeDetectionStrategy, Component, EventEmitter, Input, Output } from '@angular/core';

/**
 * The bar at the top of the workspace: the menu, where the person is, what runs, and what needs them.
 *
 * Shows two counts and nothing more about the work: how many agents are
 * running, and how many threads need the person. The second leads Home,
 * where those threads are listed.
 */
@Component({
  selector: 'app-top-bar',
  templateUrl: './top-bar.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class TopBarComponent {
  /** The open thread's label. Empty with no thread open. */
  @Input() scope = '';
  /** The selected project, or `ALL PROJECTS` when there is none. */
  @Input({ required: true }) projectName!: string;
  /** How many agents are running. */
  @Input() running = 0;
  /** How many threads need the person. */
  @Input() waiting = 0;
  /** A leader key pressed, waiting for the key that finishes the command. Nothing when none is. */
  @Input() leader: string | null = null;

  @Output() readonly menuOpened = new EventEmitter<void>();
  @Output() readonly homeOpened = new EventEmitter<void>();
  @Output() readonly settingsOpened = new EventEmitter<void>();
}
