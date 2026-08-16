import type { WebSocket } from 'ws';
import type { Filter } from 'nostr-tools/filter';
import type { NostrEvent } from 'nostr-tools/pure';
import type { EventStore } from '../storage/index.js';
import type { RelayServerConfig } from '../types.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';
import { matchFilter } from '../filters/index.js';
import { isExpired } from '../nips/expiration.js';

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
 * Build a NIP-01 EVENT frame from an ALREADY-SERIALIZED event payload,
 * splicing in the (JSON-escaped) subscription id.
 *
 * Byte-identical to `JSON.stringify(['EVENT', subscriptionId, event])` --
 * pinned by tests -- but lets the broadcast fan-out serialize the event
 * ONCE and reuse the string across every matching subscriber (relay#91:
 * 500 subscribers previously meant 500 identical `JSON.stringify(event)`
 * calls per frame, measured pinning a core in the s500 benchmark run).
 *
 * @param subscriptionId - The per-subscriber NIP-01 subscription id.
 * @param eventJson - `JSON.stringify(event)` output to reuse.
 * @returns The full EVENT frame string for the wire.
 */
export function serializeEventFrame(
  subscriptionId: string,
  eventJson: string
): string {
  return `["EVENT",${JSON.stringify(subscriptionId)},${eventJson}]`;
}

/**
 * Handles NIP-01 messages for a single WebSocket connection.
 */
export class ConnectionHandler {
  private subscriptions = new Map<string, Subscription>();
  private config: Required<RelayServerConfig>;

  constructor(
    private ws: WebSocket,
    private eventStore: EventStore,
    config: Partial<RelayServerConfig> = {}
  ) {
    this.config = { ...DEFAULT_RELAY_CONFIG, ...config };
  }

  /**
   * Handle an incoming message from the WebSocket.
   */
  handleMessage(data: string): void {
    console.log(`[ConnectionHandler] Received message:`, data.slice(0, 150));
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
    console.log(`[ConnectionHandler] Message type: ${messageType}`);

    if (messageType === 'REQ') {
      const subscriptionId = message[1];
      const filters = message.slice(2) as Filter[];
      this.handleReq(subscriptionId as string, filters);
    } else if (messageType === 'EVENT') {
      const event = message[1];
      this.handleEvent(event as NostrEvent);
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
      if (
        this.subscriptions.size >= this.config.maxSubscriptionsPerConnection
      ) {
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
    console.log(
      `[ConnectionHandler] REQ: ${subscriptionId}, filters:`,
      JSON.stringify(filters).slice(0, 100)
    );
    const events = this.eventStore.query(filters);
    console.log(
      `[ConnectionHandler] Query returned ${events.length} events for ${subscriptionId}`
    );

    // Send matching events
    for (const event of events) {
      console.log(
        `[ConnectionHandler] Sending event ${event.id.slice(0, 16)}... to ${subscriptionId}`
      );
      this.sendEvent(subscriptionId, event);
    }

    // Send EOSE
    console.log(`[ConnectionHandler] Sending EOSE for ${subscriptionId}`);
    this.sendEose(subscriptionId);
  }

  /**
   * Handle an EVENT message from a WebSocket client.
   *
   * Rejects all external writes — the relay is ILP-gated (pay to write).
   * Events are only stored through the ILP packet handler which calls
   * eventStore.store() directly and then broadcastEvent() to notify subscribers.
   */
  private handleEvent(event: NostrEvent): void {
    this.sendOk(event.id, false, 'restricted: writes require ILP payment');
  }

  /**
   * Handle a CLOSE message to terminate a subscription.
   */
  private handleClose(subscriptionId: string): void {
    // Silently remove subscription (no error if it doesn't exist per NIP-01)
    this.subscriptions.delete(subscriptionId);
  }

  /**
   * Push a new event to all matching subscriptions on this connection.
   * Used when events are stored outside the WebSocket flow (e.g., via ILP).
   *
   * @param event - The event to fan out (used for filter matching).
   * @param eventJson - Optional pre-serialized `JSON.stringify(event)`.
   *   `NostrRelayServer.broadcastEvent` serializes the event ONCE and passes
   *   it here so a 500-subscriber fan-out costs one serialization, not 500
   *   (relay#91). When omitted (direct callers), the event is serialized
   *   on first matching send.
   */
  notifyNewEvent(event: NostrEvent, eventJson?: string): void {
    // NIP-40 (relay#137): an event that arrives already past its own
    // `expiration` is never fanned out. Without this, live subscribers would
    // receive an event that a REQ one second later would refuse to serve.
    if (
      this.config.enforceExpiration &&
      isExpired(event, Math.floor(Date.now() / 1000))
    ) {
      return;
    }

    let json = eventJson;
    for (const sub of this.subscriptions.values()) {
      const matches = sub.filters.some((f) => matchFilter(event, f));
      if (matches) {
        // Serialize lazily: connections with no matching subscription (the
        // common case in a selective fan-out) never pay for it.
        json ??= JSON.stringify(event);
        this.send(serializeEventFrame(sub.id, json));
      }
    }
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

  /**
   * Emit an outbound NIP-01 EVENT frame.
   *
   * The event MUST go on the wire as canonical NIP-01 JSON —
   * `["EVENT", <subId>, {id, pubkey, created_at, kind, tags, content, sig}]`
   * with the event as a plain JSON object — so any standard nostr client can
   * parse it and verify `id`/`sig` from the wire bytes (#46). Never re-encode
   * the event (TOON text, double-JSON-stringify, etc.) at this boundary.
   * serializeEventFrame is byte-identical to the full JSON.stringify
   * envelope (pinned by tests).
   */
  private sendEvent(subscriptionId: string, event: NostrEvent): void {
    this.send(serializeEventFrame(subscriptionId, JSON.stringify(event)));
  }

  private sendEose(subscriptionId: string): void {
    this.send(['EOSE', subscriptionId]);
  }

  private sendOk(eventId: string, success: boolean, message: string): void {
    this.send(['OK', eventId, success, message]);
  }

  private sendNotice(message: string): void {
    this.send(['NOTICE', message]);
  }

  /** Send a message: pre-serialized frames go out as-is (relay#91). */
  private send(message: unknown[] | string): void {
    if (this.ws.readyState === 1) {
      // OPEN
      this.ws.send(
        typeof message === 'string' ? message : JSON.stringify(message)
      );
    }
  }
}
