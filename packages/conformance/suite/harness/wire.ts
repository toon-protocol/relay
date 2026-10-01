import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { Event, EventTemplate } from 'nostr-tools';
import WebSocket from 'ws';
import type { RunningRelay } from './relay-container.js';

export const now = (): number => Math.floor(Date.now() / 1000);

/** Sign a template with `secretKey`; `created_at` defaults to now. */
export function sign(
  secretKey: Uint8Array,
  template: Partial<EventTemplate> & { kind: number }
): Event {
  return finalizeEvent(
    {
      created_at: now(),
      tags: [],
      content: '',
      ...template,
    },
    secretKey
  );
}

export const pubkeyOf = (secretKey: Uint8Array): string =>
  getPublicKey(secretKey);

/** POST an event to the write port. Resolves with the HTTP status. */
export async function publish(
  relay: RunningRelay,
  event: Event
): Promise<number> {
  const response = await fetch(`${relay.writeUrl}/write`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ event }),
  });
  await response.text();
  return response.status;
}

/** Publish and require a 200: the write itself is not what is under test. */
export async function publishOk(
  relay: RunningRelay,
  event: Event
): Promise<void> {
  const status = await publish(relay, event);
  if (status !== 200) throw new Error(`POST /write returned ${status}`);
}

/** Send a REQ and collect the stored events until EOSE. */
export function query(
  relay: RunningRelay,
  filter: Record<string, unknown>
): Promise<Event[]> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(relay.readWsUrl);
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

/** The ids stored for `filter`, sorted so sets compare with `toEqual`. */
export async function storedIds(
  relay: RunningRelay,
  filter: Record<string, unknown>
): Promise<string[]> {
  return (await query(relay, filter)).map((e) => e.id).sort();
}

export interface LiveSubscription {
  /** Events delivered after EOSE. */
  readonly delivered: Event[];
  close(): void;
}

/** Open a subscription, wait for its EOSE, and record what is delivered live. */
export function subscribe(
  relay: RunningRelay,
  filter: Record<string, unknown>
): Promise<LiveSubscription> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(relay.readWsUrl);
    const delivered: Event[] = [];
    let live = false;
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error('no EOSE'));
    }, 10_000);
    ws.on('error', reject);
    ws.on('open', () => ws.send(JSON.stringify(['REQ', 'live', filter])));
    ws.on('message', (data) => {
      const message = JSON.parse(String(data)) as [string, ...unknown[]];
      if (message[0] === 'EVENT' && live) delivered.push(message[2] as Event);
      if (message[0] === 'EOSE') {
        live = true;
        clearTimeout(timer);
        resolve({ delivered, close: () => ws.close() });
      }
    });
  });
}

/** Give live delivery time to arrive (or, for a negative, not to). */
export const settle = (ms = 1_000): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms));
