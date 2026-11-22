import { CommonModule } from '@angular/common';
import { Component, inject, NgModule, OnInit } from '@angular/core';
import { TerminalCommand } from '../service/terminal-command';

/** Setup notices stay available even when the local service cannot connect. */
@Component({
  selector: 'app-terminal-command',
  standalone: false,
  template: `
    @if (setup.state$ | async; as state) {
      @if (!state.dismissed && state.report.notices.length) {
        <aside aria-label="Terminal command setup" aria-live="polite">
          <strong>Terminal command</strong>
          @for (notice of state.report.notices; track $index) {
            <p>{{ notice.message }}</p>
            @if (notice.detail) { <pre><code>{{ notice.detail }}</code></pre> }
            @if (notice.replacement; as replacement) {
              <button type="button" [disabled]="state.busy" (click)="setup.replace(replacement)">{{ notice.kind === 'repair' ? 'Use this AppImage’s version' : 'Replace existing command' }}</button>
            }
          }
          <footer>
            @if (setup.retryable$ | async) {
              <button type="button" [disabled]="state.busy" (click)="setup.retry()">{{ state.busy ? 'Installing…' : 'Retry installation' }}</button>
            }
            <button type="button" (click)="setup.dismiss()">Dismiss</button>
          </footer>
        </aside>
      }
    }
  `,
  styles: `
    aside { position: fixed; right: 16px; top: 16px; z-index: 10000; width: min(440px, calc(100vw - 32px)); max-height: 70vh; overflow: auto; padding: 20px; border: 1px solid var(--color-neutral-400); border-radius: 8px; background: var(--color-surface); color: var(--color-text); box-shadow: 0 4px 24px #0002; font-size: 14px; }
    p { margin: 12px 0; overflow-wrap: anywhere; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; padding: 10px; background: #8881; user-select: text; }
    footer { display: flex; justify-content: flex-end; gap: 12px; margin-top: 16px; }
    button { padding: 10px 12px; border: 1px solid currentColor; border-radius: 4px; background: transparent; color: inherit; cursor: pointer; }
    button:disabled { opacity: .5; cursor: wait; }
  `,
})
export class TerminalCommandComponent implements OnInit {
  readonly setup = inject(TerminalCommand);

  ngOnInit(): void {
    this.setup.start();
  }
}

@NgModule({
  declarations: [TerminalCommandComponent],
  imports: [CommonModule],
  exports: [TerminalCommandComponent],
})
export class TerminalCommandModule {}
