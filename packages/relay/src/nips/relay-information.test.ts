/**
 * What the NIP-11 document says, and what it refuses to say.
 *
 * The assertions worth having here are the ones about HONESTY: that the
 * document's limits are the enforced ones, that it claims payment only where
 * payment is charged, that it says nothing at all where its connector said
 * nothing, and that the refusal names the same place the document does.
 */

import { describe, it, expect } from 'vitest';
import {
  acceptsRelayInformation,
  buildRelayInformationDocument,
  NOSTR_JSON_CONTENT_TYPE,
  writeRefusalMessage,
} from './relay-information.js';
import type { RelayWriteEdge } from './relay-information.js';
import { VERSION } from '../version.js';

const LIMITS = {
  maxSubscriptionsPerConnection: 20,
  maxFiltersPerSubscription: 10,
  enforceExpiration: true,
};

const PUBKEY = 'a'.repeat(64);

const PAID_EDGE: RelayWriteEdge = {
  ilp_address: 'g.toon.relay',
  connector_url: 'https://proxy.relay.devnet.toonprotocol.dev/ilp',
  connector_seal_key: '0x04915d29908235be4b53f8f23cd7ac72c88c99be3b',
  carriage: 'btp',
  price: 1,
  settlement: [
    { chain: 'evm:84532', token: '0x49bee1', decimals: 6 },
    { chain: 'solana', token: '34eSxY7', decimals: 6 },
  ],
};

describe('acceptsRelayInformation', () => {
  it('answers to the media type NIP-11 names, however the header is written', () => {
    expect(acceptsRelayInformation(NOSTR_JSON_CONTENT_TYPE)).toBe(true);
    expect(
      acceptsRelayInformation('application/nostr+json; charset=utf-8')
    ).toBe(true);
    expect(acceptsRelayInformation('APPLICATION/NOSTR+JSON')).toBe(true);
    expect(
      acceptsRelayInformation('text/html, application/nostr+json, */*')
    ).toBe(true);
  });

  it('leaves every other request to the 426 it has always had', () => {
    // A browser's default Accept is the one that matters: `*/*` must NOT get
    // the document, or a bare GET of a relay URL stops being an upgrade
    // request and every plain client's behaviour changes.
    expect(acceptsRelayInformation('*/*')).toBe(false);
    expect(acceptsRelayInformation('application/json')).toBe(false);
    expect(acceptsRelayInformation('text/html')).toBe(false);
    expect(acceptsRelayInformation(undefined)).toBe(false);
    expect(acceptsRelayInformation('')).toBe(false);
  });
});

describe('buildRelayInformationDocument', () => {
  it('reports the limits the relay is enforcing, not a second copy of them', () => {
    const document = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: { ...LIMITS, maxSubscriptionsPerConnection: 3 },
      edge: null,
    });

    expect(document.limitation.max_subscriptions).toBe(3);
    expect(document.limitation.max_filters).toBe(10);
    expect(document.pubkey).toBe(PUBKEY);
    expect(document.version).toBe(VERSION);
  });

  it('claims NIP-40 only while it is enforced', () => {
    expect(
      buildRelayInformationDocument({
        pubkey: PUBKEY,
        limits: LIMITS,
        edge: null,
      }).supported_nips
    ).toContain(40);
    expect(
      buildRelayInformationDocument({
        pubkey: PUBKEY,
        limits: { ...LIMITS, enforceExpiration: false },
        edge: null,
      }).supported_nips
    ).not.toContain(40);
  });

  it('names the write edge in TOON words and the price in NIP-11 words', () => {
    const document = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: LIMITS,
      edge: PAID_EDGE,
    });

    // The three fields a Provider Profile pins (TOON_Network §4.1, ADR 0011),
    // spelled the same way so one parser reads a relay's edge and a
    // provider's, plus the carriage the route pins (TOON_Network#111).
    expect(document.toon).toEqual(PAID_EDGE);
    expect(document.toon?.ilp_address).toBe('g.toon.relay');
    expect(document.toon?.connector_seal_key).toBe(
      PAID_EDGE.connector_seal_key
    );
    expect(document.toon?.carriage).toBe('btp');

    // And everything NIP-11 already has words for, in NIP-11's own words, so
    // a client that has never heard of TOON still learns it must pay.
    expect(document.limitation.payment_required).toBe(true);
    expect(document.fees).toEqual({
      publication: [{ amount: 1, unit: 'uusdc' }],
    });
  });

  it('still works, and says so, on a relay that charges nothing', () => {
    const document = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: LIMITS,
      edge: { ...PAID_EDGE, price: 0 },
    });

    expect(document.limitation.payment_required).toBe(false);
    // Absent, not `[{amount: 0}]`: `payment_required: false` is the statement,
    // and a `fees` object is what a client reads to learn what it will be
    // billed.
    expect(document.fees).toBeUndefined();
    // The edge is still named. A free write is still a packet to an address.
    expect(document.toon?.ilp_address).toBe('g.toon.relay');
    expect(document.toon?.price).toBe(0);
  });

  it('says nothing where its connector said nothing', () => {
    const document = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: LIMITS,
      edge: null,
    });

    expect(document.toon).toBeUndefined();
    expect(document.fees).toBeUndefined();
    expect(document.limitation.payment_required).toBe(false);
    // But a TOON relay never takes a write on the WebSocket, whatever it
    // costs and whether or not it can say where to pay instead.
    expect(document.limitation.restricted_writes).toBe(true);
  });

  it('omits an operator field rather than serving it empty', () => {
    const named = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: LIMITS,
      edge: null,
      description: { name: 'devnet relay' },
    });
    expect(named.name).toBe('devnet relay');
    expect('description' in named).toBe(false);
    expect('contact' in named).toBe(false);
  });

  it('serializes to something a NIP-11 reader can parse', () => {
    const document = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: LIMITS,
      edge: PAID_EDGE,
    });
    expect(JSON.parse(JSON.stringify(document))).toEqual(document);
  });
});

describe('writeRefusalMessage', () => {
  it('keeps the NIP-01 prefix and the words clients already match on', () => {
    for (const edge of [null, PAID_EDGE]) {
      expect(
        writeRefusalMessage(edge).startsWith(
          'restricted: writes require ILP payment'
        )
      ).toBe(true);
    }
  });

  it('names the same place the document does', () => {
    const message = writeRefusalMessage(PAID_EDGE);
    const document = buildRelayInformationDocument({
      pubkey: PUBKEY,
      limits: LIMITS,
      edge: PAID_EDGE,
    });

    // Both are rendered from the one edge value, which is what stops a relay
    // refusing a write towards one address while advertising another.
    expect(message).toContain(document.toon?.ilp_address ?? '');
    expect(message).toContain(document.toon?.connector_url ?? '');
    expect(message).toContain('btp');
    expect(message).toContain('1 uusdc');
    expect(message).toContain(NOSTR_JSON_CONTENT_TYPE);
  });

  it('does not tell a free relay’s client to go and find a channel', () => {
    const message = writeRefusalMessage({ ...PAID_EDGE, price: 0 });
    expect(message.startsWith('restricted:')).toBe(true);
    expect(message).not.toContain('require ILP payment');
    expect(message).toContain('free');
    expect(message).toContain('g.toon.relay');
  });

  it('admits it when the relay does not know where its writes are paid for', () => {
    const message = writeRefusalMessage(null);
    expect(message).toContain('does not');
    expect(message).toContain(NOSTR_JSON_CONTENT_TYPE);
  });
});
