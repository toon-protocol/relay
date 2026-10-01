import { finalizeEvent, generateSecretKey } from 'nostr-tools/pure';
import type { Event } from 'nostr-tools';
import WebSocket from 'ws';

/** Sign a fresh event of `kind` with a throwaway key. */
export function signedEvent(kind: number, content = 'conformance'): Event {
  return finalizeEvent(
    { kind, created_at: Math.floor(Date.now() / 1000), tags: [], content },
    generateSecretKey()
  );
}

/** POST `body` (JSON-encoded unless a string) to `url`. */
export function post(
  url: string,
  body: unknown,
  headers: Record<string, string> = {}
): Promise<Response> {
  return fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...headers },
    body: typeof body === 'string' ? body : JSON.stringify(body),
  });
}

/** Send a REQ and collect events until EOSE. */
export function query(
  url: string,
  filter: Record<string, unknown>
): Promise<Event[]> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const events: Event[] = [];
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error('no EOSE'));
    }, 10_000);
    ws.on('error', reject);
    ws.on('open', () => ws.send(JSON.stringify(['REQ', 'conf', filter])));
    ws.on('message', (data) => {
      const message = JSON.parse(String(data)) as [string, ...unknown[]];
      if (message[0] === 'EVENT') events.push(message[2] as Event);
      if (message[0] === 'EOSE') {
        clearTimeout(timer);
        ws.close();
        resolve(events);
      }
    });
  });
}

export interface LiveSubscription {
  /** Resolves with the next live event, or `undefined` after `ms` of silence. */
  next(ms: number): Promise<Event | undefined>;
  close(): void;
}

/** Subscribe with `filter`, resolving once the relay has sent EOSE. */
export function subscribe(
  url: string,
  filter: Record<string, unknown>
): Promise<LiveSubscription> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const received: Event[] = [];
    let wake: (() => void) | undefined;
    ws.on('error', reject);
    ws.on('open', () => ws.send(JSON.stringify(['REQ', 'live', filter])));
    ws.on('message', (data) => {
      const message = JSON.parse(String(data)) as [string, ...unknown[]];
      if (message[0] === 'EVENT') {
        received.push(message[2] as Event);
        wake?.();
      }
      if (message[0] === 'EOSE') {
        resolve({
          async next(ms) {
            if (received.length === 0) {
              await new Promise<void>((done) => {
                wake = done;
                setTimeout(done, ms);
              });
            }
            return received.shift();
          },
          close: () => ws.close(),
        });
      }
    });
  });
}
