import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import type { IncomingMessage, Server, ServerResponse } from 'node:http';
import type { WebSocket } from 'ws';
import { WebSocketServer } from 'ws';
import type { NostrEvent } from 'nostr-tools/pure';
import type { EventStore } from '../storage/index.js';
import type { RelayServerConfig } from '../types.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';
import {
  acceptsRelayInformation,
  buildRelayInformationDocument,
  NOSTR_JSON_CONTENT_TYPE,
} from '../nips/relay-information.js';
import type { RelayInformationDocument } from '../nips/relay-information.js';
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
  private http: Server | null = null;
  private handlers = new Map<WebSocket, ConnectionHandler>();
  private config: Required<RelayServerConfig>;

  constructor(
    config: Partial<RelayServerConfig> = {},
    private eventStore: EventStore
  ) {
    this.config = { ...DEFAULT_RELAY_CONFIG, ...config };
  }

  /**
   * The NIP-11 relay information document as it stands right now.
   *
   * Built on every read rather than cached, because both halves of it move:
   * the paid write edge is re-read from the connector in the background, and
   * the limits come from the config object the connection handlers enforce.
   * A cached copy is exactly the drift this document exists to remove.
   */
  relayInformation(): RelayInformationDocument {
    return buildRelayInformationDocument({
      pubkey: this.config.pubkey,
      limits: this.config,
      edge: this.config.writeEdge(),
      description: this.config.description,
    });
  }

  /**
   * Answer a plain HTTP request on the read port.
   *
   * Before this the `ws` library owned the port outright and answered every
   * non-upgrade request `426 Upgrade Required` — which is what the live devnet
   * relay still does, and why it serves no NIP-11 document. The server is now
   * this repo's own, so that answer has to be written down; it is kept byte
   * for byte so that the ONLY request whose answer changes is the one that
   * asks for the document by name.
   */
  private handleHttpRequest(
    request: IncomingMessage,
    response: ServerResponse
  ): void {
    const method = request.method ?? 'GET';

    // NIP-11 is read by browser clients, which need the document to be
    // cross-origin readable. It is free, public and identical for everyone,
    // so there is nothing for an origin check to protect.
    const cors = {
      'access-control-allow-origin': '*',
      'access-control-allow-headers': 'accept, content-type',
      'access-control-allow-methods': 'GET, HEAD, OPTIONS',
    };

    if (method === 'OPTIONS') {
      response.writeHead(204, cors);
      response.end();
      return;
    }

    if (
      (method === 'GET' || method === 'HEAD') &&
      acceptsRelayInformation(request.headers.accept)
    ) {
      const body = JSON.stringify(this.relayInformation());
      response.writeHead(200, {
        ...cors,
        'content-type': NOSTR_JSON_CONTENT_TYPE,
        'content-length': Buffer.byteLength(body),
      });
      response.end(method === 'HEAD' ? undefined : body);
      return;
    }

    response.writeHead(426, { 'content-type': 'text/plain' });
    response.end('Upgrade Required');
  }

  /**
   * Start the WebSocket server.
   */
  async start(): Promise<void> {
    return new Promise((resolve, reject) => {
      try {
        // Our own HTTP server, so the read port can answer NIP-11 as well as
        // upgrade. `ws` attaches its `upgrade` listener to it and handles
        // every WebSocket handshake exactly as before; only the requests it
        // never wanted reach `handleHttpRequest`.
        this.http = createServer((request, response) => {
          this.handleHttpRequest(request, response);
        });

        this.wss = new WebSocketServer({ server: this.http });

        this.wss.on('connection', (ws: WebSocket) => {
          this.handleConnection(ws);
        });

        this.wss.on('error', (error: Error) => {
          console.error('[NostrRelayServer] Server error:', error.message);
        });

        this.http.on('error', (error: Error) => {
          console.error('[NostrRelayServer] Server error:', error.message);
          reject(error);
        });

        this.http.listen(this.config.port, this.config.host, () => {
          const address = this.http?.address();
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
        // The HTTP server is ours now, so closing the WebSocketServer no
        // longer releases the port -- `ws` only closes a server it created
        // itself. Forgetting this leaves the listener bound and the next
        // `start()` on the same port fails EADDRINUSE.
        const http = this.http;
        this.http = null;
        if (!http) {
          resolve();
          return;
        }
        http.close(() => resolve());
        // Any keep-alive connection left over from a NIP-11 read would hold
        // `close()` open until it timed out.
        http.closeAllConnections?.();
      });
    });
  }

  /**
   * Get the port the server is listening on.
   * Returns 0 if the server is not started.
   */
  getPort(): number {
    const address = this.http?.address();
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
