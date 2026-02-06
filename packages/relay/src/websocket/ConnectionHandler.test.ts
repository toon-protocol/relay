import { describe, it, expect, beforeEach, vi } from 'vitest';
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

    it('should send matching events', () => {
      const event = createMockEvent();
      store = createMockEventStore([event]);
      handler = new ConnectionHandler(ws, store);

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      expect(ws.send).toHaveBeenCalledWith(
        JSON.stringify(['EVENT', 'sub1', event])
      );
    });

    it('should send EOSE after events', () => {
      const event = createMockEvent();
      store = createMockEventStore([event]);
      handler = new ConnectionHandler(ws, store);

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      const calls = (ws.send as ReturnType<typeof vi.fn>).mock.calls;
      const lastCall = calls[calls.length - 1];
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
        JSON.stringify(['NOTICE', 'error: invalid message format, expected JSON array'])
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
      ws.readyState = 3; // CLOSED

      handler.handleMessage(JSON.stringify(['REQ', 'sub1', {}]));

      expect(ws.send).not.toHaveBeenCalled();
    });
  });
});
