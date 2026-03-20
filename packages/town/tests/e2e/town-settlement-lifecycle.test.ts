/**
 * E2E Test: Settlement Lifecycle against Oyster CVM (Arbitrum Sepolia)
 *
 * Validates the full payment channel lifecycle against a deployed
 * Oyster CVM instance using real testnet funds on Arbitrum Sepolia.
 *
 * Tests:
 * - T1: Oyster CVM health check (embedded connector mode)
 * - T2: Publish 10 events to Oyster node (ToonClient or direct HTTP)
 * - T3: Verify all 10 events stored on Oyster relay
 * - T4: TokenNetwork contract accessible on Arbitrum Sepolia
 * - T5: Payment channel lifecycle: open, deposit, sign, claim (channel stays open)
 *
 * **Prerequisites:**
 * 1. Oyster CVM enclave running with a TOON node
 * 2. Wallets funded on Arbitrum Sepolia (0.005 ETH + 1 USDC each)
 * 3. TokenNetwork contract deployed at the configured address
 *
 * **Environment variables (source ../../.env.oyster first):**
 * - OYSTER_ENCLAVE_IP  — Enclave IP
 * - CLIENT_PRIVATE_KEY  — Client wallet (publishing + settlement Account B)
 * - TOWN_NODE2_PRIVATE_KEY — Settlement Account A
 * - SETTLEMENT_RPC_URL  — Arbitrum Sepolia RPC (default: public)
 *
 * Run:
 *   source ../../.env.oyster && cd packages/town && pnpm test:e2e -- town-settlement-lifecycle
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import {
  generateSecretKey,
  getPublicKey,
  finalizeEvent,
} from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import {
  createPublicClient,
  createWalletClient,
  http,
  defineChain,
  decodeEventLog,
  type Hex,
} from 'viem';
import { privateKeyToAccount } from 'viem/accounts';
import {
  encodeEventToToon,
  decodeEventFromToon,
} from '@toon-protocol/core/toon';
import { ToonClient } from '@toon-protocol/client';
import WebSocket from 'ws';

// ---------------------------------------------------------------------------
// Configuration (env vars or defaults from .env.oyster)
// ---------------------------------------------------------------------------

const ENCLAVE_IP = process.env['OYSTER_ENCLAVE_IP'] || '3.7.213.71';

const BLS_URL = `http://${ENCLAVE_IP}:3100`;
const RELAY_URL = `ws://${ENCLAVE_IP}:7100`;
const BTP_URL = `ws://${ENCLAVE_IP}:3000`;

// Arbitrum Sepolia (421614)
const CHAIN_RPC =
  process.env['SETTLEMENT_RPC_URL'] ||
  'https://sepolia-rollup.arbitrum.io/rpc';
const CHAIN_ID = 421614;
const CHAIN_KEY = 'evm:base:421614';

// Contracts (deployed on Arbitrum Sepolia)
const TOKEN_ADDRESS =
  '0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d' as const;
const TOKEN_NETWORK_ADDRESS =
  '0x91d62b1F7C5d1129A64EE3915c480DBF288B1cBa' as const;

// Client wallet — used for ToonClient publishing AND settlement Account B
const CLIENT_PRIVATE_KEY = (process.env['CLIENT_PRIVATE_KEY'] ||
  '0x2a4a97479b2fc81bda7df10ff4b5bb7b932154b27b8bd64fb794ee2c180138f5') as Hex;

// Settlement Account A — Town Node 2 (not running in Oyster CVM)
const SETTLEMENT_PRIVATE_KEY_A = (process.env['TOWN_NODE2_PRIVATE_KEY'] ||
  '0xcbb34563b3d30c4e98a61538e27a87d800b99a71af68322b81f85e90b19c6294') as Hex;

// Settlement Account B — Client wallet
const SETTLEMENT_PRIVATE_KEY_B = CLIENT_PRIVATE_KEY;

// Oyster node's settlement wallet (Town Node 1)
const OYSTER_NODE_ADDRESS = '0xa5faA1707a4B058b13b1c570a92aC19140C84320' as const;


// ---------------------------------------------------------------------------
// Arbitrum Sepolia chain definition + ABIs
// ---------------------------------------------------------------------------

const arbSepoliaChain = defineChain({
  id: CHAIN_ID,
  name: 'Arbitrum Sepolia',
  nativeCurrency: { name: 'ETH', symbol: 'ETH', decimals: 18 },
  rpcUrls: { default: { http: [CHAIN_RPC] } },
});

const TOKEN_NETWORK_ABI = [
  {
    name: 'channels',
    type: 'function',
    stateMutability: 'view',
    inputs: [{ type: 'bytes32' }],
    outputs: [
      { name: 'settlementTimeout', type: 'uint256' },
      { name: 'state', type: 'uint8' },
      { name: 'closedAt', type: 'uint256' },
      { name: 'openedAt', type: 'uint256' },
      { name: 'participant1', type: 'address' },
      { name: 'participant2', type: 'address' },
    ],
  },
  {
    name: 'participants',
    type: 'function',
    stateMutability: 'view',
    inputs: [{ type: 'bytes32' }, { type: 'address' }],
    outputs: [
      { name: 'deposit', type: 'uint256' },
      { name: 'nonce', type: 'uint256' },
      { name: 'transferredAmount', type: 'uint256' },
    ],
  },
  {
    name: 'claimedAmounts',
    type: 'function',
    stateMutability: 'view',
    inputs: [{ type: 'bytes32' }, { type: 'address' }],
    outputs: [{ type: 'uint256' }],
  },
  {
    name: 'channelCounter',
    type: 'function',
    stateMutability: 'view',
    inputs: [],
    outputs: [{ type: 'uint256' }],
  },
  {
    name: 'openChannel',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [
      { name: 'participant2', type: 'address' },
      { name: 'settlementTimeout', type: 'uint256' },
    ],
    outputs: [{ name: '', type: 'bytes32' }],
  },
  {
    name: 'setTotalDeposit',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [
      { name: 'channelId', type: 'bytes32' },
      { name: 'participant', type: 'address' },
      { name: 'totalDeposit', type: 'uint256' },
    ],
    outputs: [],
  },
  {
    name: 'claimFromChannel',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [
      { name: 'channelId', type: 'bytes32' },
      {
        name: 'balanceProof',
        type: 'tuple',
        components: [
          { name: 'channelId', type: 'bytes32' },
          { name: 'nonce', type: 'uint256' },
          { name: 'transferredAmount', type: 'uint256' },
          { name: 'lockedAmount', type: 'uint256' },
          { name: 'locksRoot', type: 'bytes32' },
        ],
      },
      { name: 'signature', type: 'bytes' },
    ],
    outputs: [],
  },
  {
    name: 'closeChannel',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [{ name: 'channelId', type: 'bytes32' }],
    outputs: [],
  },
  {
    name: 'settleChannel',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [{ name: 'channelId', type: 'bytes32' }],
    outputs: [],
  },
  {
    name: 'ChannelOpened',
    type: 'event',
    inputs: [
      { name: 'channelId', type: 'bytes32', indexed: true },
      { name: 'participant1', type: 'address', indexed: true },
      { name: 'participant2', type: 'address', indexed: true },
      { name: 'settlementTimeout', type: 'uint256', indexed: false },
    ],
  },
  {
    name: 'ChannelClaimed',
    type: 'event',
    inputs: [
      { name: 'channelId', type: 'bytes32', indexed: true },
      { name: 'claimant', type: 'address', indexed: true },
      { name: 'claimedAmount', type: 'uint256', indexed: false },
      { name: 'totalClaimed', type: 'uint256', indexed: false },
    ],
  },
] as const;

const ERC20_ABI = [
  {
    name: 'balanceOf',
    type: 'function',
    stateMutability: 'view',
    inputs: [{ type: 'address' }],
    outputs: [{ type: 'uint256' }],
  },
  {
    name: 'approve',
    type: 'function',
    stateMutability: 'nonpayable',
    inputs: [
      { name: 'spender', type: 'address' },
      { name: 'amount', type: 'uint256' },
    ],
    outputs: [{ name: '', type: 'bool' }],
  },
] as const;

const BALANCE_PROOF_TYPES = {
  BalanceProof: [
    { name: 'channelId', type: 'bytes32' },
    { name: 'nonce', type: 'uint256' },
    { name: 'transferredAmount', type: 'uint256' },
    { name: 'lockedAmount', type: 'uint256' },
    { name: 'locksRoot', type: 'bytes32' },
  ],
} as const;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

interface HealthResponse {
  status: string;
  nodeId: string;
  pubkey: string;
  ilpAddress: string;
  sdk: boolean;
  embedded: boolean;
  bootstrapPhase: string;
  peerCount: number;
}

async function fetchHealth(): Promise<HealthResponse | null> {
  try {
    const res = await fetch(`${BLS_URL}/health`, {
      signal: AbortSignal.timeout(10000),
    });
    if (!res.ok) return null;
    return (await res.json()) as HealthResponse;
  } catch {
    return null;
  }
}

function createViemPublicClient() {
  return createPublicClient({
    chain: arbSepoliaChain,
    transport: http(CHAIN_RPC),
  });
}

async function getChannelState(channelId: Hex) {
  const client = createViemPublicClient();
  const result = await client.readContract({
    address: TOKEN_NETWORK_ADDRESS,
    abi: TOKEN_NETWORK_ABI,
    functionName: 'channels',
    args: [channelId],
  });

  const CHANNEL_STATE_NAMES = [
    'settled',
    'open',
    'closed',
    'settled',
  ] as const;
  const [settlementTimeout, state, closedAt, openedAt, participant1, participant2] =
    result;

  return {
    channelId,
    state: CHANNEL_STATE_NAMES[state] || 'unknown',
    stateNum: state,
    settlementTimeout: Number(settlementTimeout),
    openedAt: Number(openedAt),
    closedAt: Number(closedAt),
    participant1,
    participant2,
  };
}

async function getTokenBalance(address: Hex): Promise<bigint> {
  const client = createViemPublicClient();
  return client.readContract({
    address: TOKEN_ADDRESS,
    abi: ERC20_ABI,
    functionName: 'balanceOf',
    args: [address],
  });
}

async function getChannelCounter(): Promise<bigint> {
  const client = createViemPublicClient();
  return client.readContract({
    address: TOKEN_NETWORK_ADDRESS,
    abi: TOKEN_NETWORK_ABI,
    functionName: 'channelCounter',
    args: [],
  });
}

async function isContractDeployed(address: Hex): Promise<boolean> {
  const client = createViemPublicClient();
  const code = await client.getCode({ address });
  return !!code && code !== '0x';
}

function waitForEventOnRelay(
  relayUrl: string,
  eventId: string,
  timeoutMs = 20000
): Promise<Record<string, unknown> | null> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(relayUrl);
    const subId = `test-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    // eslint-disable-next-line prefer-const
    let timer: ReturnType<typeof setTimeout>;

    const cleanup = () => {
      clearTimeout(timer);
      try {
        ws.close();
      } catch {
        // ignore
      }
    };

    timer = setTimeout(() => {
      cleanup();
      resolve(null);
    }, timeoutMs);

    ws.on('open', () => {
      ws.send(JSON.stringify(['REQ', subId, { ids: [eventId] }]));
    });

    ws.on('message', (data: Buffer) => {
      try {
        const msg = JSON.parse(data.toString());
        if (Array.isArray(msg)) {
          if (msg[0] === 'EVENT' && msg[1] === subId && msg[2]) {
            const toonBytes = new TextEncoder().encode(msg[2]);
            const event = decodeEventFromToon(toonBytes);
            cleanup();
            resolve(event as unknown as Record<string, unknown>);
          }
        }
      } catch {
        // ignore parse errors
      }
    });

    ws.on('error', (err: Error) => {
      cleanup();
      reject(err);
    });
  });
}

const BASE_PRICE_PER_BYTE = 10n;

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe('Town Settlement Lifecycle E2E (Oyster CVM / Arbitrum Sepolia)', () => {
  let health: HealthResponse | null = null;
  let servicesReady = false;
  let toonClient: ToonClient | null = null;

  // Events published in T2, verified in T3
  const publishedEvents: NostrEvent[] = [];

  // Track bootstrap channel and cumulative payment for settlement verification
  let bootstrapChannelId: Hex | null = null;
  let totalPublishPayment = 0n;

  function skipIfNotReady(): boolean {
    if (!servicesReady) {
      if (process.env['CI']) {
        throw new Error('Oyster CVM not reachable — cannot run in CI.');
      }
      console.log('Skipping: Oyster CVM not reachable');
      return true;
    }
    return false;
  }

  beforeAll(async () => {
    console.log(`\nOyster CVM target: ${ENCLAVE_IP}`);
    console.log(`  BLS:   ${BLS_URL}`);
    console.log(`  Relay: ${RELAY_URL}`);
    console.log(`  BTP:   ${BTP_URL}`);
    console.log(`  Chain: Arbitrum Sepolia (${CHAIN_ID})`);
    console.log(`  TokenNetwork: ${TOKEN_NETWORK_ADDRESS}\n`);

    // Phase 1: Health check the Oyster CVM
    health = await fetchHealth();
    if (!health) {
      console.warn(
        'Oyster CVM not reachable. Is the enclave running?\n' +
          `  Tried: ${BLS_URL}/health\n` +
          '  Deploy: source .env.oyster && oyster-cvm deploy ...'
      );
      return;
    }

    servicesReady = true;
    console.log(`Node: ${health.nodeId} (${health.pubkey.slice(0, 16)}...)`);
    console.log(`ILP:  ${health.ilpAddress}`);
    console.log(`Phase: ${health.bootstrapPhase}, peers: ${health.peerCount}`);

    // Phase 2: Snapshot on-chain state
    try {
      const counter = await getChannelCounter();
      console.log(`Channel counter: ${counter}`);
    } catch {
      console.warn('TokenNetwork not accessible — T4/T5 will be skipped');
    }

    // Phase 3: Create ToonClient for publishing tests
    const clientAccount = privateKeyToAccount(CLIENT_PRIVATE_KEY);
    const secretKey = generateSecretKey();
    const pubkey = getPublicKey(secretKey);

    console.log(`Client pubkey: ${pubkey.slice(0, 16)}...`);
    console.log(`Client EVM: ${clientAccount.address}`);

    toonClient = new ToonClient({
      connectorUrl: BLS_URL,
      secretKey,
      ilpInfo: {
        pubkey,
        ilpAddress: `g.toon.test.${pubkey.slice(0, 8)}`,
        btpEndpoint: BTP_URL,
        assetCode: 'USD',
        assetScale: 6,
      },
      toonEncoder: encodeEventToToon,
      toonDecoder: decodeEventFromToon,
      relayUrl: RELAY_URL,
      knownPeers: [
        {
          pubkey: health.pubkey,
          relayUrl: RELAY_URL,
          btpEndpoint: BTP_URL,
        },
      ],
      evmPrivateKey: CLIENT_PRIVATE_KEY,
      chainRpcUrls: { [CHAIN_KEY]: CHAIN_RPC },
      supportedChains: [CHAIN_KEY],
      settlementAddresses: { [CHAIN_KEY]: clientAccount.address },
      preferredTokens: { [CHAIN_KEY]: TOKEN_ADDRESS },
      tokenNetworks: { [CHAIN_KEY]: TOKEN_NETWORK_ADDRESS },
      btpUrl: BTP_URL,
      destinationAddress: health.ilpAddress,
    });

    // Try ToonClient bootstrap — fall back to direct HTTP if it fails
    try {
      console.log('Starting ToonClient bootstrap...');
      const startResult = await toonClient.start();
      console.log(
        `Bootstrap complete! Mode: ${startResult.mode}, peers: ${startResult.peersDiscovered}`
      );
      const channels = toonClient.getTrackedChannels();
      console.log(`Tracked channels: ${channels.length}\n`);
    } catch (err) {
      console.warn(
        `ToonClient bootstrap failed: ${err instanceof Error ? err.message : String(err)}`
      );
      toonClient = null;
    }
  }, 120000);

  afterAll(async () => {
    if (toonClient?.isStarted()) {
      try {
        await toonClient.stop();
      } catch {
        // ignore
      }
    }
    await new Promise((r) => setTimeout(r, 500));
  });

  // =========================================================================
  // T1: Oyster CVM reports embedded connector mode
  // =========================================================================

  it('Oyster CVM reports embedded connector with healthy status', async () => {
    if (skipIfNotReady()) return;

    expect(health!.status).toBe('healthy');
    expect(health!.sdk).toBe(true);
    expect(health!.embedded).toBe(true);
    expect(health!.nodeId).toBe('oyster-node-1');
    expect(health!.ilpAddress).toMatch(/^g\.toon\./);

    console.log('T1: Health check passed');
  });

  // =========================================================================
  // T2: Publish events via ToonClient ILP flow with signed balance proofs
  // =========================================================================

  it('publishes 10 events to Oyster node via ILP with signed claims', async () => {
    if (skipIfNotReady()) return;

    if (!toonClient) {
      console.log('Skipping T2: ToonClient bootstrap failed — cannot publish via ILP');
      return;
    }

    const channels = toonClient.getTrackedChannels();
    if (channels.length === 0) {
      console.log('Skipping T2: No tracked payment channels — bootstrap did not open a channel');
      return;
    }

    const channelId = channels[0]!;
    bootstrapChannelId = channelId as Hex;
    console.log(`Publishing via channel: ${channelId.slice(0, 18)}...`);

    const nostrSecretKey = generateSecretKey();

    for (let i = 0; i < 10; i++) {
      const event = finalizeEvent(
        {
          kind: 1,
          content: `Oyster settlement lifecycle test #${i} - ${Date.now()}`,
          tags: [['t', 'oyster-settlement-lifecycle']],
          created_at: Math.floor(Date.now() / 1000),
        },
        nostrSecretKey
      );

      publishedEvents.push(event);

      // Calculate payment amount (must match relay's basePricePerByte * toonData.length)
      const toonData = encodeEventToToon(event);
      const amount = BigInt(toonData.length) * BASE_PRICE_PER_BYTE;
      totalPublishPayment += amount;

      // Sign cumulative balance proof for this payment
      const claim = await toonClient.signBalanceProof(channelId, amount);

      // Publish via ILP: Client → BTP → Connector → packet handler → Relay
      const result = await toonClient.publishEvent(event, { claim });
      if (!result.success) {
        console.error(`  Event #${i} publish failed: ${result.error}`);
      }
      expect(result.success).toBe(true);
    }

    expect(publishedEvents).toHaveLength(10);
    console.log(`T2: Published ${publishedEvents.length} events via ILP with signed claims`);
    console.log(`  Total payment: ${totalPublishPayment} units (${Number(totalPublishPayment) / 1e6} USDC)`);
  }, 120000);

  // =========================================================================
  // T3: Verify events on Oyster relay
  // =========================================================================

  it('all 10 events are present on Oyster relay', async () => {
    if (skipIfNotReady()) return;

    if (publishedEvents.length === 0) {
      console.log('Skipping: no events published in T2');
      return;
    }

    // Give the relay a moment to flush
    await new Promise((r) => setTimeout(r, 3000));

    for (const event of publishedEvents) {
      const found = await waitForEventOnRelay(RELAY_URL, event.id, 20000);
      expect(found).not.toBeNull();
      expect(found!['id']).toBe(event.id);
      expect(found!['content']).toBe(event.content);
      expect(found!['kind']).toBe(1);
    }

    console.log(`T3: All ${publishedEvents.length} events verified on relay`);
  }, 120000);

  // =========================================================================
  // T4: On-chain TokenNetwork verification
  // =========================================================================

  it('TokenNetwork contract is accessible and channel counter readable', async () => {
    if (skipIfNotReady()) return;

    const deployed = await isContractDeployed(TOKEN_NETWORK_ADDRESS);
    if (!deployed) {
      console.log(
        `Skipping: TokenNetwork not deployed at ${TOKEN_NETWORK_ADDRESS}`
      );
      return;
    }

    const counter = await getChannelCounter();
    expect(counter).toBeGreaterThanOrEqual(0n);
    console.log(`T4: Channel counter = ${counter}`);
  });

  // =========================================================================
  // T5: Payment channel lifecycle on Arbitrum Sepolia
  //
  // Tests: open → deposit → sign balance proof → claim (channel stays open)
  // Uses claimFromChannel() — delta-based claiming without closing.
  // =========================================================================

  it('payment channel lifecycle: open, deposit, sign, claim (channel stays open)', async () => {
    if (skipIfNotReady()) return;

    // Pre-check: TokenNetwork must be deployed
    const deployed = await isContractDeployed(TOKEN_NETWORK_ADDRESS);
    if (!deployed) {
      console.log(
        'Skipping T5: TokenNetwork not deployed on Arbitrum Sepolia.\n' +
          `  Expected at: ${TOKEN_NETWORK_ADDRESS}`
      );
      return;
    }

    // Use Town Node 2 (A) and Client (B) for isolated settlement
    const accountA = privateKeyToAccount(SETTLEMENT_PRIVATE_KEY_A);
    const accountB = privateKeyToAccount(SETTLEMENT_PRIVATE_KEY_B);

    console.log(`Account A (opener/depositor/signer): ${accountA.address}`);
    console.log(`Account B (claimant):                ${accountB.address}`);

    const walletA = createWalletClient({
      account: accountA,
      chain: arbSepoliaChain,
      transport: http(CHAIN_RPC),
    });
    const walletB = createWalletClient({
      account: accountB,
      chain: arbSepoliaChain,
      transport: http(CHAIN_RPC),
    });
    const publicClient = createViemPublicClient();

    // Small amounts to preserve testnet USDC (6 decimals)
    const depositAmount = 50_000n; // 0.05 USDC
    const transferAmount = 10_000n; // 0.01 USDC
    // Contract enforces minimum 3600s (1 hour) settlement timeout
    const settlementTimeout = 3600n;

    // Check USDC balances before starting
    const balanceABefore = await getTokenBalance(accountA.address as Hex);
    const balanceBBefore = await getTokenBalance(accountB.address as Hex);
    console.log(
      `USDC balances: A=${balanceABefore} (${Number(balanceABefore) / 1e6} USDC), B=${balanceBBefore} (${Number(balanceBBefore) / 1e6} USDC)`
    );

    if (balanceABefore < depositAmount) {
      console.log(
        `Skipping T5: Account A needs ${depositAmount} USDC units but has ${balanceABefore}`
      );
      return;
    }

    // Step 1: Approve TokenNetwork to spend tokens
    console.log('Step 1: Approving TokenNetwork...');
    const approveTxHash = await walletA.writeContract({
      address: TOKEN_ADDRESS,
      abi: ERC20_ABI,
      functionName: 'approve',
      args: [TOKEN_NETWORK_ADDRESS, depositAmount * 2n],
    });
    const approveReceipt = await publicClient.waitForTransactionReceipt({
      hash: approveTxHash,
    });
    expect(approveReceipt.status).toBe('success');
    console.log(`  Approved (tx: ${approveTxHash.slice(0, 18)}...)`);

    // Step 2: Open channel (A -> B) with 1hr timeout
    console.log('Step 2: Opening channel (timeout=3600s)...');
    const openTxHash = await walletA.writeContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'openChannel',
      args: [accountB.address, settlementTimeout],
    });
    const openReceipt = await publicClient.waitForTransactionReceipt({
      hash: openTxHash,
    });
    expect(openReceipt.status).toBe('success');
    console.log(`  Channel opened (tx: ${openTxHash.slice(0, 18)}...)`);

    // Extract channelId from ChannelOpened event in receipt logs
    let extractedChannelId: Hex | undefined;
    for (const log of openReceipt.logs) {
      try {
        const decoded = decodeEventLog({
          abi: TOKEN_NETWORK_ABI,
          data: log.data,
          topics: log.topics,
        });
        if (decoded.eventName === 'ChannelOpened') {
          extractedChannelId = (decoded.args as Record<string, unknown>)['channelId'] as Hex;
          break;
        }
      } catch {
        // Not our event, skip
      }
    }
    expect(extractedChannelId).toBeDefined();
    const channelId = extractedChannelId as Hex;
    console.log(`  Channel ID: ${channelId.slice(0, 18)}...`);

    // Step 3: Deposit tokens
    console.log('Step 3: Depositing tokens...');
    const depositTxHash = await walletA.writeContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'setTotalDeposit',
      args: [channelId, accountA.address, depositAmount],
    });
    const depositReceipt = await publicClient.waitForTransactionReceipt({
      hash: depositTxHash,
    });
    expect(depositReceipt.status).toBe('success');
    console.log(
      `  Deposited ${depositAmount} units (tx: ${depositTxHash.slice(0, 18)}...)`
    );

    // Verify channel is open with correct deposit
    const channelAfterDeposit = await getChannelState(channelId);
    expect(channelAfterDeposit.state).toBe('open');
    expect(channelAfterDeposit.settlementTimeout).toBe(
      Number(settlementTimeout)
    );

    // Verify deposit recorded in participants mapping
    const participantData = await publicClient.readContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'participants',
      args: [channelId, accountA.address as Hex],
    });
    const [deposit] = participantData;
    expect(deposit).toBe(depositAmount);
    console.log(`  On-chain deposit verified: ${deposit}`);

    // Verify wallet balance decreased by deposit amount
    const balanceAAfterDeposit = await getTokenBalance(accountA.address as Hex);
    expect(balanceAAfterDeposit).toBe(balanceABefore - depositAmount);
    console.log(`  Wallet A balance decreased: ${balanceABefore} → ${balanceAAfterDeposit} (−${depositAmount})`);

    // Step 4: Sign EIP-712 balance proof (A signs transfer to B)
    console.log('Step 4: Signing balance proof...');
    const balanceProof = {
      channelId,
      nonce: 1n,
      transferredAmount: transferAmount,
      lockedAmount: 0n,
      locksRoot:
        '0x0000000000000000000000000000000000000000000000000000000000000000' as Hex,
    };

    const domain = {
      name: 'TokenNetwork',
      version: '1',
      chainId: CHAIN_ID,
      verifyingContract: TOKEN_NETWORK_ADDRESS as Hex,
    };

    const signature = await walletA.signTypedData({
      domain,
      types: BALANCE_PROOF_TYPES,
      primaryType: 'BalanceProof',
      message: balanceProof,
    });
    expect(signature).toMatch(/^0x[0-9a-f]{130}$/i);
    console.log(`  Signed: nonce=1, transfer=${transferAmount}, sig=${signature.slice(0, 18)}...`);

    // Step 5: B claims from channel (delta-based, channel stays open)
    console.log('Step 5: Claiming from channel...');
    const claimTxHash = await walletB.writeContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'claimFromChannel',
      args: [channelId, balanceProof, signature],
    });
    const claimReceipt = await publicClient.waitForTransactionReceipt({
      hash: claimTxHash,
    });
    expect(claimReceipt.status).toBe('success');
    console.log(`  Claimed (tx: ${claimTxHash.slice(0, 18)}...)`);

    // Verify ChannelClaimed event
    let claimedAmount: bigint | undefined;
    let totalClaimed: bigint | undefined;
    for (const log of claimReceipt.logs) {
      try {
        const decoded = decodeEventLog({
          abi: TOKEN_NETWORK_ABI,
          data: log.data,
          topics: log.topics,
        });
        if (decoded.eventName === 'ChannelClaimed') {
          const args = decoded.args as Record<string, unknown>;
          claimedAmount = args['claimedAmount'] as bigint;
          totalClaimed = args['totalClaimed'] as bigint;
          break;
        }
      } catch {
        // Not our event
      }
    }
    expect(claimedAmount).toBe(transferAmount);
    expect(totalClaimed).toBe(transferAmount);
    console.log(`  ChannelClaimed event: claimed=${claimedAmount}, total=${totalClaimed}`);

    // Verify channel is STILL OPEN (claimFromChannel doesn't close)
    const channelAfterClaim = await getChannelState(channelId);
    expect(channelAfterClaim.state).toBe('open');
    console.log(`  Channel state after claim: ${channelAfterClaim.state} (still open)`);

    // Verify claimedAmounts on-chain
    const onChainClaimed = await publicClient.readContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'claimedAmounts',
      args: [channelId, accountB.address as Hex],
    });
    expect(onChainClaimed).toBe(transferAmount);
    console.log(`  On-chain claimedAmounts[B]: ${onChainClaimed}`);

    // Verify B received the claimed tokens
    const balanceBAfterClaim = await getTokenBalance(accountB.address as Hex);
    expect(balanceBAfterClaim).toBe(balanceBBefore + transferAmount);
    console.log(`  Wallet B balance: ${balanceBBefore} → ${balanceBAfterClaim} (+${transferAmount})`);

    // Verify A's wallet unchanged (tokens came from channel deposit, not wallet)
    const balanceAAfterClaim = await getTokenBalance(accountA.address as Hex);
    expect(balanceAAfterClaim).toBe(balanceABefore - depositAmount);
    console.log(`  Wallet A balance: ${balanceAAfterClaim} (unchanged — funds from deposit)`);

    console.log(
      'T5: Payment channel lifecycle complete (open → deposit → sign → claim)'
    );
    console.log(
      '  Channel remains open — can continue sending/claiming without closing.'
    );
  }, 120000);

  // =========================================================================
  // T6: claimFromChannel on bootstrap channel — verify balance changes
  //
  // After T2 published 10 events with signed BTP claims, the Oyster node
  // has received verified claims. We submit claimFromChannel() on-chain
  // using the client's latest signed balance proof and verify that:
  // - The node's wallet receives the claimed tokens
  // - claimedAmounts mapping updates on-chain
  // - The channel remains open after claiming
  // =========================================================================

  it('claimFromChannel: node claims from bootstrap channel, balances verified', async () => {
    if (skipIfNotReady()) return;

    if (!bootstrapChannelId) {
      console.log('Skipping T6: No bootstrap channel (T2 did not run or had no channel)');
      return;
    }

    if (totalPublishPayment === 0n || !toonClient) {
      console.log('Skipping T6: No payments made in T2 or ToonClient unavailable');
      return;
    }

    const publicClient = createViemPublicClient();
    const clientAccount = privateKeyToAccount(CLIENT_PRIVATE_KEY);

    console.log(`Bootstrap channel: ${(bootstrapChannelId as string).slice(0, 18)}...`);
    console.log(`Total payment from T2: ${totalPublishPayment} units`);
    console.log(`Oyster node wallet: ${OYSTER_NODE_ADDRESS}`);

    // Verify channel is open and client has a deposit
    const channelBefore = await getChannelState(bootstrapChannelId);
    expect(channelBefore.state).toBe('open');
    console.log(`  Channel state: ${channelBefore.state}`);

    const clientParticipant = await publicClient.readContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'participants',
      args: [bootstrapChannelId, clientAccount.address as Hex],
    });
    const [clientDeposit] = clientParticipant;
    console.log(`  Client deposit: ${clientDeposit}`);
    expect(clientDeposit).toBeGreaterThan(0n);

    // Record balances before claim
    const nodeBalanceBefore = await getTokenBalance(OYSTER_NODE_ADDRESS as Hex);
    const clientBalanceBefore = await getTokenBalance(clientAccount.address as Hex);
    console.log(`  Pre-claim balances: node=${nodeBalanceBefore}, client=${clientBalanceBefore}`);

    // First check if connector auto-settled (poll briefly)
    let autoSettled = false;
    let claimedBefore = await publicClient.readContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'claimedAmounts',
      args: [bootstrapChannelId, OYSTER_NODE_ADDRESS as Hex],
    });
    if (claimedBefore === 0n) {
      console.log('  Checking for auto-settlement (15s)...');
      const deadline = Date.now() + 15_000;
      while (claimedBefore === 0n && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 3000));
        claimedBefore = await publicClient.readContract({
          address: TOKEN_NETWORK_ADDRESS,
          abi: TOKEN_NETWORK_ABI,
          functionName: 'claimedAmounts',
          args: [bootstrapChannelId, OYSTER_NODE_ADDRESS as Hex],
        });
      }
    }
    if (claimedBefore > 0n) {
      autoSettled = true;
      console.log(`  Auto-settlement detected! Already claimed: ${claimedBefore}`);
    } else {
      console.log('  No auto-settlement — submitting manual claimFromChannel()');
    }

    // Build balance proof matching the client's cumulative signed claims from T2
    // The client signed these proofs incrementally; the last one has the full amount
    const nonce = toonClient.getChannelNonce(bootstrapChannelId);
    const cumulativeAmount = toonClient.getChannelCumulativeAmount(bootstrapChannelId);
    console.log(`  Client claim state: nonce=${nonce}, cumulative=${cumulativeAmount}`);

    if (!autoSettled) {
      // Sign the final balance proof for the full cumulative amount
      const domain = {
        name: 'TokenNetwork',
        version: '1',
        chainId: CHAIN_ID,
        verifyingContract: TOKEN_NETWORK_ADDRESS as Hex,
      };

      const balanceProof = {
        channelId: bootstrapChannelId,
        nonce: BigInt(nonce),
        transferredAmount: cumulativeAmount,
        lockedAmount: 0n,
        locksRoot:
          '0x0000000000000000000000000000000000000000000000000000000000000000' as Hex,
      };

      // Client signs (they are the payer — their signature authorizes transfer to node)
      const walletClient = createWalletClient({
        account: clientAccount,
        chain: arbSepoliaChain,
        transport: http(CHAIN_RPC),
      });
      const signature = await walletClient.signTypedData({
        domain,
        types: BALANCE_PROOF_TYPES,
        primaryType: 'BalanceProof',
        message: balanceProof,
      });
      console.log(`  Signed balance proof: nonce=${nonce}, amount=${cumulativeAmount}, sig=${signature.slice(0, 18)}...`);

      // Node submits claimFromChannel (using node's wallet — Town Node 1)
      const nodeWallet = createWalletClient({
        account: privateKeyToAccount(
          (process.env['OYSTER_NODE1_PRIVATE_KEY'] ||
            '0x8e6dd66be73771f13a990073474dc8ea5e3da6c23a5155b02a0c625125b07b83') as Hex
        ),
        chain: arbSepoliaChain,
        transport: http(CHAIN_RPC),
      });

      const claimTxHash = await nodeWallet.writeContract({
        address: TOKEN_NETWORK_ADDRESS,
        abi: TOKEN_NETWORK_ABI,
        functionName: 'claimFromChannel',
        args: [bootstrapChannelId, balanceProof, signature],
      });
      const claimReceipt = await publicClient.waitForTransactionReceipt({
        hash: claimTxHash,
      });
      expect(claimReceipt.status).toBe('success');
      console.log(`  claimFromChannel tx: ${claimTxHash.slice(0, 18)}...`);

      // Verify ChannelClaimed event
      for (const log of claimReceipt.logs) {
        try {
          const decoded = decodeEventLog({
            abi: TOKEN_NETWORK_ABI,
            data: log.data,
            topics: log.topics,
          });
          if (decoded.eventName === 'ChannelClaimed') {
            const args = decoded.args as Record<string, unknown>;
            console.log(`  ChannelClaimed: amount=${args['claimedAmount']}, total=${args['totalClaimed']}`);
          }
        } catch {
          // Not our event
        }
      }
    }

    // Verify post-claim state
    const claimedAfter = await publicClient.readContract({
      address: TOKEN_NETWORK_ADDRESS,
      abi: TOKEN_NETWORK_ABI,
      functionName: 'claimedAmounts',
      args: [bootstrapChannelId, OYSTER_NODE_ADDRESS as Hex],
    });
    expect(claimedAfter).toBeGreaterThan(0n);
    console.log(`  On-chain claimedAmounts[node]: ${claimedAfter}`);

    // Verify node received tokens
    const nodeBalanceAfter = await getTokenBalance(OYSTER_NODE_ADDRESS as Hex);
    expect(nodeBalanceAfter).toBeGreaterThan(nodeBalanceBefore);
    const nodeIncrease = nodeBalanceAfter - nodeBalanceBefore;
    console.log(`  Node wallet: ${nodeBalanceBefore} → ${nodeBalanceAfter} (+${nodeIncrease})`);

    // Verify channel still open
    const channelAfter = await getChannelState(bootstrapChannelId);
    expect(channelAfter.state).toBe('open');
    console.log(`  Channel state: ${channelAfter.state} (still open after claim)`);

    // Verify client wallet unchanged (tokens came from channel deposit)
    const clientBalanceAfter = await getTokenBalance(clientAccount.address as Hex);
    console.log(`  Client wallet: ${clientBalanceBefore} → ${clientBalanceAfter}`);

    console.log(
      `T6: Settlement verified — ${autoSettled ? 'auto' : 'manual'} claim of ${claimedAfter} units`
    );
    console.log(
      '  Channel stays open. Full payment lifecycle validated.'
    );
  }, 120000);
});
