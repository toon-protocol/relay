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
    socket.on('message', (data) =>
      this.frames.push(JSON.parse(String(data)) as Frame)
    );
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
      // A relay at its cap may accept the upgrade and then close; both resolve.
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
