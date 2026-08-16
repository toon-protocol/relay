/**
 * NIP-40 on the live broadcast path (relay#137).
 *
 * A subscriber can reach an event two ways: a REQ against stored history, or
 * the fan-out of a freshly written event. The store covers the first. If the
 * second were unguarded, an already-expired event would still land in a live
 * subscription — and a REQ one second later would refuse to serve the very
 * event that subscription just delivered.
 */

import { describe, it, expect, vi } from 'vitest';
import type { NostrEvent } from 'nostr-tools/pure';
import type { WebSocket } from 'ws';
import { ConnectionHandler } from './ConnectionHandler.js';
import type { EventStore } from '../storage/index.js';

function createMockWebSocket(): WebSocket {
  return {
    send: vi.fn(),
    close: vi.fn(),
    readyState: 1,
    on: vi.fn(),
    once: vi.fn(),
    removeListener: vi.fn(),
  } as unknown as WebSocket;
}

const emptyStore: EventStore = {
  store: vi.fn(),
  get: vi.fn(),
  query: vi.fn().mockReturnValue([]),
};

function announce(expiresAt?: number): NostrEvent {
  const now = Math.floor(Date.now() / 1000);
  return {
    id: 'f'.repeat(64),
    pubkey: 'a'.repeat(64),
    created_at: now,
    kind: 10032,
    tags: expiresAt === undefined ? [] : [['expiration', String(expiresAt)]],
    content: '{}',
    sig: 's'.repeat(128),
  };
}

function subscribedHandler(
  ws: WebSocket,
  config?: { enforceExpiration: boolean }
): ConnectionHandler {
  const handler = new ConnectionHandler(ws, emptyStore, config);
  handler.handleMessage(JSON.stringify(['REQ', 'sub1', { kinds: [10032] }]));
  (ws.send as ReturnType<typeof vi.fn>).mockClear();
  return handler;
}

describe('ConnectionHandler NIP-40 broadcast enforcement', () => {
  it('does not fan out an event that is already expired', () => {
    const ws = createMockWebSocket();
    const handler = subscribedHandler(ws);

    handler.notifyNewEvent(announce(Math.floor(Date.now() / 1000) - 10));

    expect(ws.send).not.toHaveBeenCalled();
  });

  it('fans out an event that has not expired', () => {
    const ws = createMockWebSocket();
    const handler = subscribedHandler(ws);

    handler.notifyNewEvent(announce(Math.floor(Date.now() / 1000) + 600));

    expect(ws.send).toHaveBeenCalledTimes(1);
  });

  it('fans out an event with no expiration tag', () => {
    const ws = createMockWebSocket();
    const handler = subscribedHandler(ws);

    handler.notifyNewEvent(announce());

    expect(ws.send).toHaveBeenCalledTimes(1);
  });

  it('fans out expired events again when enforcement is off', () => {
    const ws = createMockWebSocket();
    const handler = subscribedHandler(ws, { enforceExpiration: false });

    handler.notifyNewEvent(announce(Math.floor(Date.now() / 1000) - 10));

    expect(ws.send).toHaveBeenCalledTimes(1);
  });
});
