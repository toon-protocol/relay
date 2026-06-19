/**
 * Pre-flight validation pipeline for the x402 publish endpoint.
 *
 * Implements 6 free checks that run before any on-chain transaction,
 * preventing gas griefing (E3-R008). All checks are either pure
 * cryptography or read-only RPC calls (no gas cost).
 *
 * Check order (cheapest to most expensive):
 * 1. EIP-3009 signature verification (off-chain, ~1ms)
 * 2. USDC balance check (eth_call, ~50ms)
 * 3. Nonce freshness check (eth_call, ~50ms)
 * 4. TOON shallow parse (pure computation, ~0.1ms)
 * 5. Schnorr signature verification (pure crypto, ~2ms)
 * 6. Destination reachability check (local lookup, ~0.1ms)
 *
 * @module
 */

import { verifyTypedData } from 'viem';
import type { PublicClient } from 'viem';
import { shallowParseToon } from '@toon-protocol/core/toon';
import type { ToonRoutingMeta } from '@toon-protocol/core/toon';
import type { ChainPreset } from '@toon-protocol/core';
import type { Eip3009Authorization, EventStoreLike } from './x402-types.js';
import { EIP_3009_TYPES, USDC_EIP712_DOMAIN, USDC_ABI } from './x402-types.js';

/**
 * Result of running the pre-flight validation pipeline.
 */
export interface PreflightResult {
  /** Whether all checks passed. */
  passed: boolean;
  /** Which check failed (only set if passed is false). */
  failedCheck?: string;
  /** List of check names that were executed. */
  checksPerformed: string[];
}

/**
 * Callback for Schnorr signature verification.
 * Returns true if the signature is valid.
 */
export type SchnorrVerifyFn = (meta: ToonRoutingMeta) => Promise<boolean>;

/**
 * Configuration for the pre-flight validation pipeline.
 */
export interface PreflightConfig {
  /** Resolved chain configuration. */
  chainConfig: ChainPreset;
  /** Base price per byte for pricing validation. */
  basePricePerByte: bigint;
  /** This node's Nostr public key. */
  ownPubkey: string;
  /** Whether dev mode is enabled (skips Schnorr verification). */
  devMode: boolean;
  /** viem public client for read-only contract calls (optional, for testing). */
  publicClient?: PublicClient;
  /** EventStore for destination reachability check (optional). */
  eventStore?: EventStoreLike;
  /** Schnorr verification callback (optional, uses SDK verification pipeline). */
  schnorrVerify?: SchnorrVerifyFn;
}

/**
 * Run the 6-stage pre-flight validation pipeline.
 *
 * All checks are free (no gas cost). If any check fails, execution
 * stops immediately and no on-chain transaction is attempted.
 *
 * @param authorization - EIP-3009 signed authorization from the client.
 * @param toonData - Base64-encoded TOON payload.
 * @param destination - Target ILP address.
 * @param config - Pre-flight configuration.
 * @returns PreflightResult indicating success or which check failed.
 */
export async function runPreflight(
  authorization: Eip3009Authorization,
  toonData: string,
  destination: string,
  config: PreflightConfig
): Promise<PreflightResult> {
  const checksPerformed: string[] = [];

  // --- Check 1: EIP-3009 signature verification (off-chain) ---
  checksPerformed.push('eip3009-signature');
  try {
    const domain = {
      ...USDC_EIP712_DOMAIN,
      chainId: config.chainConfig.chainId,
      verifyingContract: config.chainConfig.usdcAddress as `0x${string}`,
    };

    const valid = await verifyTypedData({
      address: authorization.from as `0x${string}`,
      domain,
      types: EIP_3009_TYPES,
      primaryType: 'TransferWithAuthorization',
      message: {
        from: authorization.from as `0x${string}`,
        to: authorization.to as `0x${string}`,
        value: authorization.value,
        validAfter: BigInt(authorization.validAfter),
        validBefore: BigInt(authorization.validBefore),
        nonce: authorization.nonce as `0x${string}`,
      },
      signature: encodeSignature(authorization),
    });

    if (!valid) {
      return {
        passed: false,
        failedCheck: 'eip3009-signature',
        checksPerformed,
      };
    }
  } catch {
    return { passed: false, failedCheck: 'eip3009-signature', checksPerformed };
  }

  // --- Check 2: USDC balance check (read-only eth_call) ---
  checksPerformed.push('usdc-balance');
  if (config.publicClient) {
    try {
      const balance = await config.publicClient.readContract({
        address: config.chainConfig.usdcAddress as `0x${string}`,
        abi: USDC_ABI,
        functionName: 'balanceOf',
        args: [authorization.from as `0x${string}`],
      });
      if ((balance as bigint) < authorization.value) {
        return { passed: false, failedCheck: 'usdc-balance', checksPerformed };
      }
    } catch {
      return { passed: false, failedCheck: 'usdc-balance', checksPerformed };
    }
  }

  // --- Check 3: Nonce freshness check (read-only eth_call) ---
  checksPerformed.push('nonce-freshness');
  if (config.publicClient) {
    try {
      const used = await config.publicClient.readContract({
        address: config.chainConfig.usdcAddress as `0x${string}`,
        abi: USDC_ABI,
        functionName: 'authorizationState',
        args: [
          authorization.from as `0x${string}`,
          authorization.nonce as `0x${string}`,
        ],
      });
      if (used) {
        return {
          passed: false,
          failedCheck: 'nonce-freshness',
          checksPerformed,
        };
      }
    } catch {
      return { passed: false, failedCheck: 'nonce-freshness', checksPerformed };
    }
  }

  // --- Check 4: TOON shallow parse ---
  checksPerformed.push('toon-shallow-parse');
  let toonMeta: ToonRoutingMeta;
  try {
    const toonBytes = Buffer.from(toonData, 'base64');
    toonMeta = shallowParseToon(toonBytes);
  } catch {
    return {
      passed: false,
      failedCheck: 'toon-shallow-parse',
      checksPerformed,
    };
  }

  // --- Check 5: Schnorr signature verification ---
  checksPerformed.push('schnorr-signature');
  if (!config.devMode && config.schnorrVerify) {
    try {
      const valid = await config.schnorrVerify(toonMeta);
      if (!valid) {
        return {
          passed: false,
          failedCheck: 'schnorr-signature',
          checksPerformed,
        };
      }
    } catch {
      return {
        passed: false,
        failedCheck: 'schnorr-signature',
        checksPerformed,
      };
    }
  }

  // --- Check 6: Destination reachability check ---
  checksPerformed.push('destination-reachability');
  if (config.eventStore) {
    try {
      const events = config.eventStore.query([{ kinds: [10032] }]);
      // A destination is reachable if we have at least one kind:10032 peer
      // info event (which means the connector has peers that may route the
      // packet). Without any peer info, no ILP routes exist.
      if (events.length === 0) {
        return {
          passed: false,
          failedCheck: 'destination-reachability',
          checksPerformed,
        };
      }
    } catch {
      return {
        passed: false,
        failedCheck: 'destination-reachability',
        checksPerformed,
      };
    }
  }

  return { passed: true, checksPerformed };
}

/**
 * Encode an EIP-3009 authorization's v, r, s components into a
 * compact signature hex string for viem's verifyTypedData.
 */
function encodeSignature(auth: Eip3009Authorization): `0x${string}` {
  // r (32 bytes) + s (32 bytes) + v (1 byte)
  const r = auth.r.startsWith('0x') ? auth.r.slice(2) : auth.r;
  const s = auth.s.startsWith('0x') ? auth.s.slice(2) : auth.s;
  const v = auth.v.toString(16).padStart(2, '0');
  return `0x${r}${s}${v}`;
}
