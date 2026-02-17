/**
 * Subscribe to upstream relays and propagate events into the local EventStore.
 *
 * Follows the same lifecycle pattern as core's RelayMonitor and SocialPeerDiscovery:
 * - Accept optional SimplePool for testability
 * - start() returns { unsubscribe } cleanup handle
 * - isUnsubscribed guard prevents processing after teardown
 */

import { SimplePool } from 'nostr-tools/pool';
import { verifyEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import type { EventStore } from '../storage/index.js';

/**
 * Configuration for RelaySubscriber.
 */
export interface RelaySubscriberConfig {
  /** Upstream relay URLs to subscribe to */
  relayUrls: string[];
  /** Nostr filter for which events to pull (e.g. kinds, authors) */
  filter: Filter;
  /** Verify event signatures before storing (default: true) */
  verifySignatures?: boolean;
}

/**
 * Subscribes to upstream Nostr relays and stores received events
 * in the local EventStore. Useful for relay-to-relay event propagation.
 */
export class RelaySubscriber {
  private readonly config: RelaySubscriberConfig;
  private readonly eventStore: EventStore;
  private readonly pool: SimplePool;
  private started = false;

  /**
   * @param config - Subscriber configuration
   * @param eventStore - Storage backend to write events into
   * @param pool - Optional SimplePool instance (creates new one if not provided)
   */
  constructor(config: RelaySubscriberConfig, eventStore: EventStore, pool?: SimplePool) {
    this.config = config;
    this.eventStore = eventStore;
    this.pool = pool ?? new SimplePool();
  }

  /**
   * Start subscribing to the configured upstream relays.
   *
   * @returns Handle with unsubscribe() to stop the subscription
   * @throws Error if already started
   */
  start(): { unsubscribe: () => void } {
    if (this.started) {
      throw new Error('RelaySubscriber already started');
    }
    this.started = true;

    const shouldVerify = this.config.verifySignatures !== false;
    let isUnsubscribed = false;

    const subCloser = this.pool.subscribeMany(
      this.config.relayUrls,
      this.config.filter,
      {
        onevent: (event: NostrEvent) => {
          if (isUnsubscribed) return;

          if (shouldVerify && !verifyEvent(event)) {
            return;
          }

          try {
            this.eventStore.store(event);
          } catch (error) {
            console.warn(
              '[RelaySubscriber] Failed to store event:',
              error instanceof Error ? error.message : 'Unknown error'
            );
          }
        },
      }
    );

    return {
      unsubscribe: () => {
        if (!isUnsubscribed) {
          isUnsubscribed = true;
          subCloser.close();
          this.started = false;
        }
      },
    };
  }
}
