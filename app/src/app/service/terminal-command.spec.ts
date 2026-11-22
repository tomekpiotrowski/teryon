import { vi } from 'vitest';
import { TestBed } from '@angular/core/testing';
import { TerminalCommand, TERMINAL_COMMAND_SETUP, type SetupRequest, type SetupReport } from './terminal-command';

const READY: SetupReport = { pending: false, notices: [] };
const CONFLICT: SetupReport = { pending: false, notices: [{ kind: 'conflict', message: 'Already installed', detail: null, replacement: 'seen-file' }] };

function setup() {
  const calls: SetupRequest[] = [];
  const pending: { resolve: (report: SetupReport) => void; reject: (reason: unknown) => void }[] = [];
  TestBed.configureTestingModule({ providers: [{
    provide: TERMINAL_COMMAND_SETUP,
    useValue: (request: SetupRequest) => {
      calls.push(request);
      return new Promise<SetupReport>((resolve, reject) => pending.push({ resolve, reject }));
    },
  }] });
  return { service: TestBed.inject(TerminalCommand), calls, pending };
}

describe('Terminal command setup', () => {
  it('reads the startup result only once', async () => {
    const { service, calls, pending } = setup();
    service.start();
    service.start();
    expect(calls).toEqual([{ kind: 'status' }]);
    expect(service.state$.value.busy).toBe(true);
    pending[0].resolve(READY);
    await Promise.resolve();
    expect(service.state$.value).toEqual({ report: READY, busy: false, dismissed: false });
  });

  it('requires the conflict shown to the person before replacing anything', async () => {
    const { service, calls, pending } = setup();
    service.start();
    pending[0].resolve(CONFLICT);
    await Promise.resolve();
    service.replace('different-file');
    expect(calls).toHaveLength(1);
    service.replace('seen-file');
    service.retry();
    expect(calls).toEqual([{ kind: 'status' }, { kind: 'install', replacement: 'seen-file' }]);
    pending[1].resolve(READY);
    await Promise.resolve();
    expect(service.state$.value.report).toEqual(READY);
  });

  it('retries without replacement permission and keeps failures separate from connection state', async () => {
    const { service, calls, pending } = setup();
    service.start();
    pending[0].reject(new Error('disk is full'));
    await Promise.resolve();
    expect(service.state$.value.report.notices[0].message).toContain('disk is full');
    expect(service.state$.value.busy).toBe(false);
    service.retry();
    expect(calls[1]).toEqual({ kind: 'install', replacement: null });
    pending[1].resolve(READY);
    await Promise.resolve();
    expect(service.state$.value.report).toEqual(READY);
  });

  it('keeps the notice dismissed when a pending check finishes', async () => {
    const { service, pending } = setup();
    service.start();
    service.dismiss();
    pending[0].resolve(CONFLICT);
    await Promise.resolve();
    expect(service.state$.value.dismissed).toBe(true);
  });

  it.each(['failed', undefined])('handles an unstructured rejection: %s', async (reason) => {
    const { service, pending } = setup();
    service.start();
    pending[0].reject(reason);
    await Promise.resolve();
    expect(service.state$.value.report.notices[0].kind).toBe('failed');
    expect(service.state$.value.report.notices[0].message).toBeTruthy();
  });
});

describe('Background setup completion', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('polls a pending startup without starting another installation', async () => {
    const { service, calls, pending } = setup();
    service.start();
    pending[0].resolve({ pending: true, notices: [] });
    await Promise.resolve();
    service.retry();
    expect(service.state$.value.busy).toBe(true);
    await vi.advanceTimersByTimeAsync(100);
    expect(calls).toEqual([{ kind: 'status' }, { kind: 'status' }]);
    pending[1].resolve(CONFLICT);
    await Promise.resolve();
    expect(service.state$.value.report).toEqual(CONFLICT);
    expect(service.state$.value.busy).toBe(false);
    await vi.advanceTimersByTimeAsync(1000);
    expect(calls).toHaveLength(2);
  });

  it('cancels pending polling when the app is destroyed', async () => {
    const { service, calls, pending } = setup();
    service.start();
    pending[0].resolve({ pending: true, notices: [] });
    await Promise.resolve();
    service.ngOnDestroy();
    await vi.advanceTimersByTimeAsync(1000);
    expect(calls).toHaveLength(1);
  });

  it('does not offer an installation retry for shell environment changes', async () => {
    const { service, pending } = setup();
    let retryable = true;
    const subscription = service.retryable$.subscribe(value => retryable = value);
    service.start();
    pending[0].resolve({ pending: false, notices: [{ kind: 'path', message: 'Restart the app', detail: null, replacement: null }] });
    await Promise.resolve();
    expect(retryable).toBe(false);
    subscription.unsubscribe();
  });

  it('does not offer an installation retry when only the setup record could not be saved', async () => {
    const { service, pending } = setup();
    let retryable = true;
    const subscription = service.retryable$.subscribe(value => retryable = value);
    service.start();
    pending[0].resolve({ pending: false, notices: [{ kind: 'diagnostic', message: 'The terminal command setup finished, but its record could not be saved: Permission denied', detail: null, replacement: null }] });
    await Promise.resolve();
    expect(retryable).toBe(false);
    expect(service.state$.value.report.notices[0].kind).toBe('diagnostic');
    subscription.unsubscribe();
  });

  it('still offers a retry when installation failed and the record could not be saved', async () => {
    const { service, pending } = setup();
    let retryable = false;
    const subscription = service.retryable$.subscribe(value => retryable = value);
    service.start();
    pending[0].resolve({ pending: false, notices: [
      { kind: 'failed', message: 'Could not copy the bundled command', detail: null, replacement: null },
      { kind: 'diagnostic', message: 'Could not save the setup record: Permission denied', detail: null, replacement: null },
    ] });
    await Promise.resolve();
    expect(retryable).toBe(true);
    subscription.unsubscribe();
  });
});
