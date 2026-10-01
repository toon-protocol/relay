/**
 * Reading the write edge off a connector's own self-description.
 *
 * The fixture below is the LIVE devnet relay connector's `GET /ilp` response,
 * copied verbatim on 2026-09-23, its settlement terms (`batchSettlements`,
 * `voucherSigners`) re-copied on 2026-10-01 (keys shortened where the value's
 * length is not the point). Testing against a hand-written shape would have let this
 * module agree with itself while disagreeing with the thing it reads.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  createConnectorEdgeWatcher,
  edgeFromSelfDescription,
} from './connector-edge.js';

/** The devnet relay connector's answer, as served on 2026-10-01. */
const DEVNET_SELF_DESCRIPTION = {
  ilpAddresses: ['g.toon.relay', 'g.toon.relay.ephemeral'],
  httpEndpoint: 'https://proxy.relay.devnet.toonprotocol.dev/ilp',
  btpEndpoint: 'wss://proxy.relay.devnet.toonprotocol.dev/ilp/btp',
  peerCarriages: [],
  edgeIdentity: {
    keyId: 'connector-signer',
    publicKey: '0x04915d29908235be4b53f8f23cd7ac72c88c99be3bcca876dadf5c1a4494',
  },
  batchSettlements: [
    {
      network: 'eip155:84532',
      asset: '0x0c996d7c934c79a6255254875607fe69df25c0e1',
      payTo: '0x3f43d923a611bcb2d0bfb5d6ee2c3ac3efeaf308',
      receiverAuthorizer: '0x3f43d923a611bcb2d0bfb5d6ee2c3ac3efeaf308',
      withdrawDelay: 86400,
      name: 'USDC',
      version: '2',
      assetTransferMethod: 'eip3009',
      facilitator: 'https://onboard.devnet.toonprotocol.dev',
    },
    {
      network: 'solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1',
      asset: '34eSxY7qxQ4GzyhDJ8GpUcTz1WWzruGbJbR8q6TtxfQU',
      payTo: 'GzvGVjq3dnNM79MpWRvYCvVcAgPWzDdYisMwGxHF4u9F',
      feePayer: 'GzvGVjq3dnNM79MpWRvYCvVcAgPWzDdYisMwGxHF4u9F',
      withdrawDelay: 86400,
      tokenProgram: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
      minDeposit: '1000000',
      sponsorEndpoint: '/ilp/batch-settlement/solana/open',
    },
  ],
  voucherSigners: [
    {
      network: 'eip155:84532',
      signer: '0x3f43d923a611bcb2d0bfb5d6ee2c3ac3efeaf308',
    },
    {
      network: 'solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1',
      signer: 'GzvGVjq3dnNM79MpWRvYCvVcAgPWzDdYisMwGxHF4u9F',
    },
  ],
  routes: [
    { prefix: 'g.toon.relay', price: '1' },
    { prefix: 'g.toon.relay.ephemeral', price: '0' },
    { prefix: 'g.toon.relay.gas', price: '1001' },
    { prefix: 'g.toon.relay.store', price: '1001', pricePerKib: '10' },
  ],
  supportedVersions: [1],
  defaultVersion: 1,
};

describe('edgeFromSelfDescription', () => {
  it('reads the devnet relay connector, verbatim', () => {
    const { edge, error } = edgeFromSelfDescription(
      DEVNET_SELF_DESCRIPTION,
      'g.toon.relay'
    );

    expect(error).toBeUndefined();
    expect(edge).toEqual({
      ilp_address: 'g.toon.relay',
      // The URL the connector ADVERTISES, never the private address the relay
      // dialled to ask -- a client dials what the self-description says.
      connector_url: 'https://proxy.relay.devnet.toonprotocol.dev/ilp',
      connector_seal_key: DEVNET_SELF_DESCRIPTION.edgeIdentity.publicKey,
      price: 1,
      settlement: [
        {
          network: 'eip155:84532',
          asset: '0x0c996d7c934c79a6255254875607fe69df25c0e1',
        },
        {
          network: 'solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1',
          asset: '34eSxY7qxQ4GzyhDJ8GpUcTz1WWzruGbJbR8q6TtxfQU',
        },
      ],
    });
    // TOON_Network#111 reproduced: the devnet connector pins BTP on this
    // route and publishes no carriage at all, so the document says nothing
    // about one rather than guessing.
    expect(edge?.carriage).toBeUndefined();
  });

  it('yields an empty list, still publishing the edge, without batchSettlements', () => {
    const { batchSettlements: _dropped, ...bare } = DEVNET_SELF_DESCRIPTION;
    const { edge, error } = edgeFromSelfDescription(bare, 'g.toon.relay');

    expect(error).toBeUndefined();
    expect(edge?.settlement).toEqual([]);
  });

  it('no longer reads the retired settlements key', () => {
    const { batchSettlements: _dropped, ...bare } = DEVNET_SELF_DESCRIPTION;
    const { edge } = edgeFromSelfDescription(
      {
        ...bare,
        settlements: [{ chain: 'solana', tokenAddress: 'x', decimals: 6 }],
      },
      'g.toon.relay'
    );

    expect(edge?.settlement).toEqual([]);
  });

  it('refuses an address its connector does not terminate, and says which it does', () => {
    // The drift guard. A relay mispointed at somebody else's prefix must
    // advertise nothing, not send every client's money down that route.
    const { edge, error } = edgeFromSelfDescription(
      DEVNET_SELF_DESCRIPTION,
      'g.toon.relay.somebody-else'
    );

    expect(edge).toBeUndefined();
    expect(error).toContain('does not terminate');
    expect(error).toContain('g.toon.relay.store');
  });

  it('reads the free lane as free, which is not the same as unpriced', () => {
    const { edge } = edgeFromSelfDescription(
      DEVNET_SELF_DESCRIPTION,
      'g.toon.relay.ephemeral'
    );
    expect(edge?.price).toBe(0);
  });

  it('takes the carriage from the route once the connector publishes one', () => {
    // What TOON_Network#111 lands: `requiredTransport` per route. No change
    // is needed here when it does.
    const withPin = {
      ...DEVNET_SELF_DESCRIPTION,
      routes: [
        { prefix: 'g.toon.relay', price: '1', requiredTransport: 'btp' },
        { prefix: 'g.toon.relay.ephemeral', price: '0' },
      ],
    };
    expect(
      edgeFromSelfDescription(withPin, 'g.toon.relay').edge?.carriage
    ).toBe('btp');
    expect(
      edgeFromSelfDescription(withPin, 'g.toon.relay.ephemeral').edge?.carriage
    ).toBeUndefined();
  });

  it('falls back to the per-node carriage the connector already ships', () => {
    const nodeWide = { ...DEVNET_SELF_DESCRIPTION, requiredTransport: 'http' };
    expect(
      edgeFromSelfDescription(nodeWide, 'g.toon.relay').edge?.carriage
    ).toBe('http');
  });

  it('treats the permissive default as silence, never as a pin', () => {
    const both = { ...DEVNET_SELF_DESCRIPTION, requiredTransport: 'both' };
    expect(
      edgeFromSelfDescription(both, 'g.toon.relay').edge?.carriage
    ).toBeUndefined();
  });

  it('lets an operator fill silence but never contradict the connector', () => {
    // The stopgap for #111: used only where the connector states nothing.
    expect(
      edgeFromSelfDescription(DEVNET_SELF_DESCRIPTION, 'g.toon.relay', 'btp')
        .edge?.carriage
    ).toBe('btp');

    const pinned = {
      ...DEVNET_SELF_DESCRIPTION,
      routes: [
        { prefix: 'g.toon.relay', price: '1', requiredTransport: 'btp' },
      ],
    };
    expect(
      edgeFromSelfDescription(pinned, 'g.toon.relay', 'http').edge?.carriage
    ).toBe('btp');
  });

  it('refuses a connector that cannot say where it is or who it is', () => {
    const noEndpoint = { ...DEVNET_SELF_DESCRIPTION, httpEndpoint: undefined };
    expect(edgeFromSelfDescription(noEndpoint, 'g.toon.relay').error).toContain(
      'httpEndpoint'
    );

    const noKey = { ...DEVNET_SELF_DESCRIPTION, edgeIdentity: {} };
    expect(edgeFromSelfDescription(noKey, 'g.toon.relay').error).toContain(
      'edgeIdentity.publicKey'
    );

    expect(
      edgeFromSelfDescription('not json at all', 'g.toon.relay').error
    ).toContain('not a JSON object');
  });

  it('refuses a price it cannot state as a whole number of uusdc', () => {
    const odd = {
      ...DEVNET_SELF_DESCRIPTION,
      routes: [{ prefix: 'g.toon.relay', price: 'free' }],
    };
    expect(edgeFromSelfDescription(odd, 'g.toon.relay').error).toContain(
      'not a whole number'
    );
  });
});

describe('createConnectorEdgeWatcher', () => {
  const ok = (body: unknown): Response =>
    ({ ok: true, status: 200, json: async () => body }) as Response;

  it('publishes nothing until its connector answers, then publishes what it said', async () => {
    let answer: () => Response = () => {
      throw new Error('ECONNREFUSED');
    };
    const fetchImpl = vi.fn(async () => answer());
    const log = vi.fn();

    const watcher = createConnectorEdgeWatcher({
      connectorUrl: 'http://connector:3000/ilp',
      ilpAddress: 'g.toon.relay',
      fetchImpl: fetchImpl as unknown as typeof fetch,
      log,
    });

    // The canonical compose bundle starts the connector only once the relay
    // is healthy, so the first read always fails. It must not throw, and it
    // must not block.
    await watcher.refresh();
    expect(watcher.current()).toBeNull();

    answer = () => ok(DEVNET_SELF_DESCRIPTION);
    await watcher.refresh();
    expect(watcher.current()?.ilp_address).toBe('g.toon.relay');
    expect(watcher.current()?.price).toBe(1);

    watcher.stop();
  });

  it('forgets an edge its connector has stopped vouching for', async () => {
    let body: unknown = DEVNET_SELF_DESCRIPTION;
    const watcher = createConnectorEdgeWatcher({
      connectorUrl: 'http://connector:3000/ilp',
      ilpAddress: 'g.toon.relay',
      fetchImpl: (async () => ok(body)) as unknown as typeof fetch,
      log: vi.fn(),
    });

    await watcher.refresh();
    expect(watcher.current()).not.toBeNull();

    // A connector reconfigured to terminate the prefix somewhere else. The
    // relay stops advertising rather than keeping the last good answer.
    body = { ...DEVNET_SELF_DESCRIPTION, routes: [] };
    await watcher.refresh();
    expect(watcher.current()).toBeNull();

    watcher.stop();
  });

  it('logs a state change, not every poll', async () => {
    const log = vi.fn();
    const watcher = createConnectorEdgeWatcher({
      connectorUrl: 'http://connector:3000/ilp',
      ilpAddress: 'g.toon.relay',
      fetchImpl: (async () =>
        ok(DEVNET_SELF_DESCRIPTION)) as unknown as typeof fetch,
      log,
    });

    await watcher.refresh();
    await watcher.refresh();
    await watcher.refresh();

    // A five-second retry that logged every attempt would be a line every
    // five seconds for as long as a connector stayed down.
    expect(log.mock.calls.length).toBe(1);
    watcher.stop();
  });
});
