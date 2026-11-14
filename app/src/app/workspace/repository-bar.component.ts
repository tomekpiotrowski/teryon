import { ChangeDetectionStrategy, Component, EventEmitter, Input, Output } from '@angular/core';

/**
 * The bar above a project's files and history: the project, and the branch looked at.
 *
 * Both screens look at the same repository and start with the same bar, so
 * the bar is one component. It holds nothing itself: what it shows is handed
 * to it, and a choice of branch is handed back.
 */
@Component({
  selector: 'app-repository-bar',
  templateUrl: './repository-bar.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class RepositoryBarComponent {
  @Input({ required: true }) projectName = '';
  /** For example `2 agents live`. Nothing when no agent is running. */
  @Input() live = '';
  @Input() branches: readonly string[] = [];
  /** The branch shown. Nothing when the project's directory is not on a branch. */
  @Input() branch: string | null = null;
  /** The short name of a commit shown in place of a branch. */
  @Input() commit = '';

  /** The branch the person picked. */
  @Output() readonly branchChosen = new EventEmitter<string | null>();

  choose(event: Event): void {
    this.branchChosen.emit((event.target as HTMLSelectElement).value || null);
  }
}
