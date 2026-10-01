import { finalizeEvent, generateSecretKey } from 'nostr-tools/pure';
import type { Event, EventTemplate } from 'nostr-tools';
import WebSocket from 'ws';

/** A relay-to-client frame, parsed from the wire. */
export type Frame = [string, ...unknown[]];

/**
 * A WebSocket client that records every frame so a test can wait for the one
 * it expects, or assert that none arrives.
 */
export class Client {
  readonly frames: Frame[] = [];
  closed: { code: number; reason: string } | undefined;
  private readonly socket: WebSocket;
  private cursor = 0;

  private constructor(socket: WebSocket) {
    this.socket = socket;
    socket.on('message', (data) => {
      const text = String(data);
      try {
        this.frames.push(JSON.parse(text) as Frame);
      } catch {
        // Keep a frame that is not JSON visible to `next`, rather than
        // throwing out of the socket's event handler.
        this.frames.push(['<unparsable>', text]);
      }
    });
    socket.on('close', (code, reason) => {
      this.closed = { code, reason: String(reason) };
    });
  }

  /** Open a connection and resolve once it is open (or reject if it errors). */
  static connect(url: string): Promise<Client> {
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(url);
      const client = new Client(socket);
      socket.once('open', () => resolve(client));
      socket.once('error', reject);
      // A relay at its cap may accept the upgrade and then close it, so this
      // resolves; `untilClosed` observes the close.
    });
  }

  send(message: unknown): void {
    this.socket.send(
      typeof message === 'string' ? message : JSON.stringify(message)
    );
  }

  /** Wait for the next unread frame matching `predicate`. */
  async next(
    predicate: (frame: Frame) => boolean,
    timeoutMs = 10_000
  ): Promise<Frame> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      while (this.cursor < this.frames.length) {
        const frame = this.frames[this.cursor++];
        if (frame && predicate(frame)) return frame;
      }
      await sleep(20);
    }
    throw new Error(
      `no matching frame within ${timeoutMs}ms; saw ${JSON.stringify(this.frames)}`
    );
  }

  /** Wait for the connection to close. */
  async untilClosed(
    timeoutMs = 10_000
  ): Promise<{ code: number; reason: string }> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.closed) return this.closed;
      await sleep(20);
    }
    throw new Error('connection never closed');
  }

  /** Send a REQ and return the events before EOSE, parsed as JSON. */
  async req(
    subId: string,
    ...filters: Record<string, unknown>[]
  ): Promise<Event[]> {
    this.send(['REQ', subId, ...filters]);
    const events: Event[] = [];
    for (;;) {
      const frame = await this.next((f) => f[1] === subId);
      if (frame[0] === 'EOSE') return events;
      if (frame[0] === 'EVENT') events.push(frame[2] as Event);
    }
  }

  /** Assert nothing further arrives for `ms`; returns whatever did. */
  async quiet(ms = 750): Promise<Frame[]> {
    await sleep(ms);
    const rest = this.frames.slice(this.cursor);
    this.cursor = this.frames.length;
    return rest;
  }

  close(): void {
    this.socket.close();
  }
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** A signed kind-1 (by default) event from `secretKey`. */
export function signed(
  secretKey: Uint8Array,
  template: Partial<EventTemplate> = {}
): Event {
  return finalizeEvent(
    {
      kind: 1,
      created_at: Math.floor(Date.now() / 1000),
      tags: [],
      content: 'conformance',
      ...template,
    },
    secretKey
  );
}

export { generateSecretKey };

/** POST an event to the write port as the connector would. */
export async function publish(writeUrl: string, event: Event): Promise<void> {
  const response = await fetch(`${writeUrl}/write`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ event }),
  });
  if (response.status !== 200) {
    throw new Error(
      `write refused: ${response.status} ${await response.text()}`
    );
  }
}

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
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error('no EOSE'));
    }, 10_000);
    ws.on('error', reject);
    ws.on('open', () => ws.send(JSON.stringify(['REQ', 'live', filter])));
    ws.on('message', (data) => {
      const message = JSON.parse(String(data)) as [string, ...unknown[]];
      if (message[0] === 'EVENT') {
        received.push(message[2] as Event);
        wake?.();
      }
      if (message[0] === 'EOSE') {
        clearTimeout(timer);
        resolve({
          async next(ms) {
            if (received.length === 0) {
              let silence: NodeJS.Timeout | undefined;
              await new Promise<void>((done) => {
                wake = done;
                silence = setTimeout(done, ms);
              });
              clearTimeout(silence);
            }
            return received.shift();
          },
          close: () => ws.close(),
        });
      }
    });
  });
}
