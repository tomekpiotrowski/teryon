import { NO_WORKFLOWS } from '../workspace/workspace.model';
import { TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';
import { vi } from 'vitest';
import type { Event, ServiceMessage } from './protocol';
import { ADDRESS_FINDER, SERVICE_RESTARTER, ServiceConnection, SOCKET_FACTORY, type Socket } from './service-connection';
import { NO_REASON } from '../workspace/workspace.model';
import type { ServiceAddress } from './service-address';

/** A socket the test drives by hand, remembering what was sent. */
class FakeSocket implements Socket {
  onopen: (() => void) | null = null;
  onmessage: ((message: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  readonly sent: unknown[] = [];

  constructor(readonly address: string) {}

  send(text: string): void {
    this.sent.push(JSON.parse(text));
  }

  close(): void {
    this.onclose?.();
  }

  open(): void {
    this.onopen?.();
  }

  receive(message: ServiceMessage): void {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
}

const SNAPSHOT: ServiceMessage = { type: 'event', event: { type: 'snapshot', projects: [], threads: [], opens: { editor: false, terminal: false }, models: { claude: [], codex: [], grok: [] }, workflows: NO_WORKFLOWS, starting: null } };

/** What the address finder does on one attempt: find an address or nothing, fail, or never finish. */
type Finding = ServiceAddress | null | { readonly fails: unknown } | 'never';

/** What restarting the service does: give an address, or fail. */
type Restarting = ServiceAddress | { readonly fails: unknown };

function outcome<T>(finding: T | { readonly fails: unknown } | 'never'): Promise<T> {
  if (finding === 'never') return new Promise<T>(() => undefined);
  if (finding && typeof finding === 'object' && 'fails' in finding) return Promise.reject(finding.fails);
  return Promise.resolve(finding as T);
}

function setUp(
  findings: Finding[],
  restarting: Restarting = { fails: new Error('not in these tests') },
): { connection: ServiceConnection; sockets: FakeSocket[]; finds: () => number } {
  const sockets: FakeSocket[] = [];
  let found = 0;
  TestBed.configureTestingModule({
    providers: [
      { provide: SOCKET_FACTORY, useValue: (address: string) => sockets[sockets.push(new FakeSocket(address)) - 1] },
      { provide: ADDRESS_FINDER, useValue: () => outcome<ServiceAddress | null>(findings[Math.min(found++, findings.length - 1)]) },
      { provide: SERVICE_RESTARTER, useValue: () => outcome<ServiceAddress>(restarting) },
    ],
  });
  return { connection: TestBed.inject(ServiceConnection), sockets, finds: () => found };
}

const OLDER = { reason: 'A different version of Teryon is running.', restartable: true };

/** Lets promises waiting on the address finder settle. */
async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(0);
}

describe('ServiceConnection', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('connects to the address it was given and shows the token first', async () => {
    const { connection, sockets } = setUp([{ port: 7317, token: 'secret' }]);
    connection.start();
    await settle();
    sockets[0].open();

    expect(sockets[0].address).toBe('ws://127.0.0.1:7317/');
    expect(sockets[0].sent).toEqual([{ type: 'hello', token: 'secret' }]);
    expect(connection.state$.value.kind).toBe('connecting');
  });

  it('counts as connected once the first snapshot arrives, and passes events on', async () => {
    const { connection, sockets } = setUp([{ port: 7317, token: 'secret' }]);
    const events: Event[] = [];
    connection.events$.subscribe((event) => events.push(event));
    connection.start();
    await settle();
    sockets[0].open();
    sockets[0].receive(SNAPSHOT);

    expect(connection.state$.value.kind).toBe('connected');
    expect(events.map((event) => event.type)).toEqual(['snapshot']);
  });

  it('sends a request once connected and answers it with the reply of the same number', async () => {
    const { connection, sockets } = setUp([{ port: 7317, token: 'secret' }]);
    const answer = firstValueFrom(connection.request({ type: 'stop', thread_id: 't' }));
    await settle();
    sockets[0].open();
    expect(sockets[0].sent.length).toBe(1);

    sockets[0].receive(SNAPSHOT);
    expect(sockets[0].sent[1]).toEqual({ type: 'call', id: 1, request: { type: 'stop', thread_id: 't' } });
    sockets[0].receive({ type: 'reply', id: 99, outcome: { type: 'failed', message: 'not this one' } });
    sockets[0].receive({ type: 'reply', id: 1, outcome: { type: 'done', answer: { type: 'nothing' } } });

    await expect(answer).resolves.toEqual({ type: 'nothing' });
  });

  it('turns a refusal into an error carrying the message for the person', async () => {
    const { connection, sockets } = setUp([{ port: 7317, token: 'secret' }]);
    connection.start();
    await settle();
    sockets[0].receive(SNAPSHOT);
    const answer = firstValueFrom(connection.request({ type: 'mark_seen', thread_id: 't' }));
    sockets[0].receive({ type: 'reply', id: 1, outcome: { type: 'failed', message: 'There is no such thread.' } });

    await expect(answer).rejects.toMatchObject({ name: 'ServiceRefusal', message: 'There is no such thread.' });
  });

  it('fails a request whose connection drops, then connects again with a fresh address', async () => {
    const { connection, sockets } = setUp([
      { port: 7317, token: 'first' },
      { port: 7318, token: 'second' },
    ]);
    connection.start();
    await settle();
    sockets[0].receive(SNAPSHOT);
    const answer = firstValueFrom(connection.request({ type: 'stop', thread_id: 't' }));
    sockets[0].close();

    await expect(answer).rejects.toMatchObject({ message: 'Lost the connection to Teryon.' });
    expect(connection.state$.value.kind).toBe('connecting');
    await vi.advanceTimersByTimeAsync(300);
    sockets[1].open();
    expect(sockets[1].address).toBe('ws://127.0.0.1:7318/');
    expect(sockets[1].sent).toEqual([{ type: 'hello', token: 'second' }]);
  });

  it('says so when this window does not know where the service is, and keeps looking', async () => {
    const { connection, sockets } = setUp([null, { port: 7317, token: 'secret' }]);
    connection.start();
    await settle();

    expect(connection.state$.value.kind).toBe('no_address');
    expect(sockets.length).toBe(0);
    await vi.advanceTimersByTimeAsync(300);
    expect(sockets.length).toBe(1);
  });

  it('ignores messages that are not text or not JSON', async () => {
    const { connection, sockets } = setUp([{ port: 7317, token: 'secret' }]);
    connection.start();
    await settle();
    sockets[0].onmessage?.({ data: new ArrayBuffer(2) });
    sockets[0].onmessage?.({ data: 'not json' });

    expect(connection.state$.value.kind).toBe('connecting');
  });
  it('says why when the service cannot be reached, and whether restarting it would help', async () => {
    const { connection, sockets } = setUp([{ fails: OLDER }]);
    connection.start();
    await settle();

    expect(connection.state$.value).toEqual({ kind: 'unreachable', ...OLDER });
    expect(sockets.length).toBe(0);
  });

  it('says it cannot reach Teryon when the failure has no reason meant for the person', async () => {
    const { connection } = setUp([{ fails: new Error('invoke failed') }]);
    connection.start();
    await settle();

    expect(connection.state$.value).toEqual({ kind: 'unreachable', reason: NO_REASON, restartable: false });
  });

  it('keeps the reason on screen while trying again, and connects once the service can be reached', async () => {
    const { connection, sockets, finds } = setUp([{ fails: OLDER }, 'never']);
    connection.start();
    await settle();
    await vi.advanceTimersByTimeAsync(300);

    expect(finds()).toBe(2);
    expect(connection.state$.value).toEqual({ kind: 'unreachable', ...OLDER });
    expect(sockets.length).toBe(0);
  });

  it('connects once a later attempt finds the service', async () => {
    const { connection, sockets } = setUp([{ fails: OLDER }, { port: 7317, token: 'secret' }]);
    connection.start();
    await settle();
    await vi.advanceTimersByTimeAsync(300);
    sockets[0].receive(SNAPSHOT);

    expect(connection.state$.value.kind).toBe('connected');
  });

  it('restarts the service when asked and connects to the new one', async () => {
    const { connection, sockets } = setUp([{ fails: OLDER }], { port: 7400, token: 'fresh' });
    connection.start();
    await settle();
    const states: string[] = [];
    connection.state$.subscribe((state) => states.push(state.kind));
    const restarted = connection.restart();

    expect(connection.state$.value.kind).toBe('restarting');
    await restarted;
    sockets[0].open();
    sockets[0].receive(SNAPSHOT);

    expect(sockets[0].address).toBe('ws://127.0.0.1:7400/');
    expect(sockets[0].sent).toEqual([{ type: 'hello', token: 'fresh' }]);
    expect(states).toEqual(['unreachable', 'restarting', 'connecting', 'connected']);
  });

  it('does not try again on its own while restarting', async () => {
    const { connection, sockets, finds } = setUp([{ fails: OLDER }], { port: 7400, token: 'fresh' });
    connection.start();
    await settle();
    await connection.restart();
    await vi.advanceTimersByTimeAsync(10000);

    expect(finds()).toBe(1);
    expect(sockets.length).toBe(1);
  });

  it('ignores an attempt to connect that a restart overtook', async () => {
    let finish: (address: ServiceAddress) => void = () => undefined;
    const sockets: FakeSocket[] = [];
    TestBed.configureTestingModule({
      providers: [
        { provide: SOCKET_FACTORY, useValue: (address: string) => sockets[sockets.push(new FakeSocket(address)) - 1] },
        { provide: ADDRESS_FINDER, useValue: () => new Promise<ServiceAddress>((resolve) => (finish = resolve)) },
        { provide: SERVICE_RESTARTER, useValue: async () => ({ port: 7400, token: 'fresh' }) },
      ],
    });
    const connection = TestBed.inject(ServiceConnection);
    connection.start();
    await settle();
    await connection.restart();
    finish({ port: 7317, token: 'stale' });
    await settle();

    expect(sockets.map((socket) => socket.address)).toEqual(['ws://127.0.0.1:7400/']);
  });

  it('says why when restarting fails, and keeps trying', async () => {
    const failure = { reason: 'Teryon is taking long to stop.', restartable: false };
    const { connection, finds } = setUp([{ fails: OLDER }], { fails: failure });
    connection.start();
    await settle();
    await connection.restart();

    expect(connection.state$.value).toEqual({ kind: 'unreachable', ...failure });
    await vi.advanceTimersByTimeAsync(600);
    expect(finds()).toBe(2);
  });
});
