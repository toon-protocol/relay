import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  finalizeEvent,
  generateSecretKey,
  verifyEvent,
} from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import type { WebSocket } from 'ws';
import { ConnectionHandler } from './ConnectionHandler.js';
import type { EventStore } from '../storage/index.js';

function createMockWebSocket(): WebSocket {
  return {
    send: vi.fn(),
    close: vi.fn(),
    readyState: 1, // OPEN
    on: vi.fn(),
    once: vi.fn(),
    removeListener: vi.fn(),
  } as unknown as WebSocket;
}

function createMockEventStore(events: NostrEvent[] = []): EventStore {
  return {
    store: vi.fn(),
    get: vi.fn(),
    query: vi.fn().mockReturnValue(events),
  };
}

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

describe('ConnectionHandler', () => {
  let ws: WebSocket;
  let store: EventStore;
  let handler: ConnectionHandler;

  beforeEach(() => {
    ws = createMockWebSocket();
    store = createMockEventStore();
    handler = new ConnectionHandler(ws, store);
  });

  describe('REQ message handling', () => {
    it('should create subscription on valid REQ', () => {
      handler.handleMessage(JSON.stringify(['REQ', 'sub1', { kinds: [1] }]));

      expect(store.query).toHaveBeenCalledWith([{ kinds: [1] }]);
      expect(handler.getSubscriptionCount()).toBe(1);
    });

    it('should send matching events as canonical NIP-01 JSON (inline event object)', () => {
      const event = createMockEvent();
      store = createMockEventStore([event]);
      handler = new ConnectionHandler(ws, store);

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['EVENT', 'sub1', event])
      );
      // The event payload must be a JSON object within the frame, never a
      // re-encoded string (TOON text / double-JSON) — #46.
      const frame = (ws.send as ReturnType<typeof vi.fn>).mock.calls
        .map((c: unknown[]) => JSON.parse(c[0] as string) as unknown[])
        .find((m) => m[0] === 'EVENT')!;
      expect(typeof frame[2]).toBe('object');
      expect(frame[2]).toEqual(event);
    });

    it('served stored events verify with an independent nostr implementation (nostr-tools)', () => {
      const signed = finalizeEvent(
        {
          kind: 1,
          created_at: Math.floor(Date.now() / 1000),
          tags: [['t', 'interop']],
          content: 'canonical wire check with "escaped quotes" and `backticks`',
        },
        generateSecretKey()
      );
      store = createMockEventStore([signed]);
      handler = new ConnectionHandler(ws, store);

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      // Decode exactly like a vanilla NIP-01 client: one JSON.parse of the
      // raw wire frame, then verify id+sig from the resulting object.
      const rawFrame = (ws.send as ReturnType<typeof vi.fn>).mock.calls
        .map((c: unknown[]) => c[0] as string)
        .find((raw) => (JSON.parse(raw) as unknown[])[0] === 'EVENT')!;
      const [, subId, wireEvent] = JSON.parse(rawFrame) as [
        string,
        string,
        NostrEvent,
      ];
      expect(subId).toBe('sub1');
      expect(wireEvent.id).toBe(signed.id);
      expect(verifyEvent(wireEvent)).toBe(true);
    });

    it('should send EOSE after events', () => {
      const event = createMockEvent();
      store = createMockEventStore([event]);
      handler = new ConnectionHandler(ws, store);

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      const calls = (ws.send as ReturnType<typeof vi.fn>).mock.calls;
      const lastCall = calls[calls.length - 1]!;
      expect(lastCall[0]).toBe(JSON.stringify(['EOSE', 'sub1']));
    });

    it('should send EOSE even with no matching events', () => {
      store = createMockEventStore([]);
      handler = new ConnectionHandler(ws, store);

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      expect(ws.send).toHaveBeenCalledWith(JSON.stringify(['EOSE', 'sub1']));
    });

    it('should reject REQ with invalid subscription id', () => {
      handler.handleMessage(JSON.stringify(['REQ', '', {}]));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['NOTICE', 'error: invalid subscription id'])
      );
      expect(handler.getSubscriptionCount()).toBe(0);
    });

    it('should reject REQ with NOTICE when maxSubscriptionsPerConnection exceeded', () => {
      handler = new ConnectionHandler(ws, store, {
        maxSubscriptionsPerConnection: 2,
      });

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));
      handler.handleMessage(JSON.stringify(['REQ', 'sub2', {}]));
      handler.handleMessage(JSON.stringify(['REQ', 'sub3', {}]));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['NOTICE', 'error: too many subscriptions'])
      );
      expect(handler.getSubscriptionCount()).toBe(2);
    });

    it('should allow updating existing subscription without counting against limit', () => {
      handler = new ConnectionHandler(ws, store, {
        maxSubscriptionsPerConnection: 2,
      });

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));
      handler.handleMessage(JSON.stringify(['REQ', 'sub2', {}]));
      // Update sub1, not a new subscription
      handler.handleMessage(JSON.stringify(['REQ', 'sub1', { kinds: [1] }]));

      expect(handler.getSubscriptionCount()).toBe(2);
      // Should not have sent "too many subscriptions" notice
      const noticeMessages = (ws.send as ReturnType<typeof vi.fn>).mock.calls
        .map((c) => c[0])
        .filter((m) => m.includes('too many subscriptions'));
      expect(noticeMessages).toHaveLength(0);
    });

    it('should reject REQ with NOTICE when maxFiltersPerSubscription exceeded', () => {
      handler = new ConnectionHandler(ws, store, {
        maxFiltersPerSubscription: 2,
      });

      handler.handleMessage(
        JSON.stringify(['REQ', 'sub1', {}, {}, {}]) // 3 filters
      );

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['NOTICE', 'error: too many filters'])
      );
      expect(handler.getSubscriptionCount()).toBe(0);
    });
  });

  describe('EVENT message handling (ILP-gated)', () => {
    it('should reject external WebSocket EVENT writes', () => {
      const event = createMockEvent();
      handler.handleMessage(JSON.stringify(['EVENT', event]));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify([
          'OK',
          event.id,
          false,
          'restricted: writes require ILP payment',
        ])
      );
      expect(store.store).not.toHaveBeenCalled();
    });
  });

  describe('live subscription path (notifyNewEvent)', () => {
    it('should push new events as canonical NIP-01 JSON (inline event object)', () => {
      handler.handleMessage(JSON.stringify(['REQ', 'live1', { kinds: [1] }]));
      (ws.send as ReturnType<typeof vi.fn>).mockClear();

      const event = createMockEvent({ id: 'liveevent1' });
      handler.notifyNewEvent(event);

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['EVENT', 'live1', event])
      );
    });

    it('live-pushed events verify with an independent nostr implementation (nostr-tools)', () => {
      handler.handleMessage(JSON.stringify(['REQ', 'live1', { kinds: [1] }]));
      (ws.send as ReturnType<typeof vi.fn>).mockClear();

      const signed = finalizeEvent(
        {
          kind: 1,
          created_at: Math.floor(Date.now() / 1000),
          tags: [],
          content: 'live path canonical wire check',
        },
        generateSecretKey()
      );
      handler.notifyNewEvent(signed);

      const rawFrame = (ws.send as ReturnType<typeof vi.fn>).mock
        .calls[0]![0] as string;
      const [type, subId, wireEvent] = JSON.parse(rawFrame) as [
        string,
        string,
        NostrEvent,
      ];
      expect(type).toBe('EVENT');
      expect(subId).toBe('live1');
      expect(typeof wireEvent).toBe('object');
      expect(verifyEvent(wireEvent)).toBe(true);
    });

    it('should not push events that match no subscription filter', () => {
      handler.handleMessage(JSON.stringify(['REQ', 'live1', { kinds: [7] }]));
      (ws.send as ReturnType<typeof vi.fn>).mockClear();

      handler.notifyNewEvent(createMockEvent({ kind: 1 }));

      expect(ws.send).not.toHaveBeenCalled();
    });
  });

  describe('CLOSE message handling', () => {
    it('should remove subscription on CLOSE', () => {
      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));
      expect(handler.getSubscriptionCount()).toBe(1);

      handler.handleMessage(JSON.stringify(['CLOSE', 'sub1']));
      expect(handler.getSubscriptionCount()).toBe(0);
    });

    it('should not error on CLOSE for non-existent subscription', () => {
      expect(() => {
        handler.handleMessage(JSON.stringify(['CLOSE', 'nonexistent']));
      }).not.toThrow();
    });
  });

  describe('invalid message handling', () => {
    it('should send NOTICE for invalid JSON', () => {
      handler.handleMessage('not json');

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['NOTICE', 'error: invalid JSON'])
      );
    });

    it('should send NOTICE for non-array JSON', () => {
      handler.handleMessage(JSON.stringify({ type: 'REQ' }));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify([
          'NOTICE',
          'error: invalid message format, expected JSON array',
        ])
      );
    });

    it('should send NOTICE for unknown message type', () => {
      handler.handleMessage(JSON.stringify(['UNKNOWN', 'data']));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['NOTICE', 'error: unknown message type: UNKNOWN'])
      );
    });
  });

  describe('cleanup', () => {
    it('should clear all subscriptions', () => {
      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));
      handler.handleMessage(JSON.stringify(['REQ', 'sub2', {}]));
      expect(handler.getSubscriptionCount()).toBe(2);

      handler.cleanup();
      expect(handler.getSubscriptionCount()).toBe(0);
    });
  });

  describe('closed connection', () => {
    it('should not send if WebSocket is not open', () => {
      (ws as any).readyState = 3; // CLOSED

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      expect(ws.send).not.toHaveBeenCalled();
    });
  });
});
