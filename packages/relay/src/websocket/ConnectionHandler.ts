import type { WebSocket } from 'ws';
import type { Filter } from 'nostr-tools/filter';
import type { NostrEvent } from 'nostr-tools/pure';
import type { EventStore } from '../storage/index.js';
import type { RelayConfig } from '../types.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';

/**
 * Represents an active subscription from a client.
 */
export interface Subscription {
  /** Unique subscription identifier from the client */
  id: string;
  /** Filters applied to this subscription */
  filters: Filter[];
}

/**
 * Handles NIP-01 messages for a single WebSocket connection.
 */
export class ConnectionHandler {
  private subscriptions = new Map<string, Subscription>();
  private config: Required<RelayConfig>;

  constructor(
    private ws: WebSocket,
    private eventStore: EventStore,
    config: Partial<RelayConfig> = {}
  ) {
    this.config = { ...DEFAULT_RELAY_CONFIG, ...config };
  }

  /**
   * Handle an incoming message from the WebSocket.
   */
  handleMessage(data: string): void {
    let message: unknown[];

    try {
      const parsed = JSON.parse(data);
      if (!Array.isArray(parsed)) {
        this.sendNotice('error: invalid message format, expected JSON array');
        return;
      }
      message = parsed;
    } catch {
      this.sendNotice('error: invalid JSON');
      return;
    }

    const messageType = message[0];

    if (messageType === 'REQ') {
      const subscriptionId = message[1];
      const filters = message.slice(2) as Filter[];
      this.handleReq(subscriptionId as string, filters);
    } else if (messageType === 'CLOSE') {
      const subscriptionId = message[1];
      this.handleClose(subscriptionId as string);
    } else {
      this.sendNotice(`error: unknown message type: ${messageType}`);
    }
  }

  /**
   * Handle a REQ message to create/update a subscription.
   */
  private handleReq(subscriptionId: string, filters: Filter[]): void {
    // Validate subscription ID
    if (typeof subscriptionId !== 'string' || subscriptionId.length === 0) {
      this.sendNotice('error: invalid subscription id');
      return;
    }

    // Check subscription limit (only for new subscriptions)
    if (!this.subscriptions.has(subscriptionId)) {
      if (this.subscriptions.size >= this.config.maxSubscriptionsPerConnection) {
        this.sendNotice('error: too many subscriptions');
        return;
      }
    }

    // Check filter limit
    if (filters.length > this.config.maxFiltersPerSubscription) {
      this.sendNotice('error: too many filters');
      return;
    }

    // Store the subscription
    this.subscriptions.set(subscriptionId, {
      id: subscriptionId,
      filters,
    });

    // Query matching events
    const events = this.eventStore.query(filters);

    // Send matching events
    for (const event of events) {
      this.sendEvent(subscriptionId, event);
    }

    // Send EOSE
    this.sendEose(subscriptionId);
  }

  /**
   * Handle a CLOSE message to terminate a subscription.
   */
  private handleClose(subscriptionId: string): void {
    // Silently remove subscription (no error if it doesn't exist per NIP-01)
    this.subscriptions.delete(subscriptionId);
  }

  /**
   * Clean up all subscriptions for this connection.
   */
  cleanup(): void {
    this.subscriptions.clear();
  }

  /**
   * Get the number of active subscriptions.
   */
  getSubscriptionCount(): number {
    return this.subscriptions.size;
  }

  private sendEvent(subscriptionId: string, event: NostrEvent): void {
    this.send(['EVENT', subscriptionId, event]);
  }

  private sendEose(subscriptionId: string): void {
    this.send(['EOSE', subscriptionId]);
  }

  private sendNotice(message: string): void {
    this.send(['NOTICE', message]);
  }

  private send(message: unknown[]): void {
    if (this.ws.readyState === 1) {
      // OPEN
      this.ws.send(JSON.stringify(message));
    }
  }
}
