import { Component, Injectable, InjectionToken, signal } from '@angular/core';
import { of } from 'rxjs';

export const ERNO_CONFIG = new InjectionToken('ERNO_CONFIG', {
  factory: () => ({ baseUrl: 'http://localhost:3000', wsUrl: 'ws://localhost:3000' }),
});

export interface SyncDeltaItem {
  entity: string;
  id: string;
  sync_seq: number;
  deleted: boolean;
  data: unknown;
}

export function registerDevtoolsDatabase(_database: unknown): void {}

@Injectable({ providedIn: 'root' })
export class ErnoAuthService {
  readonly authState = signal<'authenticated'>('authenticated');
  readonly currentUser = signal({ id: 'test-user', email: 'test@example.com' });
  readonly authState$ = of('authenticated' as const);
  readonly currentUser$ = of({ id: 'test-user', email: 'test@example.com' });

  login(_email: string, _password: string) { return of(undefined); }
  register(_email: string, _password: string) { return of(undefined); }
  requestPasswordReset(_email: string) { return of(undefined); }
  confirmPasswordReset(_token: string, _password: string) { return of(undefined); }
  verifyEmail(_token: string) { return of(undefined); }
  logout() { return of(undefined); }
  deleteAccount() { return of(undefined); }
}

@Injectable({ providedIn: 'root' })
export class ErnoSyncService {
  register(): void {}
  start() { return Promise.resolve(); }
  resetCursor() { return Promise.resolve(); }
}

@Injectable({ providedIn: 'root' })
export class ErnoAlertsService {
  error(): void {}
  success(): void {}
}

// Mirrors the third-party component selector replaced by this test-only component.
// eslint-disable-next-line @angular-eslint/component-selector
@Component({ selector: 'erno-devtools', template: '', standalone: true })
export class ErnoDevtoolsComponent {}
