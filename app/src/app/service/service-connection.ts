import { DOCUMENT } from '@angular/common';
import { inject, Injectable, InjectionToken } from '@angular/core';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { BehaviorSubject, filter, first, Observable, Subject, switchMap, throwError } from 'rxjs';
import type { Answer, ClientMessage, Event, Request, ServiceMessage } from './protocol';
import { type ConnectionState, REFUSAL } from '../workspace/workspace.model';
import { ADDRESS_KEY, addressFromFragment, addressFromStorage, type ServiceAddress } from './service-address';


/** A refusal from the service, with a message meant for the person. */
export class ServiceRefusal extends Error {
  override readonly name = REFUSAL;
}

/** The smallest part of a WebSocket the connection uses, so tests can stand in for one. */
export interface Socket {
  onopen: (() => void) | null;
  onmessage: ((message: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  send(text: string): void;
  close(): void;
}

/** Opens a socket to an address. */
export type SocketFactory = (address: string) => Socket;

export const SOCKET_FACTORY = new InjectionToken<SocketFactory>('teryon socket factory', {
  providedIn: 'root',
  factory: () => (address: string) => new WebSocket(address) as unknown as Socket,
});

/** Finds out where the service is. Nothing when this window cannot know. */
export type AddressFinder = () => Promise<ServiceAddress | null>;

export const ADDRESS_FINDER = new InjectionToken<AddressFinder>('teryon address finder', {
  providedIn: 'root',
  factory: () => {
    const window = inject(DOCUMENT).defaultView;
    return async () => {
      if (isTauri()) return invoke<ServiceAddress>('service_connection');
      const fromFragment = addressFromFragment(window?.location.hash ?? '');
      if (fromFragment) {
        window?.sessionStorage.setItem(ADDRESS_KEY, JSON.stringify(fromFragment));
        window?.history.replaceState(null, '', window.location.pathname + window.location.search);
        return fromFragment;
      }
      return addressFromStorage(window?.sessionStorage.getItem(ADDRESS_KEY) ?? null);
    };
  },
});

/** The longest wait between two attempts to connect, in milliseconds. */
const LONGEST_WAIT = 5000;

/**
 * The one connection to the local service.
 *
 * It shows the token first, then passes on every event and matches replies
 * to requests by number. When the connection drops it tries again, waiting
 * longer each time, and asks for the address again because the service may
 * have restarted on another port with another token.
 *
 * A request made while not connected waits until the connection is up.
 * A request whose connection drops before the reply fails.
 */
@Injectable({ providedIn: 'root' })
export class ServiceConnection {
  private readonly openSocket = inject(SOCKET_FACTORY);
  private readonly findAddress = inject(ADDRESS_FINDER);

  readonly state$ = new BehaviorSubject<ConnectionState>('connecting');
  private readonly eventsSubject = new Subject<Event>();
  /** Every event from the service, starting with a snapshot each time the connection is made. */
  readonly events$: Observable<Event> = this.eventsSubject.asObservable();

  private socket: Socket | null = null;
  private nextId = 1;
  private readonly waiting = new Map<number, { resolve: (answer: Answer) => void; reject: (problem: Error) => void }>();
  private started = false;
  private wait = 250;

  /** Connects, unless already connected or connecting. */
  start(): void {
    if (this.started) return;
    this.started = true;
    void this.connect();
  }

  /** Sends a request once connected and gives back its answer. */
  request(request: Request): Observable<Answer> {
    this.start();
    return this.state$.pipe(
      filter((state) => state === 'connected'),
      first(),
      switchMap(() => {
        const socket = this.socket;
        if (!socket) return throwError(() => new ServiceRefusal('Lost the connection to Teryon.'));
        const id = this.nextId++;
        const answer = new Promise<Answer>((resolve, reject) => this.waiting.set(id, { resolve, reject }));
        const message: ClientMessage = { type: 'call', id, request };
        socket.send(JSON.stringify(message));
        return answer;
      }),
    );
  }

  private async connect(): Promise<void> {
    this.state$.next('connecting');
    let address: ServiceAddress | null = null;
    try {
      address = await this.findAddress();
    } catch {
      address = null;
    }
    if (!address) {
      this.state$.next('no_address');
      this.retryLater();
      return;
    }

    const socket = this.openSocket(`ws://127.0.0.1:${address.port}/`);
    this.socket = socket;
    const token = address.token;
    socket.onopen = () => {
      const hello: ClientMessage = { type: 'hello', token };
      socket.send(JSON.stringify(hello));
    };
    socket.onmessage = ({ data }) => this.receive(data);
    socket.onclose = () => {
      if (this.socket !== socket) return;
      this.socket = null;
      for (const pending of this.waiting.values()) pending.reject(new ServiceRefusal('Lost the connection to Teryon.'));
      this.waiting.clear();
      this.state$.next('connecting');
      this.retryLater();
    };
  }

  private receive(data: unknown): void {
    if (typeof data !== 'string') return;
    let message: ServiceMessage;
    try {
      message = JSON.parse(data) as ServiceMessage;
    } catch {
      return;
    }
    if (message.type === 'event') {
      if (message.event.type === 'snapshot' && this.state$.value !== 'connected') {
        this.wait = 250;
        this.state$.next('connected');
      }
      this.eventsSubject.next(message.event);
      return;
    }
    const pending = this.waiting.get(message.id);
    if (!pending) return;
    this.waiting.delete(message.id);
    if (message.outcome.type === 'done') pending.resolve(message.outcome.answer);
    else pending.reject(new ServiceRefusal(message.outcome.message));
  }

  private retryLater(): void {
    const wait = this.wait;
    this.wait = Math.min(this.wait * 2, LONGEST_WAIT);
    setTimeout(() => void this.connect(), wait);
  }
}
