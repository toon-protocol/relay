import { createHash } from 'crypto';
import { Hono } from 'hono';
import { verifyEvent } from 'nostr-tools/pure';
import type { EventStore } from '../storage/index.js';
import { decodeEventFromToon } from '../toon/index.js';
import type {
  BlsConfig,
  HandlePaymentRequest,
  HandlePaymentAcceptResponse,
  HandlePaymentRejectResponse,
} from './types.js';
import { ILP_ERROR_CODES, BlsError, isValidPubkey, SPSP_REQUEST_KIND } from './types.js';

/**
 * Generate a fulfillment from an event ID.
 * The fulfillment is SHA-256(eventId) encoded as base64.
 *
 * Note: The sender must use SHA256(SHA256(eventId)) as the condition
 * in their ILP Prepare packet.
 */
export function generateFulfillment(eventId: string): string {
  const hash = createHash('sha256').update(eventId).digest();
  return hash.toString('base64');
}

/**
 * Business Logic Server for ILP payment verification.
 *
 * Handles payment requests from an ILP connector, verifying that the
 * payment amount meets the required price for storing the included
 * Nostr event.
 */
export class BusinessLogicServer {
  private app: Hono;

  constructor(
    private config: BlsConfig,
    private eventStore: EventStore
  ) {
    // Validate ownerPubkey format if provided
    if (config.ownerPubkey !== undefined && !isValidPubkey(config.ownerPubkey)) {
      throw new BlsError(
        'Invalid ownerPubkey format: must be 64 lowercase hex characters',
        'INVALID_CONFIG'
      );
    }
    this.app = new Hono();
    this.setupRoutes();
  }

  /**
   * Set up HTTP routes.
   */
  private setupRoutes(): void {
    this.app.post('/handle-payment', async (c) => {
      try {
        const body = await c.req.json<HandlePaymentRequest>();
        const response = this.handlePayment(body);
        return c.json(response, response.accept ? 200 : 400);
      } catch (error) {
        const response: HandlePaymentRejectResponse = {
          accept: false,
          code: ILP_ERROR_CODES.INTERNAL_ERROR,
          message:
            error instanceof Error ? error.message : 'Internal server error',
        };
        return c.json(response, 500);
      }
    });

    this.app.get('/health', (c) => {
      return c.json({
        status: 'healthy',
        timestamp: Date.now(),
      });
    });
  }

  /**
   * Process a payment request.
   */
  private handlePayment(
    request: HandlePaymentRequest
  ): HandlePaymentAcceptResponse | HandlePaymentRejectResponse {
    // Validate required fields
    if (!request.amount || !request.destination || !request.data) {
      return {
        accept: false,
        code: ILP_ERROR_CODES.BAD_REQUEST,
        message: 'Missing required fields: amount, destination, data',
      };
    }

    // Decode base64 data
    let toonBytes: Uint8Array;
    try {
      toonBytes = Uint8Array.from(Buffer.from(request.data, 'base64'));
    } catch {
      return {
        accept: false,
        code: ILP_ERROR_CODES.BAD_REQUEST,
        message: 'Invalid base64 encoding in data field',
      };
    }

    // Decode TOON to Nostr event
    let event;
    try {
      event = decodeEventFromToon(toonBytes);
    } catch (error) {
      return {
        accept: false,
        code: ILP_ERROR_CODES.BAD_REQUEST,
        message: `Invalid TOON data: ${error instanceof Error ? error.message : 'Unknown error'}`,
      };
    }

    // Verify event signature
    if (!verifyEvent(event)) {
      return {
        accept: false,
        code: ILP_ERROR_CODES.BAD_REQUEST,
        message: 'Invalid event signature',
      };
    }

    // Self-write bypass: owner events skip payment verification
    if (this.config.ownerPubkey && event.pubkey === this.config.ownerPubkey) {
      try {
        this.eventStore.store(event);
      } catch (error) {
        throw new BlsError(
          `Failed to store event: ${error instanceof Error ? error.message : 'Unknown error'}`,
          'STORAGE_ERROR'
        );
      }

      return {
        accept: true,
        fulfillment: generateFulfillment(event.id),
        metadata: {
          eventId: event.id,
          storedAt: Date.now(),
        },
      };
    }

    // Calculate price: use PricingService if provided, otherwise simple calculation
    let price = this.config.pricingService
      ? this.config.pricingService.calculatePriceFromBytes(toonBytes, event.kind)
      : BigInt(toonBytes.length) * this.config.basePricePerByte;

    // Apply SPSP min price override if configured and event is SPSP request
    const { spspMinPrice } = this.config;
    const spspMinPriceApplied =
      spspMinPrice !== undefined &&
      event.kind === SPSP_REQUEST_KIND &&
      spspMinPrice < price;
    if (spspMinPriceApplied) {
      price = spspMinPrice;
    }

    // Parse and compare amounts
    let amount: bigint;
    try {
      amount = BigInt(request.amount);
    } catch {
      return {
        accept: false,
        code: ILP_ERROR_CODES.BAD_REQUEST,
        message: 'Invalid amount format',
      };
    }

    if (amount < price) {
      return {
        accept: false,
        code: ILP_ERROR_CODES.INSUFFICIENT_AMOUNT,
        message: 'Insufficient payment amount',
        metadata: {
          required: price.toString(),
          received: amount.toString(),
        },
      };
    }

    // Log 0-amount SPSP acceptance
    if (amount === 0n && event.kind === SPSP_REQUEST_KIND && spspMinPriceApplied) {
      console.log('INFO: Accepted 0-amount SPSP request from pubkey:', event.pubkey.slice(0, 16));
    }

    // Store event
    try {
      this.eventStore.store(event);
    } catch (error) {
      throw new BlsError(
        `Failed to store event: ${error instanceof Error ? error.message : 'Unknown error'}`,
        'STORAGE_ERROR'
      );
    }

    // Generate fulfillment and return success
    const storedAt = Date.now();
    return {
      accept: true,
      fulfillment: generateFulfillment(event.id),
      metadata: {
        eventId: event.id,
        storedAt,
      },
    };
  }

  /**
   * Get the Hono app instance for testing or composition.
   */
  getApp(): Hono {
    return this.app;
  }

  /**
   * Start the HTTP server on the specified port.
   */
  start(port: number): void {
    // Note: In Node.js, you would typically use @hono/node-server
    // For now, this method is a placeholder for actual server startup
    // The actual server binding would be done by the consumer
    console.log(`BLS would start on port ${port}`);
  }
}
