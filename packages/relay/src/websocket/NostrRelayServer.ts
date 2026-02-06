import type { WebSocket } from 'ws';
import { WebSocketServer } from 'ws';
import type { EventStore } from '../storage/index.js';
import type { RelayConfig } from '../types.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';
import { ConnectionHandler } from './ConnectionHandler.js';

/**
 * A NIP-01 compliant Nostr relay WebSocket server.
 * Handles client connections and routes messages to ConnectionHandlers.
 */
export class NostrRelayServer {
  private wss: WebSocketServer | null = null;
  private handlers = new Map<WebSocket, ConnectionHandler>();
  private config: Required<RelayConfig>;

  constructor(
    config: Partial<RelayConfig> = {},
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
        this.wss = new WebSocketServer({ port: this.config.port });

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

  private handleConnection(ws: WebSocket): void {
    // Check max connections
    if (this.handlers.size >= this.config.maxConnections) {
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
