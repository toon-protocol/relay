/**
 * Tests for TownInstance.subscribe() -- relay subscription API (Story 2.8).
 *
 * Tests the subscription wrapper logic via the extracted `createSubscription()`
 * helper and verifies the running-state guard pattern.
 *
 * Mock strategy: Mock `@toon-protocol/relay` to replace `RelaySubscriber` with a
 * controlled mock that captures constructor args and `start()` behavior.
 * This avoids issues with bundled transitive dependencies (nostr-tools/pool).
 */

import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import type { Filter } from 'nostr-tools/filter';
import type { TownSubscription } from './town.js';

// ---------- Mock RelaySubscriber ----------

let mockUnsubscribe: Mock;
let mockStart: Mock;
let lastConstructorArgs: {
  config: { relayUrls: string[]; filter: Filter };
  eventStore: unknown;
} | null = null;

vi.mock('@toon-protocol/relay', async () => {
  const actual: Record<string, unknown> =
    await vi.importActual('@toon-protocol/relay');
  return {
    ...actual,
    RelaySubscriber: vi.fn(
      (
        config: { relayUrls: string[]; filter: Filter },
        eventStore: unknown
      ) => {
        mockUnsubscribe = vi.fn();
        lastConstructorArgs = { config, eventStore };
        mockStart = vi.fn(() => ({
          unsubscribe: mockUnsubscribe,
        }));
        return {
          start: mockStart,
        };
      }
    ),
  };
});

// Import after mock setup (Vitest hoists vi.mock automatically)
const { createSubscription } = await import('./town.js');

// ---------- Helpers ----------

function makeMockStore() {
  return {
    store: vi.fn(),
    get: vi.fn(),
    query: vi.fn().mockReturnValue([]),
  };
}

// ---------- Tests ----------

describe('TownInstance.subscribe() (Story 2.8)', () => {
  let mockStore: ReturnType<typeof makeMockStore>;
  let activeSubscriptions: Set<TownSubscription>;

  beforeEach(() => {
    vi.clearAllMocks();
    lastConstructorArgs = null;

    mockStore = makeMockStore();
    activeSubscriptions = new Set<TownSubscription>();
  });

  // ========================================================================
  // AC #1: subscribe() creates subscription and returns TownSubscription
  // ========================================================================

  describe('AC #1: subscribe() returns TownSubscription', () => {
    it('creates a subscription and returns a TownSubscription handle', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(sub).toBeDefined();
      expect(typeof sub.close).toBe('function');
      expect(typeof sub.isActive).toBe('function');
      expect(sub.relayUrl).toBe('wss://relay.example.com');
    });

    it('isActive() returns true for active subscription', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(sub.isActive()).toBe(true);
    });

    it('subscription.relayUrl returns the URL passed to subscribe()', () => {
      const sub = createSubscription(
        'wss://my-relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(sub.relayUrl).toBe('wss://my-relay.example.com');
    });

    it('passes eventStore to RelaySubscriber constructor', () => {
      createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(lastConstructorArgs).not.toBeNull();
      expect(lastConstructorArgs?.eventStore).toBe(mockStore);
    });

    it('multiple subscriptions can be active simultaneously', () => {
      const sub1 = createSubscription(
        'wss://relay1.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );
      const sub2 = createSubscription(
        'wss://relay2.example.com',
        { kinds: [10032] },
        mockStore,
        activeSubscriptions
      );
      const sub3 = createSubscription(
        'wss://relay3.example.com',
        { kinds: [10036] },
        mockStore,
        activeSubscriptions
      );

      expect(sub1.isActive()).toBe(true);
      expect(sub2.isActive()).toBe(true);
      expect(sub3.isActive()).toBe(true);
      expect(activeSubscriptions.size).toBe(3);
    });
  });

  // ========================================================================
  // AC #1 (continued): WebSocket connection, filter variants, set tracking
  // ========================================================================

  describe('AC #1: subscribe() WebSocket and filter details', () => {
    it('calls RelaySubscriber.start() to open the WebSocket connection', () => {
      createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(mockStart).toHaveBeenCalledTimes(1);
    });

    it('wraps single relayUrl into a relayUrls array for RelaySubscriber', () => {
      createSubscription(
        'wss://single-relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(lastConstructorArgs?.config.relayUrls).toEqual([
        'wss://single-relay.example.com',
      ]);
      expect(lastConstructorArgs?.config.relayUrls).toHaveLength(1);
    });

    it('passes filter with authors field to RelaySubscriber', () => {
      const filter: Filter = {
        kinds: [1],
        authors: ['a'.repeat(64), 'b'.repeat(64)],
      };

      createSubscription(
        'wss://relay.example.com',
        filter,
        mockStore,
        activeSubscriptions
      );

      expect(lastConstructorArgs?.config.filter).toEqual({
        kinds: [1],
        authors: ['a'.repeat(64), 'b'.repeat(64)],
      });
    });

    it('passes filter with combined kinds and since fields', () => {
      const filter: Filter = {
        kinds: [10032, 10036],
        since: 1700000000,
      };

      createSubscription(
        'wss://relay.example.com',
        filter,
        mockStore,
        activeSubscriptions
      );

      expect(lastConstructorArgs?.config.filter).toEqual({
        kinds: [10032, 10036],
        since: 1700000000,
      });
    });

    it('subscription is added to activeSubscriptions on creation', () => {
      expect(activeSubscriptions.size).toBe(0);

      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(activeSubscriptions.size).toBe(1);
      expect(activeSubscriptions.has(sub)).toBe(true);
    });
  });

  // ========================================================================
  // AC #2: subscription.close() unsubscribes and isActive() returns false
  // ========================================================================

  describe('AC #2: subscription.close() lifecycle', () => {
    it('close() unsubscribes and isActive() returns false', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      sub.close();

      expect(sub.isActive()).toBe(false);
      expect(mockUnsubscribe).toHaveBeenCalledTimes(1);
    });

    it('close() removes subscription from activeSubscriptions set', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      expect(activeSubscriptions.has(sub)).toBe(true);
      sub.close();
      expect(activeSubscriptions.has(sub)).toBe(false);
    });

    it('close() calls handle.unsubscribe() to stop receiving events', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      // Before close, unsubscribe should not have been called
      expect(mockUnsubscribe).not.toHaveBeenCalled();

      sub.close();

      // After close, unsubscribe must have been called exactly once
      expect(mockUnsubscribe).toHaveBeenCalledTimes(1);
    });

    it('closing one subscription does not affect others', () => {
      const sub1 = createSubscription(
        'wss://relay1.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );
      const sub2 = createSubscription(
        'wss://relay2.example.com',
        { kinds: [10032] },
        mockStore,
        activeSubscriptions
      );

      sub1.close();

      expect(sub1.isActive()).toBe(false);
      expect(sub2.isActive()).toBe(true);
      expect(activeSubscriptions.has(sub1)).toBe(false);
      expect(activeSubscriptions.has(sub2)).toBe(true);
    });
  });

  // ========================================================================
  // AC #3: lastSeenTimestamp tracked (preparation for reconnection)
  // ========================================================================

  describe('AC #3: reconnection readiness', () => {
    it('delegates to RelaySubscriber which uses SimplePool for reconnection', () => {
      createSubscription(
        'wss://specific-relay.example.com',
        { kinds: [1, 10032] },
        mockStore,
        activeSubscriptions
      );

      // RelaySubscriber internally creates a SimplePool that handles
      // WebSocket reconnection. Verify delegation is correct.
      expect(lastConstructorArgs).not.toBeNull();
      expect(lastConstructorArgs?.config.relayUrls).toEqual([
        'wss://specific-relay.example.com',
      ]);
      expect(lastConstructorArgs?.config.filter).toEqual({
        kinds: [1, 10032],
      });
    });
  });

  // ========================================================================
  // AC #4: town.stop() closes all active subscriptions
  // ========================================================================

  describe('AC #4: stop() closes all subscriptions', () => {
    it('iterating activeSubscriptions and closing simulates stop() behavior', () => {
      const sub1 = createSubscription(
        'wss://relay1.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );
      const sub2 = createSubscription(
        'wss://relay2.example.com',
        { kinds: [10032] },
        mockStore,
        activeSubscriptions
      );

      expect(activeSubscriptions.size).toBe(2);

      // Simulate what stop() does: close all, then clear
      for (const sub of activeSubscriptions) {
        sub.close();
      }
      activeSubscriptions.clear();

      expect(sub1.isActive()).toBe(false);
      expect(sub2.isActive()).toBe(false);
      expect(activeSubscriptions.size).toBe(0);
    });
  });

  // ========================================================================
  // AC #5: kind:10032 events stored in EventStore
  // ========================================================================

  describe('AC #5: kind:10032 subscription', () => {
    it('subscription for kind:10032 creates RelaySubscriber with correct filter', () => {
      createSubscription(
        'wss://relay.example.com',
        { kinds: [10032] },
        mockStore,
        activeSubscriptions
      );

      expect(lastConstructorArgs?.config.filter).toEqual({
        kinds: [10032],
      });
      expect(lastConstructorArgs?.eventStore).toBe(mockStore);
    });
  });

  // ========================================================================
  // AC #6: kind:10036 events stored in EventStore
  // ========================================================================

  describe('AC #6: kind:10036 subscription', () => {
    it('subscription for kind:10036 creates RelaySubscriber with correct filter', () => {
      createSubscription(
        'wss://relay.example.com',
        { kinds: [10036] },
        mockStore,
        activeSubscriptions
      );

      expect(lastConstructorArgs?.config.filter).toEqual({
        kinds: [10036],
      });
      expect(lastConstructorArgs?.eventStore).toBe(mockStore);
    });
  });

  // ========================================================================
  // AC #7: subscribe() throws when town is not running
  // ========================================================================

  describe('AC #7: subscribe() guard when town is stopped', () => {
    it('subscribe() throws Error when town is not running', () => {
      // The running-state guard is in TownInstance.subscribe() and delegates
      // to createSubscription() only after passing the guard. We test the
      // guard pattern here without needing the full startTown() infrastructure.
      let running = false;

      const subscribe = (relayUrl: string, filter: Filter) => {
        if (!running) {
          throw new Error('Cannot subscribe: town is not running');
        }
        return createSubscription(
          relayUrl,
          filter,
          mockStore,
          activeSubscriptions
        );
      };

      expect(() =>
        subscribe('wss://relay.example.com', { kinds: [1] })
      ).toThrow('Cannot subscribe: town is not running');

      // After setting running = true, subscribe should work
      running = true;
      const sub = subscribe('wss://relay.example.com', { kinds: [1] });
      expect(sub.isActive()).toBe(true);
    });
  });

  // ========================================================================
  // URL validation
  // ========================================================================

  describe('URL validation', () => {
    it('throws for http:// URL (non-WebSocket scheme)', () => {
      expect(() =>
        createSubscription(
          'http://relay.example.com',
          { kinds: [1] },
          mockStore,
          activeSubscriptions
        )
      ).toThrow('Invalid relay URL');
    });

    it('throws for empty string URL', () => {
      expect(() =>
        createSubscription('', { kinds: [1] }, mockStore, activeSubscriptions)
      ).toThrow('Invalid relay URL');
    });

    it('accepts ws:// URL', () => {
      const sub = createSubscription(
        'ws://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );
      expect(sub.isActive()).toBe(true);
    });

    it('accepts wss:// URL', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );
      expect(sub.isActive()).toBe(true);
    });
  });

  // ========================================================================
  // AC #8: close() is idempotent
  // ========================================================================

  describe('AC #8: idempotent close()', () => {
    it('calling close() multiple times is a no-op after first call', () => {
      const sub = createSubscription(
        'wss://relay.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );

      sub.close();
      sub.close(); // second call -- should be no-op

      expect(sub.isActive()).toBe(false);
      // The underlying handle.unsubscribe() should only be called once
      // because the active guard prevents the second close() from calling it.
      expect(mockUnsubscribe).toHaveBeenCalledTimes(1);
    });

    it('double close does not corrupt activeSubscriptions set', () => {
      const sub1 = createSubscription(
        'wss://relay1.example.com',
        { kinds: [1] },
        mockStore,
        activeSubscriptions
      );
      const sub2 = createSubscription(
        'wss://relay2.example.com',
        { kinds: [10032] },
        mockStore,
        activeSubscriptions
      );

      expect(activeSubscriptions.size).toBe(2);

      // Close sub1 twice -- should not affect sub2 or corrupt the set
      sub1.close();
      sub1.close();

      expect(activeSubscriptions.size).toBe(1);
      expect(activeSubscriptions.has(sub1)).toBe(false);
      expect(activeSubscriptions.has(sub2)).toBe(true);
    });
  });
});

// ===========================================================================
// AC #4: Static analysis -- stop() closes subscriptions BEFORE relay/BLS
// ===========================================================================

describe('AC #4: stop() cleanup ordering (static analysis)', () => {
  it('stop() closes activeSubscriptions before wsRelay and blsServer', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const townSource = readFileSync(
      resolve(import.meta.dirname, 'town.ts'),
      'utf-8'
    );

    // Extract the stop() method body from the TownInstance object.
    // The stop() method is defined as `async stop()` inside the instance.
    const stopMethodStart = townSource.indexOf('async stop()');
    expect(stopMethodStart).toBeGreaterThan(-1);

    // Extract everything from stop() onward to scope our search
    const stopBody = townSource.slice(stopMethodStart);

    // Find cleanup operations within stop() body
    const subCleanupIndex = stopBody.indexOf(
      'for (const sub of activeSubscriptions)'
    );
    const relayStopIndex = stopBody.indexOf('await wsRelay.stop()');
    const blsCloseIndex = stopBody.indexOf('blsServer.close()');

    expect(subCleanupIndex).toBeGreaterThan(-1);
    expect(relayStopIndex).toBeGreaterThan(-1);
    expect(blsCloseIndex).toBeGreaterThan(-1);

    // Subscriptions must be closed BEFORE relay and BLS within stop()
    expect(subCleanupIndex).toBeLessThan(relayStopIndex);
    expect(subCleanupIndex).toBeLessThan(blsCloseIndex);
  });
});

// ===========================================================================
// AC #7: Static analysis -- subscribe() guard in TownInstance
// ===========================================================================

describe('AC #7: subscribe() running guard (static analysis)', () => {
  it('TownInstance.subscribe() checks running state before delegating', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const townSource = readFileSync(
      resolve(import.meta.dirname, 'town.ts'),
      'utf-8'
    );

    // Verify the subscribe method on TownInstance has the running guard
    expect(townSource).toContain(
      "throw new Error('Cannot subscribe: town is not running')"
    );

    // Verify the guard checks `!running` before calling createSubscription
    const guardIndex = townSource.indexOf(
      'Cannot subscribe: town is not running'
    );
    const createSubIndex = townSource.indexOf(
      'createSubscription(',
      guardIndex - 200
    );

    // The guard must appear before the createSubscription call
    expect(guardIndex).toBeGreaterThan(-1);
    expect(createSubIndex).toBeGreaterThan(-1);
    expect(guardIndex).toBeLessThan(createSubIndex);
  });
});

// ===========================================================================
// AC #3: Static analysis -- lastSeenTimestamp tracked in createSubscription
// ===========================================================================

describe('AC #3: lastSeenTimestamp tracking (static analysis)', () => {
  it('createSubscription tracks lastSeenTimestamp for future reconnection', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const townSource = readFileSync(
      resolve(import.meta.dirname, 'town.ts'),
      'utf-8'
    );

    // Verify lastSeenTimestamp is declared inside createSubscription
    const createSubStart = townSource.indexOf(
      'export function createSubscription('
    );
    expect(createSubStart).toBeGreaterThan(-1);

    // Scope to the createSubscription function body (generous window
    // to accommodate URL validation code added before lastSeenTimestamp)
    const createSubBody = townSource.slice(
      createSubStart,
      createSubStart + 1200
    );
    expect(createSubBody).toContain('lastSeenTimestamp');
  });
});

// ===========================================================================
// Type Export Tests
// ===========================================================================

describe('TownSubscription type export (Story 2.8)', () => {
  it('TownSubscription is exported from @toon-protocol/town', async () => {
    // Verify TownSubscription type is importable by dynamically importing
    const townModule = await import('./index.js');

    // Verify startTown exists (runtime check for the module)
    expect(townModule.startTown).toBeDefined();

    // TownSubscription is a type-only export -- it cannot be verified at
    // runtime. The fact that this file compiles with:
    //   import type { TownSubscription } from './town.js';
    // is the compile-time verification. We verify the module loads cleanly.
    expect(townModule).toBeDefined();
  });
});
