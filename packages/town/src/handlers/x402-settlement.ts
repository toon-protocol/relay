/**
 * EIP-3009 on-chain settlement module for the x402 publish endpoint.
 *
 * Executes `transferWithAuthorization` on the USDC contract to settle
 * the gasless USDC transfer from the client to the facilitator (node
 * operator). The facilitator pays gas; the client pays only USDC.
 *
 * Settlement atomicity (E3-R006):
 * - If settlement fails (revert), no ILP PREPARE is constructed.
 * - If settlement succeeds but ILP PREPARE is rejected, no refund.
 *
 * @module
 */

import type { WalletClient, PublicClient } from 'viem';
import type { ChainPreset } from '@crosstown/core';
import type { Eip3009Authorization } from './x402-types.js';
import { USDC_ABI } from './x402-types.js';

/**
 * Result of an EIP-3009 settlement attempt.
 */
export interface X402SettlementResult {
  /** Whether the on-chain transaction succeeded. */
  success: boolean;
  /** Transaction hash (only set on success). */
  txHash?: string;
  /** Error message (only set on failure). */
  error?: string;
}

/**
 * @deprecated Use X402SettlementResult instead.
 */
export type SettlementResult = X402SettlementResult;

/**
 * Configuration for the settlement module.
 *
 * Named `X402SettlementConfig` to avoid collision with
 * `SettlementConfig` from `@crosstown/core` (bootstrap).
 */
export interface X402SettlementConfig {
  /** Resolved chain configuration. */
  chainConfig: ChainPreset;
  /** viem wallet client for the facilitator (submits the tx, pays gas). */
  walletClient: WalletClient;
  /** viem public client for waiting on transaction receipts. */
  publicClient?: PublicClient;
}

/**
 * Settle an EIP-3009 `transferWithAuthorization` on-chain.
 *
 * Submits the client's signed authorization to the USDC contract.
 * The facilitator (node operator) pays gas for the transaction.
 *
 * @param authorization - Signed EIP-3009 authorization from the client.
 * @param config - Settlement configuration with wallet client.
 * @returns SettlementResult indicating success/failure.
 */
/**
 * @deprecated Use X402SettlementConfig instead.
 */
export type SettlementConfig = X402SettlementConfig;

export async function settleEip3009(
  authorization: Eip3009Authorization,
  config: X402SettlementConfig
): Promise<X402SettlementResult> {
  try {
    const hash = await config.walletClient.writeContract({
      address: config.chainConfig.usdcAddress as `0x${string}`,
      abi: USDC_ABI,
      functionName: 'transferWithAuthorization',
      args: [
        authorization.from as `0x${string}`,
        authorization.to as `0x${string}`,
        authorization.value,
        BigInt(authorization.validAfter),
        BigInt(authorization.validBefore),
        authorization.nonce as `0x${string}`,
        authorization.v,
        authorization.r as `0x${string}`,
        authorization.s as `0x${string}`,
      ],
      chain: null, // Use the wallet client's configured chain
      account: config.walletClient.account ?? null,
    });

    // Optionally wait for receipt if public client is available
    if (config.publicClient) {
      const receipt = await config.publicClient.waitForTransactionReceipt({
        hash,
      });
      if (receipt.status === 'reverted') {
        return {
          success: false,
          error: 'Transaction reverted on-chain',
        };
      }
    }

    return {
      success: true,
      txHash: hash,
    };
  } catch (error: unknown) {
    const message =
      error instanceof Error ? error.message : 'Unknown settlement error';
    return {
      success: false,
      error: message,
    };
  }
}
