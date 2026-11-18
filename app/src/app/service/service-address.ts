/**
 * Where the local service listens, and its token.
 *
 * Every program that talks to the service must show the token, which the
 * service writes to `service.json`, readable only by the person running it.
 * Inside the desktop app the window asks Tauri, which reads that file. In a
 * browser, used for development and tests, the address printed by
 * `teryon service --print-address` carries the port and the token after `#`;
 * they are kept for the browser tab and removed from the address bar.
 */

export interface ServiceAddress {
  readonly port: number;
  readonly token: string;
}

/** Where the browser tab keeps the address once it has been read from the address bar. */
export const ADDRESS_KEY = 'teryon-service';

/** Reads `#port=…&token=…`. Nothing when either is missing or the port is not a number. */
export function addressFromFragment(fragment: string): ServiceAddress | null {
  const parts = new URLSearchParams(fragment.replace(/^#/, ''));
  const port = Number(parts.get('port'));
  const token = parts.get('token');
  if (!token || !Number.isInteger(port) || port <= 0 || port > 65535) return null;
  return { port, token };
}

/** Reads an address kept earlier. Nothing when there is none or it is damaged. */
export function addressFromStorage(text: string | null): ServiceAddress | null {
  if (!text) return null;
  try {
    const kept = JSON.parse(text) as Partial<ServiceAddress>;
    return typeof kept.port === 'number' && typeof kept.token === 'string' ? { port: kept.port, token: kept.token } : null;
  } catch {
    return null;
  }
}
