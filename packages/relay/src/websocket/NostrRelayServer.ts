import { readFileSync } from 'node:fs';
import type { WebSocket } from 'ws';
import { WebSocketServer } from 'ws';
import type { NostrEvent } from 'nostr-tools/pure';
import type { EventStore } from '../storage/index.js';
import type { RelayServerConfig } from '../types.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';
import { ConnectionHandler } from './ConnectionHandler.js';

/**
 * File descriptors reserved for everything that is not a client WS
 * connection (SQLite, HTTP server sockets, stdio, worker threads...).
 */
const FD_HEADROOM = 128;

/**
 * Read this process's soft "Max open files" limit from /proc/self/limits.
 * Returns null off-Linux or on any parse failure (the check is advisory).
 *
 * @internal Exported for unit testing.
 */
export function readOpenFilesSoftLimit(
  read: (path: string) => string = (p) => readFileSync(p, 'utf8')
): number | null {
  try {
    const line = read('/proc/self/limits')
      .split('\n')
      .find((l) => l.startsWith('Max open files'));
    const match = line?.match(/Max open files\s+(\S+)/);
    if (!match?.[1]) return null;
    if (match[1] === 'unlimited') return Infinity;
    const limit = parseInt(match[1], 10);
    return Number.isNaN(limit) ? null : limit;
  } catch {
    return null;
  }
}

/**
 * A NIP-01 compliant Nostr relay WebSocket server.
 * Handles client connections and routes messages to ConnectionHandlers.
 */
export class NostrRelayServer {
  private wss: WebSocketServer | null = null;
  private handlers = new Map<WebSocket, ConnectionHandler>();
  private config: Required<RelayServerConfig>;

  constructor(
    config: Partial<RelayServerConfig> = {},
    private eventStore: EventStore
  ) {
    this.config = { ...DEFAULT_RELAY_CONFIG, ...config };
  }

  /**
   * Start the WebSocket server.
   */
  async start(): Promise<void> {
    return new Promise((resolve, reject) => {
      try {
        this.wss = new WebSocketServer({
          port: this.config.port,
          host: this.config.host,
        });

        this.wss.on('connection', (ws: WebSocket) => {
          this.handleConnection(ws);
        });

        this.wss.on('error', (error: Error) => {
          console.error('[NostrRelayServer] Server error:', error.message);
        });

        this.wss.on('listening', () => {
          const address = this.wss?.address();
          if (address && typeof address === 'object') {
            console.log(`[NostrRelayServer] Listening on port ${address.port}`);
          }
          // Advisory fd-limit check (relay#90): each connection costs one
          // fd, so a maxConnections above the soft nofile limit would hit
          // EMFILE long before the configured cap.
          const fdLimit = readOpenFilesSoftLimit();
          if (
            fdLimit !== null &&
            Number.isFinite(fdLimit) &&
            this.config.maxConnections > fdLimit - FD_HEADROOM
          ) {
            console.warn(
              `[NostrRelayServer] maxConnections (${this.config.maxConnections}) ` +
                `exceeds the process fd soft limit (${fdLimit}) minus ` +
                `${FD_HEADROOM} headroom -- connections will fail with EMFILE ` +
                `before the cap. Raise \`ulimit -n\` or lower maxConnections.`
            );
          }
          resolve();
        });
      } catch (error) {
        reject(error);
      }
    });
  }

  /**
   * Stop the WebSocket server and close all connections.
   */
  async stop(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.wss) {
        resolve();
        return;
      }

      // Clean up all connection handlers
      for (const [ws, handler] of this.handlers) {
        handler.cleanup();
        ws.close();
      }
      this.handlers.clear();

      this.wss.close(() => {
        this.wss = null;
        resolve();
      });
    });
  }

  /**
   * Get the port the server is listening on.
   * Returns 0 if the server is not started.
   */
  getPort(): number {
    if (!this.wss) return 0;
    const address = this.wss.address();
    if (address && typeof address === 'object') {
      return address.port;
    }
    return 0;
  }

  /**
   * Get the number of connected clients.
   */
  getClientCount(): number {
    return this.handlers.size;
  }

  /**
   * Broadcast an event to all connected clients with matching subscriptions.
   * Call this after storing an event outside the WebSocket flow (e.g., via ILP)
   * so that discovery subscribers are notified.
   *
   * Serialize-once fan-out (relay#91): the event payload is stringified ONE
   * time here and reused for every matching subscriber -- only the small
   * per-subscription `["EVENT",<subId>,...]` envelope is spliced per send.
   * Previously each of N subscribers re-serialized the identical event
   * (N=500 pinned a core doing 500 identical stringifies per frame).
   */
  broadcastEvent(event: NostrEvent): void {
    const eventJson = JSON.stringify(event);
    for (const handler of this.handlers.values()) {
      handler.notifyNewEvent(event, eventJson);
    }
  }

  private handleConnection(ws: WebSocket): void {
    // Check max connections
    if (this.handlers.size >= this.config.maxConnections) {
      console.warn(
        `[NostrRelayServer] connection rejected: maxConnections ` +
          `(${this.config.maxConnections}) reached -- raise TOON_MAX_CONNECTIONS ` +
          `if this box has headroom (relay#90)`
      );
      ws.close(1013, 'max connections reached');
      return;
    }

    console.log('[NostrRelayServer] Client connected');

    const handler = new ConnectionHandler(ws, this.eventStore, this.config);
    this.handlers.set(ws, handler);

    ws.on('message', (data: Buffer | string) => {
      const message = typeof data === 'string' ? data : data.toString();
      handler.handleMessage(message);
    });

    ws.on('close', () => {
      console.log('[NostrRelayServer] Client disconnected');
      handler.cleanup();
      this.handlers.delete(ws);
    });

    ws.on('error', (error: Error) => {
      console.error('[NostrRelayServer] Client error:', error.message);
      handler.cleanup();
      this.handlers.delete(ws);
    });
  }
}
