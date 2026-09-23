/**
 * Reading the write edge off a connector's own self-description.
 *
 * The fixture below is the LIVE devnet relay connector's `GET /ilp` response,
 * copied verbatim on 2026-09-23 (keys shortened where the value's length is
 * not the point). Testing against a hand-written shape would have let this
 * module agree with itself while disagreeing with the thing it reads.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  createConnectorEdgeWatcher,
  edgeFromSelfDescription,
} from './connector-edge.js';

/** The devnet relay connector's answer, as served on 2026-09-23. */
const DEVNET_SELF_DESCRIPTION = {
  ilpAddresses: ['g.toon.relay', 'g.toon.relay.ephemeral'],
  httpEndpoint: 'https://proxy.relay.devnet.toonprotocol.dev/ilp',
  btpEndpoint: 'wss://proxy.relay.devnet.toonprotocol.dev/ilp/btp',
  peerCarriages: [],
  edgeIdentity: {
    keyId: 'connector-signer',
    publicKey: '0x04915d29908235be4b53f8f23cd7ac72c88c99be3bcca876dadf5c1a4494',
  },
  settlements: [
    {
      chain: 'evm:84532',
      settlementAddress: '0x3f43d923a611bcb2d0bfb5d6ee2c3ac3efeaf308',
      tokenNetworkRegistry: '0x0c41d9d424d6b075a3cea1068a694f7847a8cca5',
      tokenNetwork: '0xe9e05dfecfe165266c88d73e61d483612651952a',
      tokenAddress: '0x49bee1bca5d15fb0963117923403f9498119a9ce',
      decimals: 6,
    },
    {
      chain: 'solana',
      settlementAddress: 'GzvGVjq3dnNM79MpWRvYCvVcAgPWzDdYisMwGxHF4u9F',
      programId: '2aEVJ8koKD8LTZrLRSGtAtU7LBt4e7QjjCgf1kzQ7Rip',
      tokenAddress: '34eSxY7qxQ4GzyhDJ8GpUcTz1WWzruGbJbR8q6TtxfQU',
      decimals: 6,
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
          chain: 'evm:84532',
          token: '0x49bee1bca5d15fb0963117923403f9498119a9ce',
          decimals: 6,
        },
        {
          chain: 'solana',
          token: '34eSxY7qxQ4GzyhDJ8GpUcTz1WWzruGbJbR8q6TtxfQU',
          decimals: 6,
        },
      ],
    });
    // TOON_Network#111 reproduced: the devnet connector pins BTP on this
    // route and publishes no carriage at all, so the document says nothing
    // about one rather than guessing.
    expect(edge?.carriage).toBeUndefined();
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
