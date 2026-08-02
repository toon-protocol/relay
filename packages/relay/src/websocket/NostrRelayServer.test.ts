import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { WebSocket } from 'ws';
import { existsSync, unlinkSync } from 'fs';
import type { NostrEvent } from 'nostr-tools/pure';
import {
  NostrRelayServer,
  readOpenFilesSoftLimit,
} from './NostrRelayServer.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';
import { InMemoryEventStore, SqliteEventStore } from '../storage/index.js';

// Suppress console.log during tests
vi.spyOn(console, 'log').mockImplementation(() => undefined);
vi.spyOn(console, 'error').mockImplementation(() => undefined);

function createMockEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  return {
    id: 'event1',
    pubkey: 'pubkey1',
    created_at: 1000,
    kind: 1,
    tags: [],
    content: 'test content',
    sig: 'sig1',
    ...overrides,
  };
}

function waitForOpen(ws: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    if (ws.readyState === WebSocket.OPEN) {
      resolve();
      return;
    }
    ws.on('open', () => resolve());
    ws.on('error', reject);
  });
}

function waitForMessage(ws: WebSocket, timeoutMs = 5000): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error('Timeout waiting for message'));
    }, timeoutMs);

    ws.once('message', (data) => {
      clearTimeout(timeout);
      try {
        resolve(JSON.parse(data.toString()));
      } catch (err) {
        reject(err);
      }
    });
    ws.once('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });
}

function collectMessages(
  ws: WebSocket,
  count: number,
  timeoutMs = 5000
): Promise<unknown[]> {
  return new Promise((resolve, reject) => {
    const messages: unknown[] = [];
    const timeout = setTimeout(() => {
      reject(
        new Error(
          `Timeout waiting for messages. Got ${messages.length}, expected ${count}`
        )
      );
    }, timeoutMs);

    const handler = (data: Buffer) => {
      try {
        messages.push(JSON.parse(data.toString()));
        if (messages.length === count) {
          clearTimeout(timeout);
          ws.off('message', handler);
          resolve(messages);
        }
      } catch (err) {
        clearTimeout(timeout);
        reject(err);
      }
    };

    ws.on('message', handler);
    ws.once('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });
}

describe('NostrRelayServer', () => {
  let server: NostrRelayServer;
  let store: InMemoryEventStore;

  beforeEach(() => {
    store = new InMemoryEventStore();
  });

  afterEach(async () => {
    if (server) {
      await server.stop();
    }
  });

  it('should start on configured port', async () => {
    server = new NostrRelayServer({ port: 0 }, store);
    await server.start();

    const port = server.getPort();
    expect(port).toBeGreaterThan(0);
  });

  it('should return 0 for port when not started', () => {
    server = new NostrRelayServer({ port: 0 }, store);
    expect(server.getPort()).toBe(0);
  });

  it('should accept WebSocket connections', async () => {
    server = new NostrRelayServer({ port: 0 }, store);
    await server.start();

    const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
    await waitForOpen(ws);

    expect(ws.readyState).toBe(WebSocket.OPEN);
    expect(server.getClientCount()).toBe(1);

    ws.close();
  });

  it('should handle multiple client connections', async () => {
    server = new NostrRelayServer({ port: 0 }, store);
    await server.start();

    const ws1 = new WebSocket(`ws://localhost:${server.getPort()}`);
    const ws2 = new WebSocket(`ws://localhost:${server.getPort()}`);

    await Promise.all([waitForOpen(ws1), waitForOpen(ws2)]);

    expect(server.getClientCount()).toBe(2);

    ws1.close();
    ws2.close();
  });

  it('should stop cleanly and close connections', async () => {
    server = new NostrRelayServer({ port: 0 }, store);
    await server.start();

    const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
    await waitForOpen(ws);

    await server.stop();

    expect(server.getPort()).toBe(0);
    expect(server.getClientCount()).toBe(0);
  });

  describe('integration: REQ → EVENT → EOSE flow', () => {
    it('should send events and EOSE on REQ', async () => {
      const event = createMockEvent({ id: 'testevent123' });
      store.store(event);

      server = new NostrRelayServer({ port: 0 }, store);
      await server.start();

      const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws);

      // Set up message collection before sending REQ
      const messagesPromise = collectMessages(ws, 2);

      // Send REQ
      ws.send(JSON.stringify(['REQ', 'mysub', {}]));

      // Wait for both EVENT and EOSE
      const messages = await messagesPromise;
      // Canonical NIP-01: the event rides in the frame as a plain JSON object.
      expect(messages[0]).toEqual(['EVENT', 'mysub', event]);
      expect(messages[1]).toEqual(['EOSE', 'mysub']);

      ws.close();
    });

    it('should handle REQ with no matching events', async () => {
      server = new NostrRelayServer({ port: 0 }, store);
      await server.start();

      const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws);

      // Send REQ for kind that doesn't exist
      ws.send(JSON.stringify(['REQ', 'emptysub', { kinds: [9999] }]));

      // Should immediately get EOSE
      const eoseMessage = await waitForMessage(ws);
      expect(eoseMessage).toEqual(['EOSE', 'emptysub']);

      ws.close();
    });

    it('should send NOTICE for invalid messages', async () => {
      server = new NostrRelayServer({ port: 0 }, store);
      await server.start();

      const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws);

      // Send invalid message
      ws.send('not json');

      const noticeMessage = await waitForMessage(ws);
      expect(noticeMessage).toEqual(['NOTICE', 'error: invalid JSON']);

      ws.close();
    });
  });

  describe('integration: SqliteEventStore with NostrRelayServer', () => {
    const TEST_DB_PATH = './test-relay-events.db';
    let sqliteStore: SqliteEventStore;

    afterEach(() => {
      if (sqliteStore) {
        sqliteStore.close();
      }
      if (existsSync(TEST_DB_PATH)) {
        unlinkSync(TEST_DB_PATH);
      }
    });

    it('should return events from SQLite store via REQ', async () => {
      sqliteStore = new SqliteEventStore(':memory:');
      const event = createMockEvent({
        id: 'sqliteevent123',
        kind: 1,
        content: 'stored in sqlite',
      });
      sqliteStore.store(event);

      server = new NostrRelayServer({ port: 0 }, sqliteStore);
      await server.start();

      const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws);

      const messagesPromise = collectMessages(ws, 2);
      ws.send(JSON.stringify(['REQ', 'sqlitesub', { kinds: [1] }]));

      const messages = await messagesPromise;
      const eventMsg = messages[0] as unknown[];
      expect(eventMsg[0]).toBe('EVENT');
      expect(eventMsg[1]).toBe('sqlitesub');
      // Canonical NIP-01: the event is a JSON object, reconstructed from
      // SQLite columns — never a re-encoded string.
      expect(eventMsg[2]).toEqual(event);
      expect(messages[1]).toEqual(['EOSE', 'sqlitesub']);

      ws.close();
    });

    it('should persist events across server restart with file-based database', async () => {
      // First server instance - store event
      sqliteStore = new SqliteEventStore(TEST_DB_PATH);
      const event = createMockEvent({
        id: 'persistedevent456',
        kind: 1,
        content: 'should persist',
      });
      sqliteStore.store(event);

      server = new NostrRelayServer({ port: 0 }, sqliteStore);
      await server.start();

      // Verify event is accessible
      const ws1 = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws1);

      let messagesPromise = collectMessages(ws1, 2);
      ws1.send(JSON.stringify(['REQ', 'sub1', {}]));

      let messages = await messagesPromise;
      let eventMsg = messages[0] as unknown[];
      expect(eventMsg[0]).toBe('EVENT');
      expect(eventMsg[1]).toBe('sub1');
      expect(eventMsg[2]).toEqual(event);
      expect(messages[1]).toEqual(['EOSE', 'sub1']);

      ws1.close();
      await server.stop();
      sqliteStore.close();

      // Second server instance - should still have the event
      sqliteStore = new SqliteEventStore(TEST_DB_PATH);
      server = new NostrRelayServer({ port: 0 }, sqliteStore);
      await server.start();

      const ws2 = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws2);

      messagesPromise = collectMessages(ws2, 2);
      ws2.send(JSON.stringify(['REQ', 'sub2', {}]));

      messages = await messagesPromise;
      eventMsg = messages[0] as unknown[];
      expect(eventMsg[0]).toBe('EVENT');
      expect(eventMsg[1]).toBe('sub2');
      expect(eventMsg[2]).toEqual(event);
      expect(messages[1]).toEqual(['EOSE', 'sub2']);

      ws2.close();
    });

    it('should send EOSE after SQLite query completes', async () => {
      sqliteStore = new SqliteEventStore(':memory:');

      // Store multiple events
      for (let i = 0; i < 5; i++) {
        sqliteStore.store(
          createMockEvent({
            id: `event${i}`,
            kind: 1,
            created_at: 1000 + i,
          })
        );
      }

      server = new NostrRelayServer({ port: 0 }, sqliteStore);
      await server.start();

      const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws);

      // Collect 5 EVENTs + 1 EOSE = 6 messages
      const messagesPromise = collectMessages(ws, 6);
      ws.send(JSON.stringify(['REQ', 'multisub', { kinds: [1] }]));

      const messages = await messagesPromise;

      // First 5 should be EVENTs
      for (let i = 0; i < 5; i++) {
        expect((messages[i] as unknown[])[0]).toBe('EVENT');
        expect((messages[i] as unknown[])[1]).toBe('multisub');
      }

      // Last should be EOSE
      expect(messages[5]).toEqual(['EOSE', 'multisub']);

      ws.close();
    });
  });
});

describe('connection cap (relay#90)', () => {
  let server: NostrRelayServer;
  const store = new InMemoryEventStore();

  afterEach(async () => {
    await server.stop();
  });

  it('defaults to 4096 concurrent connections (the stock 100 capped huddle listeners)', () => {
    server = new NostrRelayServer({ port: 0 }, store);
    expect(DEFAULT_RELAY_CONFIG.maxConnections).toBe(4096);
  });

  it('rejects connections beyond maxConnections with close code 1013', async () => {
    server = new NostrRelayServer({ port: 0, maxConnections: 1 }, store);
    await server.start();

    const ws1 = new WebSocket(`ws://localhost:${server.getPort()}`);
    await waitForOpen(ws1);

    const ws2 = new WebSocket(`ws://localhost:${server.getPort()}`);
    const closeCode = await new Promise<number>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error('timed out waiting for close')),
        5000
      );
      ws2.on('close', (code: number) => {
        clearTimeout(timer);
        resolve(code);
      });
      ws2.on('error', () => undefined);
    });

    expect(closeCode).toBe(1013);
    expect(server.getClientCount()).toBe(1);
    ws1.close();
  });
});

describe('readOpenFilesSoftLimit (fd-limit advisory, relay#90)', () => {
  const limitsFile = [
    'Limit                     Soft Limit           Hard Limit           Units',
    'Max cpu time              unlimited            unlimited            seconds',
    'Max open files            1024                 1048576              files',
    '',
  ].join('\n');

  it('parses the soft limit from /proc/self/limits content', () => {
    expect(readOpenFilesSoftLimit(() => limitsFile)).toBe(1024);
  });

  it('maps "unlimited" to Infinity', () => {
    expect(
      readOpenFilesSoftLimit(() => limitsFile.replace(/1024 /, 'unlimited'))
    ).toBe(Infinity);
  });

  it('returns null when the file is unreadable (non-Linux) or malformed', () => {
    expect(
      readOpenFilesSoftLimit(() => {
        throw new Error('ENOENT');
      })
    ).toBeNull();
    expect(readOpenFilesSoftLimit(() => 'nothing relevant')).toBeNull();
  });
});

describe('serialize-once broadcast (relay#91)', () => {
  let server: NostrRelayServer;

  afterEach(async () => {
    await server.stop();
  });

  it('stringifies the event exactly once per broadcast while every subscriber receives a canonical frame', async () => {
    server = new NostrRelayServer({ port: 0 }, new InMemoryEventStore());
    await server.start();

    // Three subscribers with distinct subscription ids, all matching.
    const clients: WebSocket[] = [];
    for (let i = 0; i < 3; i++) {
      const ws = new WebSocket(`ws://localhost:${server.getPort()}`);
      await waitForOpen(ws);
      const eose = waitForMessage(ws);
      ws.send(JSON.stringify(['REQ', `sub${i}`, { kinds: [20001] }]));
      expect(await eose).toEqual(['EOSE', `sub${i}`]);
      clients.push(ws);
    }

    const event = createMockEvent({ id: 'bcast1', kind: 20001 });
    const framePromises = clients.map((ws) => waitForMessage(ws));

    // The fan-out must serialize the event payload ONCE, not once per
    // subscriber (relay#91: 500 subscribers = 500 identical stringifies).
    const spy = vi.spyOn(JSON, 'stringify');
    server.broadcastEvent(event);
    const eventSerializations = spy.mock.calls.filter(
      (call) => call[0] === event
    ).length;
    spy.mockRestore();
    expect(eventSerializations).toBe(1);

    // Every subscriber still gets the canonical NIP-01 envelope with ITS
    // subscription id (the spliced frame is byte-identical to
    // JSON.stringify -- pinned in ConnectionHandler tests).
    const frames = await Promise.all(framePromises);
    frames.forEach((frame, i) => {
      expect(frame).toEqual(['EVENT', `sub${i}`, event]);
    });

    for (const ws of clients) ws.close();
  });
});
