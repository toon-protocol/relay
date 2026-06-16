/**
 * x402 publish handler for the TOON protocol.
 *
 * Implements the HTTP-native payment on-ramp via the x402 protocol pattern.
 * Allows any HTTP client (AI agents, browsers, CLI tools) to publish Nostr
 * events to the network by paying USDC, without understanding ILP or
 * running an ILP client.
 *
 * Flow:
 * 1. Client sends request without X-PAYMENT header -> 402 with pricing
 * 2. Client signs EIP-3009 auth and retries with X-PAYMENT header
 * 3. Handler runs 6 free pre-flight checks
 * 4. Handler settles USDC on-chain via transferWithAuthorization
 * 5. Handler constructs ILP PREPARE via shared buildIlpPrepare()
 * 6. Handler routes PREPARE through connector
 * 7. Handler returns 200 with event ID and tx hash
 *
 * @module
 */

import type { Context } from 'hono';
import type { NostrEvent } from 'nostr-tools/pure';
import { buildIlpPrepare, encodeEventToToon } from '@toon-protocol/core';
import type {
  ChainPreset,
  IlpClient,
  BuildIlpPrepareParams,
} from '@toon-protocol/core';
import { calculateX402Price } from './x402-pricing.js';
import { runPreflight } from './x402-preflight.js';
import type { PreflightConfig } from './x402-preflight.js';
import { settleEip3009 } from './x402-settlement.js';
import type {
  X402SettlementConfig,
  X402SettlementResult,
} from './x402-settlement.js';
import type {
  Eip3009Authorization,
  EventStoreLike,
  X402PublishRequest,
  X402PublishResponse,
  X402PricingResponse,
} from './x402-types.js';
import type { WalletClient, PublicClient } from 'viem';

/**
 * Configuration for the x402 publish handler.
 */
export interface X402HandlerConfig {
  /** Whether x402 is enabled for this node. */
  x402Enabled: boolean;
  /** Resolved chain configuration. */
  chainConfig: ChainPreset;
  /** Base price per byte in USDC micro-units. */
  basePricePerByte: bigint;
  /** Routing buffer percentage for multi-hop overhead (default: 10). */
  routingBufferPercent: number;
  /** Facilitator's EVM address (receives USDC payments). */
  facilitatorAddress: string;
  /** This node's Nostr public key. */
  ownPubkey: string;
  /** Whether dev mode is enabled (skips Schnorr verification). */
  devMode: boolean;
  /** ILP client for sending PREPARE packets. */
  ilpClient?: IlpClient;
  /** Event store for destination reachability check. */
  eventStore?: EventStoreLike;
  /** TOON encoder function (defaults to core's encodeEventToToon). */
  toonEncoder?: (event: NostrEvent) => Uint8Array;
  /** viem wallet client for on-chain settlement (facilitator pays gas). */
  walletClient?: WalletClient;
  /** viem public client for read-only contract calls. */
  publicClient?: PublicClient;
  /** Override settle function (for testing). */
  settle?: (
    auth: Eip3009Authorization,
    config: X402SettlementConfig
  ) => Promise<X402SettlementResult>;
  /** Override pre-flight function (for testing). */
  runPreflightFn?: typeof runPreflight;
}

/**
 * x402 publish handler instance.
 */
export interface X402Handler {
  /** Handle a /publish request (both 402 pricing and paid publish). */
  handlePublish: (c: Context) => Promise<Response>;
}

/**
 * Create an x402 publish handler.
 *
 * Returns a handler that processes both the 402 pricing negotiation
 * (no X-PAYMENT header) and the paid publish flow (with X-PAYMENT header).
 *
 * @param config - Handler configuration.
 * @returns X402Handler with handlePublish method.
 */
export function createX402Handler(config: X402HandlerConfig): X402Handler {
  const encoder = config.toonEncoder ?? encodeEventToToon;

  // Validate facilitator address at construction time (fail fast)
  if (
    config.x402Enabled &&
    (!config.facilitatorAddress ||
      !/^0x[0-9a-fA-F]{40}$/.test(config.facilitatorAddress))
  ) {
    throw new Error(
      'x402 enabled but facilitatorAddress is not a valid EVM address'
    );
  }

  return {
    async handlePublish(c: Context): Promise<Response> {
      // --- Gate: x402 disabled ---
      if (!config.x402Enabled) {
        return c.json({ error: 'x402 not enabled' }, 404);
      }

      // --- Parse request body ---
      let body: X402PublishRequest;
      try {
        body = (await c.req.json()) as X402PublishRequest;
      } catch {
        return c.json({ error: 'Invalid request body' }, 400);
      }

      if (!body.event || !body.destination) {
        return c.json(
          { error: 'Missing required fields: event, destination' },
          400
        );
      }

      // Validate destination ILP address format (must start with 'g.' per ILP)
      if (
        typeof body.destination !== 'string' ||
        !body.destination.startsWith('g.')
      ) {
        return c.json(
          { error: 'Invalid destination: must be a global ILP address (g.*)' },
          400
        );
      }

      // --- TOON-encode the event ---
      let toonBytes: Uint8Array;
      try {
        toonBytes = encoder(body.event);
      } catch {
        return c.json({ error: 'Failed to TOON-encode event' }, 400);
      }

      const toonBase64 = Buffer.from(toonBytes).toString('base64');

      // --- Check for X-PAYMENT header ---
      const paymentHeader = c.req.header('X-PAYMENT');

      if (!paymentHeader) {
        // --- 402 Pricing Response ---
        const price = calculateX402Price(
          {
            basePricePerByte: config.basePricePerByte,
            routingBufferPercent: config.routingBufferPercent,
          },
          toonBytes.length
        );

        const pricing: X402PricingResponse = {
          amount: String(price),
          facilitatorAddress: config.facilitatorAddress,
          paymentNetwork: 'eip-3009',
          chainId: config.chainConfig.chainId,
          usdcAddress: config.chainConfig.usdcAddress,
        };

        return c.json(pricing, 402);
      }

      // --- Parse EIP-3009 authorization from X-PAYMENT header ---
      let authorization: Eip3009Authorization;
      try {
        const parsed: unknown = JSON.parse(paymentHeader);
        authorization = parseAuthorization(parsed);
      } catch {
        return c.json({ error: 'Invalid X-PAYMENT header' }, 400);
      }

      // --- Pre-flight validation (6 free checks) ---
      const preflightConfig: PreflightConfig = {
        chainConfig: config.chainConfig,
        basePricePerByte: config.basePricePerByte,
        ownPubkey: config.ownPubkey,
        devMode: config.devMode,
        publicClient: config.publicClient,
        eventStore: config.eventStore,
      };

      try {
        const preflightFn = config.runPreflightFn ?? runPreflight;
        const preflightResult = await preflightFn(
          authorization,
          toonBase64,
          body.destination,
          preflightConfig
        );

        if (!preflightResult.passed) {
          return c.json(
            {
              error: `Pre-flight check failed: ${preflightResult.failedCheck}`,
              failedCheck: preflightResult.failedCheck,
            },
            400
          );
        }
      } catch {
        // CWE-209: generic error, log details server-side
        console.error('[x402] Pre-flight error');
        return c.json({ error: 'Internal server error' }, 500);
      }

      // --- On-chain settlement ---
      let settlementResult: X402SettlementResult;
      try {
        const settleFn = config.settle ?? settleEip3009;

        // Guard: walletClient is required for real settlement
        if (!config.settle && !config.walletClient) {
          console.error('[x402] Settlement error: walletClient not configured');
          return c.json({ error: 'Internal server error' }, 500);
        }

        const settlementConfig: X402SettlementConfig = {
          chainConfig: config.chainConfig,
          walletClient: config.walletClient as WalletClient,
          publicClient: config.publicClient,
        };

        settlementResult = await settleFn(authorization, settlementConfig);
      } catch {
        // CWE-209: generic error
        console.error('[x402] Settlement error');
        return c.json({ error: 'Internal server error' }, 500);
      }

      if (!settlementResult.success) {
        // Log the full error server-side; return generic message to client
        // to avoid leaking on-chain revert reasons (CWE-209).
        console.error(
          '[x402] Settlement failed:',
          settlementResult.error ?? 'unknown'
        );
        return c.json({ error: 'Settlement failed' }, 400);
      }

      // --- Construct and send ILP PREPARE ---
      const amount = config.basePricePerByte * BigInt(toonBytes.length);

      const prepareParams: BuildIlpPrepareParams = {
        destination: body.destination,
        amount,
        data: toonBytes,
      };

      const prepare = buildIlpPrepare(prepareParams);

      let deliveryStatus: 'fulfilled' | 'rejected' = 'rejected';

      if (config.ilpClient) {
        try {
          const ilpResult = await config.ilpClient.sendIlpPacket(prepare);
          deliveryStatus = ilpResult.accepted ? 'fulfilled' : 'rejected';
        } catch {
          // ILP send failed, but settlement already succeeded.
          // No refund per protocol design.
          deliveryStatus = 'rejected';
        }
      }

      // --- Build response ---
      const response: X402PublishResponse = {
        eventId: body.event.id,
        settlementTxHash: settlementResult.txHash ?? '',
        deliveryStatus,
        refundInitiated: false,
      };

      return c.json(response, 200);
    },
  };
}

/**
 * Parse and validate an EIP-3009 authorization from the X-PAYMENT header.
 *
 * @param parsed - Parsed JSON from the header.
 * @returns Validated Eip3009Authorization.
 * @throws If required fields are missing or invalid.
 */
/**
 * Validate a hex string has the expected format: 0x-prefixed, correct length,
 * and contains only valid hex characters.
 */
function isValidHex(value: string, expectedLength: number): boolean {
  if (value.length !== expectedLength) return false;
  if (!value.startsWith('0x')) return false;
  return /^0x[0-9a-fA-F]+$/.test(value);
}

function parseAuthorization(parsed: unknown): Eip3009Authorization {
  if (typeof parsed !== 'object' || parsed === null) {
    throw new Error('Authorization must be an object');
  }

  const obj = parsed as Record<string, unknown>;

  const from = obj['from'];
  const to = obj['to'];
  const value = obj['value'];
  const validAfter = obj['validAfter'];
  const validBefore = obj['validBefore'];
  const nonce = obj['nonce'];
  const v = obj['v'];
  const r = obj['r'];
  const s = obj['s'];

  // EVM addresses: 0x + 40 hex chars = 42 total
  if (typeof from !== 'string' || !isValidHex(from, 42)) {
    throw new Error('Invalid from address');
  }
  if (typeof to !== 'string' || !isValidHex(to, 42)) {
    throw new Error('Invalid to address');
  }
  // bytes32 nonce: 0x + 64 hex chars = 66 total
  if (typeof nonce !== 'string' || !isValidHex(nonce, 66)) {
    throw new Error('Invalid nonce');
  }
  // bytes32 r and s: 0x + 64 hex chars = 66 total
  if (typeof r !== 'string' || !isValidHex(r, 66)) {
    throw new Error('Invalid r');
  }
  if (typeof s !== 'string' || !isValidHex(s, 66)) {
    throw new Error('Invalid s');
  }
  // v must be 27 or 28 (standard ECDSA recovery id)
  if (typeof v !== 'number' || (v !== 27 && v !== 28)) {
    throw new Error('Invalid v');
  }

  // Validate validAfter and validBefore are numeric (prevent NaN propagation)
  const parsedValidAfter = Number(validAfter);
  const parsedValidBefore = Number(validBefore);
  if (Number.isNaN(parsedValidAfter) || parsedValidAfter < 0) {
    throw new Error('Invalid validAfter');
  }
  if (Number.isNaN(parsedValidBefore) || parsedValidBefore < 0) {
    throw new Error('Invalid validBefore');
  }

  // Validate value is a non-negative numeric value
  const valueStr = String(value);
  let valueBigInt: bigint;
  try {
    valueBigInt = BigInt(valueStr);
  } catch {
    throw new Error('Invalid value');
  }
  if (valueBigInt < 0n) {
    throw new Error('Invalid value: must be non-negative');
  }

  return {
    from,
    to,
    value: valueBigInt,
    validAfter: parsedValidAfter,
    validBefore: parsedValidBefore,
    nonce,
    v,
    r,
    s,
  };
}
