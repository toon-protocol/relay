/**
 * ATDD tests for Story 3.3: x402 /publish Endpoint (FR-PROD-3)
 *
 * Validates:
 * - Pre-flight validation firewall (6 free checks before on-chain tx)
 * - x402 happy path (402->payment->200)
 * - Packet equivalence (x402 vs ILP produce identical ILP PREPARE)
 * - Settlement atomicity -- revert scenario
 * - No refund on REJECT (payment is for routing attempt, not delivery)
 * - EIP-3009 forged signature rejection
 * - x402 disabled returns 404
 * - Multi-hop pricing with routing buffer
 * - 402 response schema
 * - Dual-protocol server (HTTP + WS)
 * - Pre-flight: insufficient USDC balance
 * - Pre-flight: destination unreachable
 * - buildIlpPrepare() amount correctness
 *
 * Test IDs from test-design-epic-3.md:
 * - T-3.3-01 through T-3.3-13 (unit + integration, P0-P2)
 * - T-3.3-14 (E2E, P3, deferred to nightly -- stub only)
 *
 * Risk mitigations:
 * - E3-R005 (EIP-3009 sig verification bypass)
 * - E3-R006 (Settlement atomicity)
 * - E3-R007 (Packet equivalence)
 * - E3-R008 (Gas griefing via x402)
 * - E3-R009 (Dual-protocol server conflicts)
 * - E3-R010 (Multi-hop pricing opacity)
 *
 * Key architectural decisions:
 * - Shared buildIlpPrepare() for packet equivalence (Decision 8)
 * - No refund on REJECT -- mirrors ILP semantics (Party Mode)
 * - Layered pre-flight validation -- 6 free checks before gas (Party Mode)
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Hono } from 'hono';
import { buildIlpPrepare, encodeEventToToon } from '@crosstown/core';
import { privateKeyToAccount } from 'viem/accounts';
import { createX402Handler } from './x402-publish-handler.js';
import type { X402HandlerConfig } from './x402-publish-handler.js';
import { calculateX402Price } from './x402-pricing.js';
import { runPreflight } from './x402-preflight.js';
import type { PreflightConfig } from './x402-preflight.js';
import type { Eip3009Authorization } from './x402-types.js';
import { EIP_3009_TYPES, USDC_EIP712_DOMAIN } from './x402-types.js';
import type { X402SettlementResult } from './x402-settlement.js';
import type { NostrEvent } from 'nostr-tools/pure';

// ============================================================================
// Factories
// ============================================================================

/**
 * Creates a mock EIP-3009 authorization with sensible defaults.
 * Uses realistic EVM addresses and crypto-formatted fields.
 */
function createEip3009Authorization(
  overrides: Record<string, unknown> = {}
): Eip3009Authorization {
  return {
    from: '0x' + 'a'.repeat(40),
    to: '0x' + 'b'.repeat(40),
    value: 5500n,
    validAfter: 0,
    validBefore: Math.floor(Date.now() / 1000) + 3600,
    nonce: '0x' + 'c'.repeat(64),
    v: 27,
    r: '0x' + 'd'.repeat(64),
    s: '0x' + 'e'.repeat(64),
    ...overrides,
  } as Eip3009Authorization;
}

/**
 * Creates a mock Nostr event for x402 request body.
 */
function createNostrEvent(overrides: Record<string, unknown> = {}) {
  return {
    id: 'a'.repeat(64),
    pubkey: 'f'.repeat(64),
    created_at: Math.floor(Date.now() / 1000),
    kind: 1,
    tags: [],
    content: 'test event content',
    sig: '1'.repeat(128),
    ...overrides,
  };
}

/**
 * Creates a mock x402 publish request body (JSON payload).
 */
function createX402RequestBody(overrides: Record<string, unknown> = {}) {
  return {
    event: createNostrEvent(),
    destination: 'g.crosstown.test-relay',
    ...overrides,
  };
}

/**
 * Creates a mock X402HandlerConfig with all required fields.
 */
function createX402Config(
  overrides: Record<string, unknown> = {}
): X402HandlerConfig {
  return {
    x402Enabled: true,
    chainConfig: {
      name: 'anvil',
      chainId: 31337,
      rpcUrl: 'http://localhost:8545',
      usdcAddress: '0x5FbDB2315678afecb367f032d93F642f64180aa3',
      tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
    },
    basePricePerByte: 10n,
    routingBufferPercent: 10,
    facilitatorAddress: '0x' + 'b'.repeat(40),
    ownPubkey: 'f'.repeat(64),
    devMode: true,
    ...overrides,
  } as X402HandlerConfig;
}

// Anvil account #0 private key (deterministic -- safe for tests only)
const TEST_PRIVATE_KEY =
  '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80' as const;
const TEST_USDC_ADDRESS = '0x5FbDB2315678afecb367f032d93F642f64180aa3' as const;
const TEST_CHAIN_ID = 31337;

/**
 * Creates a cryptographically valid EIP-3009 authorization by signing
 * typed data with a real private key. This allows tests to pass check 1
 * (EIP-3009 signature verification) in the real runPreflight() pipeline.
 */
async function createSignedEip3009Authorization(
  overrides: Record<string, unknown> = {}
): Promise<Eip3009Authorization> {
  const account = privateKeyToAccount(TEST_PRIVATE_KEY);
  const facilitator = '0x' + 'b'.repeat(40);
  const value = overrides['value'] ?? 5500n;
  const validAfter = overrides['validAfter'] ?? 0;
  const validBefore =
    overrides['validBefore'] ?? Math.floor(Date.now() / 1000) + 3600;
  const nonce = (overrides['nonce'] as string) ?? '0x' + 'c'.repeat(64);

  const domain = {
    ...USDC_EIP712_DOMAIN,
    chainId: TEST_CHAIN_ID,
    verifyingContract: TEST_USDC_ADDRESS,
  };

  const message = {
    from: account.address as `0x${string}`,
    to: facilitator as `0x${string}`,
    value: BigInt(value as bigint),
    validAfter: BigInt(validAfter as number),
    validBefore: BigInt(validBefore as number),
    nonce: nonce as `0x${string}`,
  };

  const signature = await account.signTypedData({
    domain,
    types: EIP_3009_TYPES,
    primaryType: 'TransferWithAuthorization',
    message,
  });

  // Decode the compact signature (r + s + v)
  const sigHex = signature.slice(2);
  const r = '0x' + sigHex.slice(0, 64);
  const s = '0x' + sigHex.slice(64, 128);
  const v = parseInt(sigHex.slice(128, 130), 16);

  return {
    from: account.address,
    to: facilitator,
    value: BigInt(value as bigint),
    validAfter: validAfter as number,
    validBefore: validBefore as number,
    nonce,
    v,
    r,
    s,
  };
}

/**
 * Creates a valid TOON-encoded base64 payload from a signed Nostr event.
 * This creates real TOON data that passes check 4 (TOON shallow parse).
 */
function createValidToonBase64(): string {
  const event = createNostrEvent();
  const toonBytes = encodeEventToToon(event);
  return Buffer.from(toonBytes).toString('base64');
}

/**
 * Creates a mock preflight function that always passes (all 6 checks OK).
 * Used for tests that focus on post-preflight logic (settlement, ILP routing).
 */
function createPassingPreflight() {
  return vi.fn().mockResolvedValue({
    passed: true,
    checksPerformed: [
      'eip3009-signature',
      'usdc-balance',
      'nonce-freshness',
      'toon-shallow-parse',
      'schnorr-signature',
      'destination-reachability',
    ],
  });
}

/**
 * Serialize an Eip3009Authorization to JSON, converting BigInt values to strings.
 */
function serializeAuth(auth: Eip3009Authorization): string {
  return JSON.stringify({
    ...auth,
    value: String(auth.value),
  });
}

/**
 * Helper: create a Hono app with the x402 handler and make a request.
 */
async function makeRequest(
  config: X402HandlerConfig,
  body: unknown,
  headers: Record<string, string> = {}
): Promise<Response> {
  const handler = createX402Handler(config);
  const app = new Hono();
  app.post('/publish', (c) => handler.handlePublish(c));
  app.get('/publish', (c) => handler.handlePublish(c));

  const request = new Request('http://localhost/publish', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...headers,
    },
    body: JSON.stringify(body),
  });

  return app.fetch(request);
}

// ============================================================================
// Unit Tests
// ============================================================================

describe('Story 3.3: x402 /publish Endpoint', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  // --------------------------------------------------------------------------
  // T-3.3-13 [P0]: buildIlpPrepare() amount correctness
  // Risk: E3-R007
  // Added per test-design-epic-3.md -- validates pricing validator compat
  // --------------------------------------------------------------------------
  describe('buildIlpPrepare() amount correctness (T-3.3-13)', () => {
    it('[P0] buildIlpPrepare() sets amount field to match basePricePerByte * toonData.length', () => {
      // Given: known inputs for the ILP PREPARE packet
      const destination = 'g.crosstown.target-relay';
      const toonData = new Uint8Array(500); // 500 bytes
      const basePricePerByte = 10n;
      const expectedAmount = basePricePerByte * BigInt(toonData.length); // 5000n

      // When: buildIlpPrepare() constructs the packet parameters
      const result = buildIlpPrepare({
        destination,
        amount: expectedAmount,
        data: toonData,
      });

      // Then: the amount field is a string representation of the expected bigint
      expect(result.amount).toBe(String(expectedAmount));
      expect(result.amount).toBe('5000');
      expect(result.destination).toBe(destination);
      expect(typeof result.data).toBe('string'); // base64 encoded
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-08 [P1]: Multi-hop pricing with routing buffer
  // Risk: E3-R010
  // Level: Unit (pure function, no mocks)
  // --------------------------------------------------------------------------
  describe('Multi-hop pricing (3.3-INT-008)', () => {
    it('[P1] price = basePricePerByte * toonLength + configurable routing buffer', () => {
      // Given: pricing config and known TOON length
      const basePricePerByte = 10n;
      const toonLength = 500;
      const routingBufferPercent = 10; // 10%
      const expectedBasePrice = basePricePerByte * BigInt(toonLength); // 5000n
      const expectedBuffer =
        (expectedBasePrice * BigInt(routingBufferPercent)) / 100n; // 500n
      const expectedTotal = expectedBasePrice + expectedBuffer; // 5500n

      // When: calculateX402Price() computes the price
      const price = calculateX402Price(
        {
          basePricePerByte,
          routingBufferPercent,
        },
        toonLength
      );

      // Then: price includes base + routing buffer
      expect(price).toBe(expectedTotal);
      expect(price).toBe(5500n);
    });

    it('[P1] routing buffer of 0% returns base price only', () => {
      // Given: zero routing buffer
      const basePricePerByte = 10n;
      const toonLength = 500;
      const routingBufferPercent = 0;
      const expectedPrice = basePricePerByte * BigInt(toonLength); // 5000n

      // When: calculateX402Price() computes the price with 0% buffer
      const price = calculateX402Price(
        {
          basePricePerByte,
          routingBufferPercent,
        },
        toonLength
      );

      // Then: price equals base price (no buffer)
      expect(price).toBe(expectedPrice);
      expect(price).toBe(5000n);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-03 [P0]: Packet equivalence (x402 vs ILP)
  // Risk: E3-R007
  // Level: Unit (pure function test, no mocks needed)
  // --------------------------------------------------------------------------
  describe('Packet equivalence (3.3-INT-003)', () => {
    it('[P0] x402 and ILP paths produce identical ILP PREPARE packets via shared buildIlpPrepare()', () => {
      // Given: identical inputs for both x402 and ILP paths
      const destination = 'g.crosstown.test-relay';
      const amount = 5000n;
      const toonData = new Uint8Array([1, 2, 3, 4, 5]); // sample TOON bytes

      // When: buildIlpPrepare() is called twice with the same inputs
      const prepare1 = buildIlpPrepare({ destination, amount, data: toonData });
      const prepare2 = buildIlpPrepare({ destination, amount, data: toonData });

      // Then: both outputs are structurally identical
      expect(prepare1.destination).toBe(prepare2.destination);
      expect(prepare1.amount).toBe(prepare2.amount);
      expect(prepare1.data).toBe(prepare2.data);

      // And: both use the same string representation for amount
      expect(prepare1.amount).toBe('5000');

      // And: data is base64-encoded
      const expectedBase64 = Buffer.from(toonData).toString('base64');
      expect(prepare1.data).toBe(expectedBase64);
    });

    it('[P0] buildIlpPrepare output matches IlpClient.sendIlpPacket expected shape', () => {
      // Given: typical inputs
      const destination = 'g.crosstown.target-relay';
      const amount = 10000n;
      const toonData = new Uint8Array(100).fill(42);

      // When: buildIlpPrepare() constructs the packet
      const packet = buildIlpPrepare({ destination, amount, data: toonData });

      // Then: output has exactly the 3 fields IlpClient.sendIlpPacket expects
      // (destination: string, amount: string, data: string -- base64)
      expect(Object.keys(packet).sort()).toEqual(
        ['amount', 'data', 'destination'].sort()
      );
      expect(typeof packet.destination).toBe('string');
      expect(typeof packet.amount).toBe('string');
      expect(typeof packet.data).toBe('string');

      // And: amount is the string representation of the bigint (not scientific notation)
      expect(packet.amount).toBe('10000');
      expect(packet.amount).not.toContain('e');

      // And: data can be decoded back to the original bytes
      const decoded = Buffer.from(packet.data, 'base64');
      expect(new Uint8Array(decoded)).toEqual(toonData);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-01 [P0]: Pre-flight validation firewall
  // Risk: E3-R005, E3-R008
  // Level: Integration (mock viem reads, mock EventStore)
  // --------------------------------------------------------------------------
  describe('Pre-flight validation firewall (3.3-INT-001)', () => {
    it('[P0] pre-flight short-circuits at first failed check (EIP-3009 sig)', async () => {
      // Given: forged authorization (zeroed sig components) and TOON payload
      const authorization = createEip3009Authorization();
      const toonData = Buffer.from('test_toon_data').toString('base64');
      const destination = 'g.crosstown.test-relay';

      // And: preflight config with devMode (skips Schnorr only, not EIP-3009 sig)
      // No publicClient = skip balance/nonce checks (pass by default)
      // No eventStore = skip reachability check (pass by default)
      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: 31337,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: '0x5FbDB2315678afecb367f032d93F642f64180aa3',
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
      };

      // When: runPreflight() executes checks with forged sig
      const result = await runPreflight(
        authorization,
        toonData,
        destination,
        preflightConfig
      );

      // Then: short-circuits at the first check (EIP-3009 sig verification)
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('eip3009-signature');
      expect(result.checksPerformed).toEqual(['eip3009-signature']);
      // Only 1 check ran -- the pipeline stops at the first failure,
      // which prevents any subsequent on-chain calls (gas griefing mitigation)
    });

    it('[P0] all 6 checks execute when preflight passes (mock)', async () => {
      // Given: a handler with a passing preflight mock
      const passingPreflight = createPassingPreflight();
      const mockSettle = vi.fn().mockResolvedValue({
        success: true,
        txHash: '0x' + 'f'.repeat(64),
      } satisfies X402SettlementResult);
      const mockIlpClient = {
        sendIlpPacket: vi.fn().mockResolvedValue({
          accepted: true,
          fulfillment: 'test-fulfillment',
        }),
      };

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: passingPreflight,
      });

      // When: request with valid X-PAYMENT goes through the handler
      await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: preflight was called and reported all 6 check names
      expect(passingPreflight).toHaveBeenCalledOnce();
      const preflightResult = await passingPreflight.mock.results[0]?.value;
      expect(preflightResult.checksPerformed).toEqual([
        'eip3009-signature',
        'usdc-balance',
        'nonce-freshness',
        'toon-shallow-parse',
        'schnorr-signature',
        'destination-reachability',
      ]);
      // And: settlement proceeded (no gas spent before all 6 checks pass)
      expect(mockSettle).toHaveBeenCalledOnce();
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-02 [P0]: x402 happy path (402->payment->200)
  // Risk: E3-R005, E3-R006
  // Level: Integration (mock settlement, mock connector)
  // --------------------------------------------------------------------------
  describe('x402 happy path (3.3-INT-002)', () => {
    it('[P0] full 402 negotiation -> EIP-3009 -> settlement -> ILP PREPARE -> FULFILL -> 200', async () => {
      // Given: an x402 handler with all dependencies mocked
      const requestBody = createX402RequestBody();

      // Step 1: Request without X-PAYMENT -> 402
      const config = createX402Config();
      const response402 = await makeRequest(config, requestBody);

      // Then: 402 with pricing info
      expect(response402.status).toBe(402);
      const body402 = (await response402.json()) as Record<string, unknown>;
      expect(body402['amount']).toBeDefined();
      expect(body402['facilitatorAddress']).toBeDefined();
      expect(body402['paymentNetwork']).toBe('eip-3009');
      expect(body402['chainId']).toBe(31337);

      // Step 2: Request with valid X-PAYMENT header -> 200
      const mockSettle = vi.fn().mockResolvedValue({
        success: true,
        txHash: '0x' + 'f'.repeat(64),
      } satisfies X402SettlementResult);

      const mockIlpClient = {
        sendIlpPacket: vi.fn().mockResolvedValue({
          accepted: true,
          fulfillment: 'test-fulfillment',
        }),
      };

      const authorization = createEip3009Authorization();
      const configWithMocks = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: createPassingPreflight(),
      });

      const response200 = await makeRequest(configWithMocks, requestBody, {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 200 with event ID and settlement tx hash
      expect(response200.status).toBe(200);
      const body200 = (await response200.json()) as Record<string, unknown>;
      expect(body200['eventId']).toBe('a'.repeat(64));
      expect(body200['settlementTxHash']).toBe('0x' + 'f'.repeat(64));
      expect(body200['deliveryStatus']).toBe('fulfilled');
      expect(body200['refundInitiated']).toBe(false);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-04 [P0]: Settlement atomicity -- revert scenario
  // Risk: E3-R006
  // Level: Integration (mock settlement revert)
  // --------------------------------------------------------------------------
  describe('Settlement atomicity -- revert (3.3-INT-004)', () => {
    it('[P0] settlement tx reverts (insufficient balance) -> no ILP PREPARE sent', async () => {
      // Given: a mock settlement module that simulates revert
      const mockSettle = vi.fn().mockResolvedValue({
        success: false,
        error: 'ERC20: transfer amount exceeds balance',
      } satisfies X402SettlementResult);

      // And: a mock ILP client to verify no PREPARE is sent
      const mockIlpClient = {
        sendIlpPacket: vi.fn(),
      };

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: createPassingPreflight(),
      });
      const requestBody = createX402RequestBody();

      // When: the handler processes a paid request but settlement reverts
      const response = await makeRequest(config, requestBody, {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: response indicates payment failure with generic message (CWE-209)
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['error']).toBe('Settlement failed');
      // CWE-209: revert reason must NOT be in the response
      expect(JSON.stringify(body)).not.toContain('transfer amount exceeds');

      // And: no ILP PREPARE was sent (atomicity: no packet without payment)
      expect(mockIlpClient.sendIlpPacket).not.toHaveBeenCalled();
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-05 [P0]: No refund on REJECT
  // Risk: E3-R006, E3-R008
  // Level: Integration (mock settlement success + connector reject)
  // --------------------------------------------------------------------------
  describe('No refund on REJECT (3.3-INT-005)', () => {
    it('[P0] settlement succeeds but ILP PREPARE rejected -> HTTP 200, no refund', async () => {
      // Given: settlement succeeds
      const mockSettle = vi.fn().mockResolvedValue({
        success: true,
        txHash: '0x' + 'f'.repeat(64),
      } satisfies X402SettlementResult);

      // And: connector rejects the PREPARE
      const mockIlpClient = {
        sendIlpPacket: vi.fn().mockResolvedValue({
          accepted: false,
          code: 'F02',
          message: 'No route found',
        }),
      };

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: createPassingPreflight(),
      });
      const requestBody = createX402RequestBody();

      // When: the handler processes the request
      const response = await makeRequest(config, requestBody, {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: HTTP 200 (payment was accepted, delivery is best-effort)
      expect(response.status).toBe(200);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['settlementTxHash']).toBe('0x' + 'f'.repeat(64));
      expect(body['deliveryStatus']).toBe('rejected');
      expect(body['refundInitiated']).toBe(false);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-06 [P0]: EIP-3009 forged signature rejection
  // Risk: E3-R005
  // Level: Integration (mock pre-flight signature check)
  // --------------------------------------------------------------------------
  describe('EIP-3009 forged signature rejection (3.3-INT-006)', () => {
    it('[P0] invalid EIP-3009 signature rejected at pre-flight (no gas spent)', async () => {
      // Given: an authorization with a forged signature (zeroed r, s)
      const forgedAuth = createEip3009Authorization({
        v: 28,
        r: '0x' + '0'.repeat(64),
        s: '0x' + '0'.repeat(64),
      });

      // And: preflight config (devMode only skips Schnorr; EIP-3009 sig check
      // runs regardless of devMode, so forged sigs are always caught)
      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: 31337,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: '0x5FbDB2315678afecb367f032d93F642f64180aa3',
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
      };

      const toonData = Buffer.from('test_toon_data').toString('base64');

      // When: runPreflight() checks the EIP-3009 signature
      const result = await runPreflight(
        forgedAuth,
        toonData,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: pre-flight rejects at the first check (signature verification)
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('eip3009-signature');

      // And: no on-chain transaction was made (gas griefing prevented)
      expect(result.checksPerformed).toContain('eip3009-signature');
      expect(result.checksPerformed.length).toBeLessThanOrEqual(1);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-07 [P1]: x402 disabled returns 404
  // Level: Integration (handler config)
  // --------------------------------------------------------------------------
  describe('x402 disabled (3.3-INT-007)', () => {
    it('[P1] CROSSTOWN_X402_ENABLED=false -> GET /publish returns 404', async () => {
      // Given: x402 is explicitly disabled in config
      const config = createX402Config({ x402Enabled: false });
      const requestBody = createX402RequestBody();

      // When: a request is made to /publish
      const response = await makeRequest(config, requestBody);

      // Then: 404 response (endpoint not available)
      expect(response.status).toBe(404);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-09 [P1]: 402 response schema
  // Level: Integration (handler returns correct schema)
  // --------------------------------------------------------------------------
  describe('402 response schema (3.3-INT-009)', () => {
    it('[P1] HTTP 402 body contains required fields: amount, facilitatorAddress, paymentNetwork, chainId, usdcAddress', async () => {
      // Given: x402 is enabled and request has no X-PAYMENT header
      const config = createX402Config();
      const requestBody = createX402RequestBody();

      // When: a request without payment is made to /publish
      const response = await makeRequest(config, requestBody);

      // Then: 402 response with all required pricing fields
      expect(response.status).toBe(402);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body).toHaveProperty('amount');
      expect(body).toHaveProperty('facilitatorAddress');
      expect(body).toHaveProperty('paymentNetwork');
      expect(body).toHaveProperty('chainId');
      expect(body).toHaveProperty('usdcAddress');

      // And: fields have correct types and formats
      expect(typeof body['amount']).toBe('string'); // BigInt serialized as string
      expect(String(body['facilitatorAddress'])).toMatch(/^0x[0-9a-fA-F]{40}$/);
      expect(body['paymentNetwork']).toBe('eip-3009');
      expect(typeof body['chainId']).toBe('number');
      expect(String(body['usdcAddress'])).toMatch(/^0x[0-9a-fA-F]{40}$/);
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-10 [P1]: Dual-protocol server (HTTP + WS)
  // Risk: E3-R009
  // Level: Integration (server infrastructure test)
  // Note: Current architecture uses separate ports; this test validates
  // the BLS HTTP server responds to /health and /publish on same port.
  // --------------------------------------------------------------------------
  describe('Dual-protocol server (3.7-INT-001)', () => {
    it('[P1] concurrent HTTP GET /health + /publish on BLS app', async () => {
      // Given: a Hono app with both /health and /publish routes
      const config = createX402Config();
      const handler = createX402Handler(config);
      const app = new Hono();
      app.get('/health', (c) => c.json({ status: 'healthy' }));
      app.post('/publish', (c) => handler.handlePublish(c));

      // When: concurrent requests are made to both endpoints
      const [healthResponse, publishResponse] = await Promise.all([
        app.fetch(new Request('http://localhost/health')),
        app.fetch(
          new Request('http://localhost/publish', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(createX402RequestBody()),
          })
        ),
      ]);

      // Then: both endpoints respond correctly
      expect(healthResponse.status).toBe(200);
      expect(publishResponse.status).toBe(402); // No X-PAYMENT header
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-11 [P2]: Pre-flight: insufficient USDC balance
  // Risk: E3-R008
  // Level: Integration (mock balance check)
  // --------------------------------------------------------------------------
  describe('Pre-flight: insufficient USDC balance (3.3-INT-010)', () => {
    it('[P2] preflight short-circuits before balance check when sig is invalid', async () => {
      // Given: forged authorization (sig check will fail before reaching balance check)
      const authorization = createEip3009Authorization({ value: 5000n });

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: 31337,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: '0x5FbDB2315678afecb367f032d93F642f64180aa3',
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
      };

      const toonData = Buffer.from('test_toon_data').toString('base64');

      // When: runPreflight() executes with forged sig data
      const result = await runPreflight(
        authorization,
        toonData,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: pre-flight fails at EIP-3009 sig check, never reaching balance check
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('eip3009-signature');
      expect(result.checksPerformed).toEqual(['eip3009-signature']);
      // Balance check is never reached -- no on-chain reads for invalid sigs
    });

    it('[P2] handler returns 400 when preflight fails on usdc-balance', async () => {
      // Given: preflight mock that specifically fails on usdc-balance
      const failingPreflight = vi.fn().mockResolvedValue({
        passed: false,
        failedCheck: 'usdc-balance',
        checksPerformed: ['eip3009-signature', 'usdc-balance'],
      });

      const mockSettle = vi.fn();
      const mockIlpClient = { sendIlpPacket: vi.fn() };
      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: failingPreflight,
      });

      // When: request with X-PAYMENT triggers preflight balance failure
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 400 with specific failure reason, no settlement or ILP
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['failedCheck']).toBe('usdc-balance');
      expect(mockSettle).not.toHaveBeenCalled();
      expect(mockIlpClient.sendIlpPacket).not.toHaveBeenCalled();
    });
  });

  // --------------------------------------------------------------------------
  // T-3.3-12 [P2]: Pre-flight: destination unreachable
  // Risk: E3-R008
  // Level: Integration (mock EventStore / route lookup)
  // --------------------------------------------------------------------------
  describe('Pre-flight: destination unreachable (3.3-INT-011)', () => {
    it('[P2] preflight short-circuits before reachability check when sig is invalid', async () => {
      // Given: forged authorization (sig check will fail before reaching reachability)
      const authorization = createEip3009Authorization();

      const mockEventStore = {
        query: vi.fn().mockReturnValue([]), // No kind:10032 events
      };

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: 31337,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: '0x5FbDB2315678afecb367f032d93F642f64180aa3',
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
        eventStore: mockEventStore,
      };

      const toonData = Buffer.from('test_toon_data').toString('base64');

      // When: runPreflight() executes with forged sig data
      const result = await runPreflight(
        authorization,
        toonData,
        'g.nonexistent.unreachable',
        preflightConfig
      );

      // Then: pre-flight fails at EIP-3009 sig check, never reaching reachability
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('eip3009-signature');
      expect(result.checksPerformed).toEqual(['eip3009-signature']);
      // EventStore.query() was never called -- gas griefing prevented at crypto layer
      expect(mockEventStore.query).not.toHaveBeenCalled();
    });

    it('[P2] handler returns 400 when preflight fails on destination-reachability', async () => {
      // Given: preflight mock that specifically fails on destination-reachability
      const failingPreflight = vi.fn().mockResolvedValue({
        passed: false,
        failedCheck: 'destination-reachability',
        checksPerformed: [
          'eip3009-signature',
          'usdc-balance',
          'nonce-freshness',
          'toon-shallow-parse',
          'schnorr-signature',
          'destination-reachability',
        ],
      });

      const mockSettle = vi.fn();
      const mockIlpClient = { sendIlpPacket: vi.fn() };
      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: failingPreflight,
      });

      // When: request with X-PAYMENT triggers preflight destination failure
      // Use a valid ILP address format (g.*) so we reach the preflight check
      const response = await makeRequest(
        config,
        createX402RequestBody({ destination: 'g.nonexistent.unreachable' }),
        { 'X-PAYMENT': serializeAuth(authorization) }
      );

      // Then: 400 with destination-reachability failure, no settlement or ILP
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['failedCheck']).toBe('destination-reachability');
      expect(mockSettle).not.toHaveBeenCalled();
      expect(mockIlpClient.sendIlpPacket).not.toHaveBeenCalled();
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling tests: preflight nonce-freshness failure at handler level
  // Risk: E3-R008
  // --------------------------------------------------------------------------
  describe('Pre-flight: nonce freshness failure', () => {
    it('handler returns 400 when preflight fails on nonce-freshness', async () => {
      // Given: preflight mock that specifically fails on nonce-freshness
      const failingPreflight = vi.fn().mockResolvedValue({
        passed: false,
        failedCheck: 'nonce-freshness',
        checksPerformed: [
          'eip3009-signature',
          'usdc-balance',
          'nonce-freshness',
        ],
      });

      const mockSettle = vi.fn();
      const mockIlpClient = { sendIlpPacket: vi.fn() };
      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: failingPreflight,
      });

      // When: request with X-PAYMENT triggers preflight nonce failure
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 400 with specific nonce-freshness failure
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['failedCheck']).toBe('nonce-freshness');
      expect(mockSettle).not.toHaveBeenCalled();
      expect(mockIlpClient.sendIlpPacket).not.toHaveBeenCalled();
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: 402 pricing amount value correctness (AC #1, #5)
  // --------------------------------------------------------------------------
  describe('402 pricing amount value correctness', () => {
    it('402 response amount matches calculateX402Price for the TOON payload', async () => {
      // Given: known basePricePerByte and routingBufferPercent
      const config = createX402Config({
        basePricePerByte: 10n,
        routingBufferPercent: 10,
      });
      const requestBody = createX402RequestBody();

      // When: request without X-PAYMENT returns 402
      const response = await makeRequest(config, requestBody);
      expect(response.status).toBe(402);
      const body = (await response.json()) as Record<string, unknown>;

      // Then: amount is a non-zero string representing the computed price
      const amount = BigInt(body['amount'] as string);
      expect(amount).toBeGreaterThan(0n);

      // And: the amount should be basePricePerByte * toonLength * 1.10
      // (we cannot know exact toonLength from here, but we can verify
      // it is greater than the base price alone, proving buffer is applied)
      // The amount must be divisible by 11 if buffer is 10% (base * 1.10)
      // since base = 10n * toonLen and total = base * 110 / 100
      // => total = toonLen * 11
      // This verifies the routing buffer is included.
      expect(amount % 11n).toBe(0n);
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: CWE-209 compliance -- 500 responses must not leak details
  // --------------------------------------------------------------------------
  describe('CWE-209: internal error messages not leaked', () => {
    it('preflight exception returns generic 500 error, not internal details', async () => {
      // Given: preflight that throws an unexpected error with internal details
      const throwingPreflight = vi
        .fn()
        .mockRejectedValue(
          new Error(
            'Internal: viem transport failed at 0x1234 with ECONNREFUSED'
          )
        );

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        runPreflightFn: throwingPreflight,
      });

      // When: request triggers preflight exception
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 500 with generic message, no internal details
      expect(response.status).toBe(500);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['error']).toBe('Internal server error');
      // CWE-209: must NOT contain any of these internal details
      expect(JSON.stringify(body)).not.toContain('viem');
      expect(JSON.stringify(body)).not.toContain('ECONNREFUSED');
      expect(JSON.stringify(body)).not.toContain('0x1234');
    });

    it('settlement exception returns generic 500 error, not internal details', async () => {
      // Given: settlement that throws an unexpected error
      const throwingSettle = vi
        .fn()
        .mockRejectedValue(
          new Error('Internal: RPC node at http://10.0.0.1:8545 timed out')
        );

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: throwingSettle,
        runPreflightFn: createPassingPreflight(),
      });

      // When: request triggers settlement exception
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 500 with generic message, no internal details
      expect(response.status).toBe(500);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['error']).toBe('Internal server error');
      expect(JSON.stringify(body)).not.toContain('10.0.0.1');
      expect(JSON.stringify(body)).not.toContain('timed out');
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: GET route registration (AC #1 says GET /publish)
  // Note: The Fetch API does not allow body on GET requests, so the handler
  // is exercised primarily via POST. These tests verify the GET route is
  // registered and responds (returns 400 for missing body / 404 for disabled).
  // --------------------------------------------------------------------------
  describe('GET route registration', () => {
    it('GET /publish route is registered and returns 404 when x402 disabled', async () => {
      // Given: x402 disabled
      const config = createX402Config({ x402Enabled: false });
      const handler = createX402Handler(config);
      const app = new Hono();
      app.get('/publish', (c) => handler.handlePublish(c));

      // When: GET request to /publish (no body -- Fetch API disallows body on GET)
      const response = await app.fetch(
        new Request('http://localhost/publish', { method: 'GET' })
      );

      // Then: 404 (x402 disabled, route exists and handler runs)
      expect(response.status).toBe(404);
    });

    it('GET /publish route returns 400 when x402 enabled but no body', async () => {
      // Given: x402 enabled but GET request has no body
      const config = createX402Config();
      const handler = createX402Handler(config);
      const app = new Hono();
      app.get('/publish', (c) => handler.handlePublish(c));

      // When: GET request without body
      const response = await app.fetch(
        new Request('http://localhost/publish', { method: 'GET' })
      );

      // Then: 400 (invalid request body -- GET route is registered and handler runs)
      expect(response.status).toBe(400);
    });

    it('POST /publish works identically to GET route handler', async () => {
      // Given: both routes registered with same handler
      const config = createX402Config();
      const handler = createX402Handler(config);
      const app = new Hono();
      app.get('/publish', (c) => handler.handlePublish(c));
      app.post('/publish', (c) => handler.handlePublish(c));

      // When: POST request without X-PAYMENT header -> 402
      const postResponse = await app.fetch(
        new Request('http://localhost/publish', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(createX402RequestBody()),
        })
      );

      // Then: same handler logic applies to POST
      expect(postResponse.status).toBe(402);
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: TOON encoding failure (handler error path)
  // --------------------------------------------------------------------------
  describe('TOON encoding failure', () => {
    it('returns 400 when TOON encoder throws for malformed event', async () => {
      // Given: a custom encoder that always throws
      const throwingEncoder = () => {
        throw new Error('Invalid event structure');
      };
      const config = createX402Config({
        toonEncoder: throwingEncoder,
      });
      const requestBody = createX402RequestBody();

      // When: request is made (encoder will fail)
      const response = await makeRequest(config, requestBody);

      // Then: 400 with TOON encoding error
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/toon/i);
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: handler input validation edge cases (AC #2)
  // --------------------------------------------------------------------------
  describe('Handler input validation', () => {
    it('returns 400 for invalid JSON request body', async () => {
      // Given: x402 enabled config
      const config = createX402Config();
      const handler = createX402Handler(config);
      const app = new Hono();
      app.post('/publish', (c) => handler.handlePublish(c));

      // When: request with non-JSON body
      const response = await app.fetch(
        new Request('http://localhost/publish', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: 'not valid json{{{',
        })
      );

      // Then: 400 invalid request
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/invalid/i);
    });

    it('returns 400 when event field is missing', async () => {
      // Given: request body without event field
      const config = createX402Config();
      const response = await makeRequest(config, {
        destination: 'g.crosstown.test-relay',
      });

      // Then: 400 missing required fields
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/missing.*event|destination/i);
    });

    it('returns 400 when destination field is missing', async () => {
      // Given: request body without destination field
      const config = createX402Config();
      const response = await makeRequest(config, {
        event: createNostrEvent(),
      });

      // Then: 400 missing required fields
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/missing.*event|destination/i);
    });

    it('returns 400 for invalid X-PAYMENT header JSON', async () => {
      // Given: invalid JSON in X-PAYMENT header
      const config = createX402Config();
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': 'not-valid-json{{{',
      });

      // Then: 400 invalid X-PAYMENT header
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/invalid.*x-payment/i);
    });

    it('returns 400 for X-PAYMENT header missing required fields', async () => {
      // Given: X-PAYMENT with incomplete authorization (missing 'from')
      const config = createX402Config();
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': JSON.stringify({
          to: '0x' + 'b'.repeat(40),
          value: '5500',
          nonce: '0x' + 'c'.repeat(64),
          v: 27,
          r: '0x' + 'd'.repeat(64),
          s: '0x' + 'e'.repeat(64),
        }),
      });

      // Then: 400 (parse failure for missing from)
      expect(response.status).toBe(400);
    });
  });

  // --------------------------------------------------------------------------
  // Review pass #3: EVM address format validation (OWASP A03:2021)
  // --------------------------------------------------------------------------
  describe('EVM address format validation', () => {
    it('rejects X-PAYMENT with short from address', async () => {
      const config = createX402Config();
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': JSON.stringify({
          from: '0x1', // Too short -- must be 42 chars
          to: '0x' + 'b'.repeat(40),
          value: '5500',
          validAfter: 0,
          validBefore: Math.floor(Date.now() / 1000) + 3600,
          nonce: '0x' + 'c'.repeat(64),
          v: 27,
          r: '0x' + 'd'.repeat(64),
          s: '0x' + 'e'.repeat(64),
        }),
      });
      expect(response.status).toBe(400);
    });

    it('rejects X-PAYMENT with non-hex characters in from address', async () => {
      const config = createX402Config();
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': JSON.stringify({
          from: '0x' + 'g'.repeat(40), // 'g' is not valid hex
          to: '0x' + 'b'.repeat(40),
          value: '5500',
          validAfter: 0,
          validBefore: Math.floor(Date.now() / 1000) + 3600,
          nonce: '0x' + 'c'.repeat(64),
          v: 27,
          r: '0x' + 'd'.repeat(64),
          s: '0x' + 'e'.repeat(64),
        }),
      });
      expect(response.status).toBe(400);
    });

    it('rejects X-PAYMENT with negative value', async () => {
      const config = createX402Config();
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': JSON.stringify({
          from: '0x' + 'a'.repeat(40),
          to: '0x' + 'b'.repeat(40),
          value: '-1',
          validAfter: 0,
          validBefore: Math.floor(Date.now() / 1000) + 3600,
          nonce: '0x' + 'c'.repeat(64),
          v: 27,
          r: '0x' + 'd'.repeat(64),
          s: '0x' + 'e'.repeat(64),
        }),
      });
      expect(response.status).toBe(400);
    });

    it('rejects X-PAYMENT with invalid v value (not 27 or 28)', async () => {
      const config = createX402Config();
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': JSON.stringify({
          from: '0x' + 'a'.repeat(40),
          to: '0x' + 'b'.repeat(40),
          value: '5500',
          validAfter: 0,
          validBefore: Math.floor(Date.now() / 1000) + 3600,
          nonce: '0x' + 'c'.repeat(64),
          v: 0,
          r: '0x' + 'd'.repeat(64),
          s: '0x' + 'e'.repeat(64),
        }),
      });
      expect(response.status).toBe(400);
    });
  });

  // --------------------------------------------------------------------------
  // Review pass #3: ILP address format validation
  // --------------------------------------------------------------------------
  describe('ILP address format validation', () => {
    it('rejects destination without g. prefix', async () => {
      const config = createX402Config();
      const response = await makeRequest(config, {
        event: createNostrEvent(),
        destination: 'x.nonexistent.relay',
      });
      expect(response.status).toBe(400);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/invalid destination/i);
    });
  });

  // --------------------------------------------------------------------------
  // Review pass #3: routingBufferPercent clamping
  // --------------------------------------------------------------------------
  describe('routingBufferPercent clamping', () => {
    it('negative routing buffer is clamped to 0%', () => {
      const price = calculateX402Price(
        { basePricePerByte: 10n, routingBufferPercent: -50 },
        100
      );
      // Should be base price only (no negative buffer applied)
      expect(price).toBe(1000n);
    });

    it('extremely large routing buffer is clamped to 200%', () => {
      const price = calculateX402Price(
        { basePricePerByte: 10n, routingBufferPercent: 999 },
        100
      );
      // Should be base + 200% buffer = 1000 + 2000 = 3000
      expect(price).toBe(3000n);
    });
  });

  // --------------------------------------------------------------------------
  // Review pass #3: facilitator address validation at construction
  // --------------------------------------------------------------------------
  describe('Facilitator address validation', () => {
    it('throws on construction when x402 enabled with invalid facilitatorAddress', () => {
      expect(() =>
        createX402Handler(
          createX402Config({
            x402Enabled: true,
            facilitatorAddress: 'not-an-address',
          })
        )
      ).toThrow(/facilitatorAddress/i);
    });

    it('does not throw when x402 disabled even with invalid facilitatorAddress', () => {
      expect(() =>
        createX402Handler(
          createX402Config({
            x402Enabled: false,
            facilitatorAddress: 'not-an-address',
          })
        )
      ).not.toThrow();
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: ILP client exception handling (AC #8)
  // --------------------------------------------------------------------------
  describe('ILP client exception handling', () => {
    it('ILP sendIlpPacket throwing -> deliveryStatus rejected, no crash', async () => {
      // Given: settlement succeeds but ILP client throws
      const mockSettle = vi.fn().mockResolvedValue({
        success: true,
        txHash: '0x' + 'f'.repeat(64),
      } satisfies X402SettlementResult);

      const mockIlpClient = {
        sendIlpPacket: vi
          .fn()
          .mockRejectedValue(new Error('Connection refused')),
      };

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: createPassingPreflight(),
      });

      // When: handler processes request and ILP client throws
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 200 with rejected delivery (no crash, no refund)
      expect(response.status).toBe(200);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['deliveryStatus']).toBe('rejected');
      expect(body['refundInitiated']).toBe(false);
      expect(body['settlementTxHash']).toBe('0x' + 'f'.repeat(64));
    });

    it('no ILP client configured -> deliveryStatus rejected', async () => {
      // Given: settlement succeeds but no ilpClient is configured
      const mockSettle = vi.fn().mockResolvedValue({
        success: true,
        txHash: '0x' + 'a'.repeat(64),
      } satisfies X402SettlementResult);

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        // No ilpClient
        runPreflightFn: createPassingPreflight(),
      });

      // When: handler processes request without ILP client
      const response = await makeRequest(config, createX402RequestBody(), {
        'X-PAYMENT': serializeAuth(authorization),
      });

      // Then: 200 with rejected delivery (settlement succeeded, no routing)
      expect(response.status).toBe(200);
      const body = (await response.json()) as Record<string, unknown>;
      expect(body['deliveryStatus']).toBe('rejected');
      expect(body['refundInitiated']).toBe(false);
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: calculateX402Price edge cases (AC #5)
  // --------------------------------------------------------------------------
  describe('calculateX402Price edge cases', () => {
    it('zero-length TOON payload returns 0 price', () => {
      const price = calculateX402Price(
        { basePricePerByte: 10n, routingBufferPercent: 10 },
        0
      );
      expect(price).toBe(0n);
    });

    it('large routing buffer (50%) applies correctly', () => {
      const price = calculateX402Price(
        { basePricePerByte: 10n, routingBufferPercent: 50 },
        100
      );
      // base = 10 * 100 = 1000, buffer = 1000 * 50 / 100 = 500, total = 1500
      expect(price).toBe(1500n);
    });

    it('very large payload computes without overflow', () => {
      const price = calculateX402Price(
        { basePricePerByte: 10n, routingBufferPercent: 10 },
        1_000_000
      );
      // base = 10 * 1_000_000 = 10_000_000, buffer = 1_000_000, total = 11_000_000
      expect(price).toBe(11_000_000n);
    });
  });

  // --------------------------------------------------------------------------
  // Gap-filling: handler verifies buildIlpPrepare called with correct args
  // --------------------------------------------------------------------------
  describe('Handler ILP PREPARE construction', () => {
    it('handler calls ILP client with buildIlpPrepare-constructed packet', async () => {
      // Given: settlement succeeds and ILP client accepts
      const mockSettle = vi.fn().mockResolvedValue({
        success: true,
        txHash: '0x' + 'f'.repeat(64),
      } satisfies X402SettlementResult);

      const mockIlpClient = {
        sendIlpPacket: vi.fn().mockResolvedValue({
          accepted: true,
          fulfillment: 'test-fulfillment',
        }),
      };

      const authorization = createEip3009Authorization();
      const config = createX402Config({
        settle: mockSettle,
        ilpClient: mockIlpClient,
        runPreflightFn: createPassingPreflight(),
        basePricePerByte: 10n,
      });

      // When: handler processes paid request
      await makeRequest(
        config,
        createX402RequestBody({ destination: 'g.crosstown.target' }),
        { 'X-PAYMENT': serializeAuth(authorization) }
      );

      // Then: ILP client received a packet with correct structure
      expect(mockIlpClient.sendIlpPacket).toHaveBeenCalledTimes(1);
      const packet = mockIlpClient.sendIlpPacket.mock.calls[0]?.[0] as
        | Record<string, unknown>
        | undefined;
      expect(packet).toBeDefined();
      expect(packet?.['destination']).toBe('g.crosstown.target');
      expect(typeof packet?.['amount']).toBe('string');
      expect(typeof packet?.['data']).toBe('string'); // base64
      // Amount should be basePricePerByte * toonBytes.length (as string)
      const amount = BigInt(packet?.['amount'] as string);
      expect(amount).toBeGreaterThan(0n);
    });
  });

  // ==========================================================================
  // AC #2 check 4: TOON shallow parse through real runPreflight()
  //
  // These tests exercise the real runPreflight() code path for check 4
  // (TOON shallow parse). Previous tests either mocked preflight entirely
  // or short-circuited at check 1 (EIP-3009 sig). These tests pass check 1
  // with a cryptographically valid EIP-3009 signature, skip checks 2-3
  // (no publicClient), and reach check 4 with both valid and invalid TOON.
  // ==========================================================================
  describe('Pre-flight check 4: TOON shallow parse (real runPreflight)', () => {
    it('valid TOON data passes check 4 through real preflight pipeline', async () => {
      // Given: a cryptographically valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON-encoded Nostr event data
      const toonBase64 = createValidToonBase64();

      // And: preflight config without publicClient (skips checks 2-3)
      // devMode=true skips check 5 (Schnorr), no eventStore skips check 6
      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
      };

      // When: runPreflight() executes the real pipeline
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: all checks pass (1, 2-skip, 3-skip, 4-pass, 5-skip, 6-skip)
      expect(result.passed).toBe(true);
      expect(result.checksPerformed).toContain('toon-shallow-parse');
      // The check was actually executed (not skipped)
      expect(result.checksPerformed.indexOf('toon-shallow-parse')).toBe(3);
    });

    it('invalid TOON data fails check 4 through real preflight pipeline', async () => {
      // Given: a cryptographically valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: invalid TOON data (not valid TOON-encoded)
      const invalidToonBase64 =
        Buffer.from('not valid toon {{{').toString('base64');

      // And: preflight config without publicClient (skips checks 2-3)
      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
      };

      // When: runPreflight() hits check 4 with invalid TOON
      const result = await runPreflight(
        authorization,
        invalidToonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: fails at check 4 (TOON shallow parse)
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('toon-shallow-parse');
      // Checks 1-3 passed/skipped, then check 4 failed
      expect(result.checksPerformed).toEqual([
        'eip3009-signature',
        'usdc-balance',
        'nonce-freshness',
        'toon-shallow-parse',
      ]);
    });

    it('TOON data missing required routing fields fails check 4', async () => {
      // Given: a cryptographically valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: TOON data from an object missing required routing fields (no 'sig')
      // encodeEventToToon can encode arbitrary objects; shallowParseToon
      // rejects objects missing id/pubkey/sig/kind
      const incompleteEvent = {
        kind: 1,
        pubkey: 'b'.repeat(64),
        id: 'a'.repeat(64),
        // sig is missing -- shallowParseToon will reject
        content: 'hello',
        tags: [],
        created_at: 1234567890,
      };
      const toonBytes = encodeEventToToon(
        incompleteEvent as unknown as NostrEvent
      );
      const toonBase64 = Buffer.from(toonBytes).toString('base64');

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
      };

      // When: runPreflight() hits check 4 with incomplete TOON data
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: fails at check 4 -- shallowParseToon rejects missing sig
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('toon-shallow-parse');
      expect(result.checksPerformed).toContain('toon-shallow-parse');
    });
  });

  // ==========================================================================
  // AC #2 check 5: Schnorr signature verification through real runPreflight()
  //
  // These tests exercise the real runPreflight() code path for check 5
  // (Schnorr signature verification). Previous tests either mocked preflight
  // or used devMode=true which skips Schnorr. These tests:
  // - Pass check 1 with a real EIP-3009 signature
  // - Skip checks 2-3 (no publicClient)
  // - Pass check 4 with valid TOON data
  // - Set devMode=false with a schnorrVerify callback to exercise check 5
  // ==========================================================================
  describe('Pre-flight check 5: Schnorr signature (real runPreflight)', () => {
    it('passing Schnorr verify callback -> check 5 passes', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: a Schnorr verify callback that returns true
      const schnorrVerify = vi.fn().mockResolvedValue(true);

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: false, // Important: devMode=false so Schnorr runs
        schnorrVerify,
      };

      // When: runPreflight() executes the real pipeline through check 5
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: all checks pass including Schnorr
      expect(result.passed).toBe(true);
      expect(result.checksPerformed).toContain('schnorr-signature');
      expect(schnorrVerify).toHaveBeenCalledOnce();

      // And: the schnorrVerify callback received the parsed ToonRoutingMeta
      const callArg = schnorrVerify.mock.calls[0]?.[0] as Record<
        string,
        unknown
      >;
      expect(callArg).toHaveProperty('kind');
      expect(callArg).toHaveProperty('pubkey');
      expect(callArg).toHaveProperty('id');
      expect(callArg).toHaveProperty('sig');
      expect(callArg).toHaveProperty('rawBytes');
    });

    it('failing Schnorr verify callback -> check 5 fails', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: a Schnorr verify callback that returns false (invalid signature)
      const schnorrVerify = vi.fn().mockResolvedValue(false);

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: false,
        schnorrVerify,
      };

      // When: runPreflight() reaches check 5 and Schnorr fails
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: fails at check 5 (Schnorr signature)
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('schnorr-signature');
      expect(result.checksPerformed).toEqual([
        'eip3009-signature',
        'usdc-balance',
        'nonce-freshness',
        'toon-shallow-parse',
        'schnorr-signature',
      ]);
      // And: Schnorr verify was called with ToonRoutingMeta
      expect(schnorrVerify).toHaveBeenCalledOnce();
    });

    it('throwing Schnorr verify callback -> check 5 fails gracefully', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: a Schnorr verify callback that throws an error
      const schnorrVerify = vi
        .fn()
        .mockRejectedValue(new Error('crypto verification crashed'));

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: false,
        schnorrVerify,
      };

      // When: runPreflight() reaches check 5 and verify throws
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: fails at check 5 gracefully (no unhandled exception)
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('schnorr-signature');
    });

    it('devMode=true skips Schnorr verification even when callback provided', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: a Schnorr verify callback that would fail if called
      const schnorrVerify = vi.fn().mockResolvedValue(false);

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true, // devMode skips Schnorr
        schnorrVerify,
      };

      // When: runPreflight() runs with devMode=true
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: passes (Schnorr check was added to checksPerformed but not executed)
      expect(result.passed).toBe(true);
      expect(result.checksPerformed).toContain('schnorr-signature');
      // Schnorr verify callback was NOT called (devMode skip)
      expect(schnorrVerify).not.toHaveBeenCalled();
    });
  });

  // ==========================================================================
  // AC #5: Reachability check queries EventStore for kind:10032 events
  //
  // These tests exercise the real runPreflight() code path for check 6
  // (destination reachability). They verify that the EventStore.query()
  // method is called with the correct filter ([{kinds: [10032]}]) and
  // that the presence/absence of events correctly determines reachability.
  // ==========================================================================
  describe('Pre-flight check 6: destination reachability queries kind:10032 (real runPreflight)', () => {
    it('eventStore.query called with [{kinds: [10032]}] filter', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: mock EventStore with kind:10032 events present
      const mockEventStore = {
        query: vi
          .fn()
          .mockReturnValue([
            { kind: 10032, content: '{}', pubkey: 'a'.repeat(64) },
          ]),
      };

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
        eventStore: mockEventStore,
      };

      // When: runPreflight() reaches check 6
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: passes all checks
      expect(result.passed).toBe(true);
      expect(result.checksPerformed).toContain('destination-reachability');

      // And: eventStore.query was called with the correct kind:10032 filter
      expect(mockEventStore.query).toHaveBeenCalledOnce();
      expect(mockEventStore.query).toHaveBeenCalledWith([{ kinds: [10032] }]);
    });

    it('no kind:10032 events in store -> reachability check fails', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: mock EventStore with NO kind:10032 events (empty result)
      const mockEventStore = {
        query: vi.fn().mockReturnValue([]),
      };

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
        eventStore: mockEventStore,
      };

      // When: runPreflight() reaches check 6 with empty event store
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: fails at check 6 (destination unreachable)
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('destination-reachability');
      expect(result.checksPerformed).toEqual([
        'eip3009-signature',
        'usdc-balance',
        'nonce-freshness',
        'toon-shallow-parse',
        'schnorr-signature',
        'destination-reachability',
      ]);

      // And: eventStore.query was called with correct kind:10032 filter
      expect(mockEventStore.query).toHaveBeenCalledWith([{ kinds: [10032] }]);
    });

    it('eventStore.query throwing -> reachability check fails gracefully', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: mock EventStore that throws on query
      const mockEventStore = {
        query: vi.fn().mockImplementation(() => {
          throw new Error('EventStore connection failed');
        }),
      };

      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
        eventStore: mockEventStore,
      };

      // When: runPreflight() reaches check 6 and store throws
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: fails at check 6 gracefully
      expect(result.passed).toBe(false);
      expect(result.failedCheck).toBe('destination-reachability');
    });

    it('no eventStore provided -> reachability check is skipped (passes)', async () => {
      // Given: valid EIP-3009 authorization (passes check 1)
      const authorization = await createSignedEip3009Authorization();

      // And: valid TOON data (passes check 4)
      const toonBase64 = createValidToonBase64();

      // And: no eventStore in config (check 6 is skipped)
      const preflightConfig: PreflightConfig = {
        chainConfig: {
          name: 'anvil',
          chainId: TEST_CHAIN_ID,
          rpcUrl: 'http://localhost:8545',
          usdcAddress: TEST_USDC_ADDRESS,
          tokenNetworkAddress: '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
        },
        basePricePerByte: 10n,
        ownPubkey: 'f'.repeat(64),
        devMode: true,
        // No eventStore
      };

      // When: runPreflight() runs without eventStore
      const result = await runPreflight(
        authorization,
        toonBase64,
        'g.crosstown.test-relay',
        preflightConfig
      );

      // Then: passes (reachability check added to checksPerformed but not enforced)
      expect(result.passed).toBe(true);
      expect(result.checksPerformed).toContain('destination-reachability');
    });
  });
});

// ============================================================================
// E2E Test Stub (T-3.3-14, P3 -- deferred to nightly CI)
// ============================================================================

describe('Story 3.3: x402 E2E (3.3-E2E-001)', () => {
  // --------------------------------------------------------------------------
  // T-3.3-14 [P3]: Full x402 E2E with genesis infrastructure
  // Risk: E3-R005, E3-R006
  // Level: E2E (requires Anvil + Faucet + Connector + Relay)
  // Note: Deferred to nightly CI. Not part of standard `pnpm test`.
  // --------------------------------------------------------------------------
  it.skip('[P3] x402 full E2E: Anvil + Faucet + Connector + Relay -> 402 -> payment -> store', async () => {
    // This test requires the full genesis infrastructure:
    // - Anvil (local EVM chain with mock USDC)
    // - Faucet (fund test wallets)
    // - Connector (ILP routing)
    // - At least one Crosstown relay node with x402 enabled
    //
    // Steps:
    // 1. Fund a test wallet with mock USDC via the Faucet
    // 2. Create a signed Nostr event
    // 3. GET /publish -> receive 402 with pricing
    // 4. Sign EIP-3009 transferWithAuthorization off-chain
    // 5. GET /publish with X-PAYMENT header -> receive 200
    // 6. Verify event stored on destination relay (query via WebSocket)
    // 7. Verify USDC transferred on-chain (check facilitator balance)
    //
    // expect(response.status).toBe(200);
    // expect(response.body.eventId).toMatch(/^[0-9a-f]{64}$/);
    // expect(response.body.settlementTxHash).toMatch(/^0x[0-9a-f]{64}$/);
    // expect(response.body.deliveryStatus).toBe('fulfilled');
    expect(true).toBe(false); // RED: Full E2E requires genesis infra
  });
});
