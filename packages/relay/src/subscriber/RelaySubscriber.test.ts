/**
 * Tests for RelaySubscriber — upstream relay subscription and event propagation.
 */

import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import { RelaySubscriber } from './RelaySubscriber.js';
import type { EventStore } from '../storage/index.js';

// Capture the onevent handler from pool.subscribeMany
let capturedOnevent: ((event: NostrEvent) => void) | null = null;
let capturedRelays: string[] | null = null;
let capturedFilter: Filter | null = null;
let mockCloser: { close: Mock };

// Mock SimplePool
vi.mock('nostr-tools/pool', () => ({
  SimplePool: vi.fn(() => ({
    subscribeMany: vi.fn(
      (relays: string[], filter: Filter, opts: { onevent: (event: NostrEvent) => void }) => {
        capturedRelays = relays;
        capturedFilter = filter;
        capturedOnevent = opts.onevent;
        mockCloser = { close: vi.fn() };
        return mockCloser;
      }
    ),
  })),
}));

// Mock verifyEvent — default to returning true
const mockVerifyEvent = vi.fn(() => true);
vi.mock('nostr-tools/pure', async () => {
  const actual = await vi.importActual<typeof import('nostr-tools/pure')>('nostr-tools/pure');
  return {
    ...actual,
    verifyEvent: (...args: unknown[]) => mockVerifyEvent(...args),
  };
});

/** Safely invoke capturedOnevent, throwing if not yet captured. */
function fireEvent(event: NostrEvent): void {
  if (!capturedOnevent) throw new Error('onevent not captured yet');
  capturedOnevent(event);
}

function makeEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  return {
    id: 'e'.repeat(64),
    pubkey: 'a'.repeat(64),
    created_at: Math.floor(Date.now() / 1000),
    kind: 1,
    tags: [],
    content: 'hello',
    sig: 'f'.repeat(128),
    ...overrides,
  };
}

describe('RelaySubscriber', () => {
  let mockStore: EventStore & { store: Mock; get: Mock; query: Mock };

  const defaultConfig = {
    relayUrls: ['wss://relay1.example.com', 'wss://relay2.example.com'],
    filter: { kinds: [1] } as Filter,
  };

  beforeEach(() => {
    vi.clearAllMocks();
    capturedOnevent = null;
    capturedRelays = null;
    capturedFilter = null;
    mockVerifyEvent.mockReturnValue(true);

    mockStore = {
      store: vi.fn(),
      get: vi.fn(),
      query: vi.fn().mockReturnValue([]),
    };
  });

  it('passes correct relay URLs and filters to subscribeMany', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    subscriber.start();

    expect(capturedRelays).toEqual(['wss://relay1.example.com', 'wss://relay2.example.com']);
    expect(capturedFilter).toEqual({ kinds: [1] });
  });

  it('stores valid events', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    subscriber.start();

    const event = makeEvent();
    fireEvent(event);

    expect(mockStore.store).toHaveBeenCalledWith(event);
  });

  it('verifies event signatures by default', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    subscriber.start();

    const event = makeEvent();
    fireEvent(event);

    expect(mockVerifyEvent).toHaveBeenCalledWith(event);
    expect(mockStore.store).toHaveBeenCalledWith(event);
  });

  it('skips events with invalid signatures when verify enabled', () => {
    mockVerifyEvent.mockReturnValue(false);

    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    subscriber.start();

    fireEvent(makeEvent());

    expect(mockVerifyEvent).toHaveBeenCalled();
    expect(mockStore.store).not.toHaveBeenCalled();
  });

  it('stores events without verification when verifySignatures: false', () => {
    const subscriber = new RelaySubscriber(
      { ...defaultConfig, verifySignatures: false },
      mockStore
    );
    subscriber.start();

    fireEvent(makeEvent());

    expect(mockVerifyEvent).not.toHaveBeenCalled();
    expect(mockStore.store).toHaveBeenCalled();
  });

  it('unsubscribe() calls subCloser.close()', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    const handle = subscriber.start();

    handle.unsubscribe();

    expect(mockCloser.close).toHaveBeenCalled();
  });

  it('events after unsubscribe are ignored', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    const handle = subscriber.start();

    handle.unsubscribe();
    fireEvent(makeEvent());

    expect(mockStore.store).not.toHaveBeenCalled();
  });

  it('double start() throws', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    subscriber.start();

    expect(() => subscriber.start()).toThrow('RelaySubscriber already started');
  });

  it('can restart after unsubscribe', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    const handle = subscriber.start();
    handle.unsubscribe();

    // Should not throw — started flag was reset
    const handle2 = subscriber.start();
    expect(handle2).toHaveProperty('unsubscribe');
    handle2.unsubscribe();
  });

  it('store() errors are caught and logged, not thrown', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockStore.store.mockImplementation(() => {
      throw new Error('DB write failed');
    });

    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    subscriber.start();

    // Should not throw
    expect(() => fireEvent(makeEvent())).not.toThrow();

    expect(warnSpy).toHaveBeenCalledWith(
      '[RelaySubscriber] Failed to store event:',
      'DB write failed'
    );

    warnSpy.mockRestore();
  });

  it('unsubscribe is idempotent', () => {
    const subscriber = new RelaySubscriber(defaultConfig, mockStore);
    const handle = subscriber.start();

    handle.unsubscribe();
    handle.unsubscribe();

    expect(mockCloser.close).toHaveBeenCalledTimes(1);
  });
});
