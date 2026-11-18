import { TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';
import { vi } from 'vitest';
import type { Event, ServiceMessage } from './protocol';
import { ADDRESS_FINDER, ServiceConnection, SOCKET_FACTORY, type Socket } from './service-connection';
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

const SNAPSHOT: ServiceMessage = { type: 'event', event: { type: 'snapshot', projects: [], threads: [], opens: { editor: false, terminal: false } } };

function setUp(addresses: (ServiceAddress | null)[]): { connection: ServiceConnection; sockets: FakeSocket[] } {
  const sockets: FakeSocket[] = [];
  let found = 0;
  TestBed.configureTestingModule({
    providers: [
      { provide: SOCKET_FACTORY, useValue: (address: string) => sockets[sockets.push(new FakeSocket(address)) - 1] },
      { provide: ADDRESS_FINDER, useValue: async () => addresses[Math.min(found++, addresses.length - 1)] },
    ],
  });
  return { connection: TestBed.inject(ServiceConnection), sockets };
}

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
    expect(connection.state$.value).toBe('connecting');
  });

  it('counts as connected once the first snapshot arrives, and passes events on', async () => {
    const { connection, sockets } = setUp([{ port: 7317, token: 'secret' }]);
    const events: Event[] = [];
    connection.events$.subscribe((event) => events.push(event));
    connection.start();
    await settle();
    sockets[0].open();
    sockets[0].receive(SNAPSHOT);

    expect(connection.state$.value).toBe('connected');
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
    expect(connection.state$.value).toBe('connecting');
    await vi.advanceTimersByTimeAsync(300);
    sockets[1].open();
    expect(sockets[1].address).toBe('ws://127.0.0.1:7318/');
    expect(sockets[1].sent).toEqual([{ type: 'hello', token: 'second' }]);
  });

  it('says so when this window does not know where the service is, and keeps looking', async () => {
    const { connection, sockets } = setUp([null, { port: 7317, token: 'secret' }]);
    connection.start();
    await settle();

    expect(connection.state$.value).toBe('no_address');
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

    expect(connection.state$.value).toBe('connecting');
  });
});
