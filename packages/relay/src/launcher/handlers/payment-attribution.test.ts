/**
 * Unit tests for the ADR 0040 payment-statement reader (relay#133).
 *
 * The reader's whole job is to be strict in one direction only: a well-formed
 * triple is passed through verbatim (the relay re-validates nothing), and
 * anything else becomes `undefined` rather than a half-recorded fact.
 */

import { describe, it, expect } from 'vitest';
import { Hono } from 'hono';
import { readPaymentAttribution } from './payment-attribution.js';
import type { PaymentAttribution } from './payment-attribution.js';

const EVM_KEY = `evm:0x${'a'.repeat(64)}`;
const SOLANA_KEY = 'solana:9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';

/** Run the reader against a request carrying `headers`. */
async function read(
  headers: Record<string, string>
): Promise<PaymentAttribution | undefined> {
  let seen: PaymentAttribution | undefined;
  const app = new Hono();
  app.post('/write', (c) => {
    seen = readPaymentAttribution(c);
    return c.body(null, 204);
  });

  await app.fetch(
    new Request('http://localhost/write', { method: 'POST', headers })
  );

  return seen;
}

describe('readPaymentAttribution', () => {
  it('reads a complete EVM statement verbatim', async () => {
    expect(
      await read({
        'X-TOON-Payer': EVM_KEY,
        'X-TOON-Amount': '1',
        'X-TOON-Chain': 'evm',
      })
    ).toEqual({ payer: EVM_KEY, amount: '1', chain: 'evm' });
  });

  it('reads a complete Solana statement verbatim', async () => {
    expect(
      await read({
        'X-TOON-Payer': SOLANA_KEY,
        'X-TOON-Amount': '1000',
        'X-TOON-Chain': 'solana',
      })
    ).toEqual({ payer: SOLANA_KEY, amount: '1000', chain: 'solana' });
  });

  it('is header-name case insensitive, as HTTP requires', async () => {
    expect(
      await read({
        'x-toon-payer': EVM_KEY,
        'x-toon-amount': '7',
        'x-toon-chain': 'evm',
      })
    ).toEqual({ payer: EVM_KEY, amount: '7', chain: 'evm' });
  });

  it('reads nothing when the connector stated nothing', async () => {
    expect(await read({})).toBeUndefined();
  });

  it('accepts a zero amount -- a stated free write is still a statement', async () => {
    expect(
      await read({
        'X-TOON-Payer': EVM_KEY,
        'X-TOON-Amount': '0',
        'X-TOON-Chain': 'evm',
      })
    ).toEqual({ payer: EVM_KEY, amount: '0', chain: 'evm' });
  });

  describe('discards a statement it cannot trust whole', () => {
    const cases: [string, Record<string, string>][] = [
      ['payer alone', { 'X-TOON-Payer': EVM_KEY }],
      ['amount alone', { 'X-TOON-Amount': '1' }],
      ['chain alone', { 'X-TOON-Chain': 'evm' }],
      [
        'payer + amount, no chain',
        { 'X-TOON-Payer': EVM_KEY, 'X-TOON-Amount': '1' },
      ],
      [
        'an EVM payer declared solana',
        {
          'X-TOON-Payer': EVM_KEY,
          'X-TOON-Amount': '1',
          'X-TOON-Chain': 'solana',
        },
      ],
      [
        'a Solana payer declared evm',
        {
          'X-TOON-Payer': SOLANA_KEY,
          'X-TOON-Amount': '1',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'an unknown chain namespace',
        {
          'X-TOON-Payer': EVM_KEY,
          'X-TOON-Amount': '1',
          'X-TOON-Chain': 'mina',
        },
      ],
      [
        'a numeric chain id (the TypeScript-era shape, relay#122)',
        {
          'X-TOON-Payer': '0xpayer',
          'X-TOON-Amount': '5500',
          'X-TOON-Chain': '31337',
        },
      ],
      [
        'an un-namespaced payer',
        {
          'X-TOON-Payer': `0x${'a'.repeat(64)}`,
          'X-TOON-Amount': '1',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'an upper-case hex payer',
        {
          'X-TOON-Payer': `evm:0x${'A'.repeat(64)}`,
          'X-TOON-Amount': '1',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'a short EVM key',
        {
          'X-TOON-Payer': `evm:0x${'a'.repeat(40)}`,
          'X-TOON-Amount': '1',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'a fractional amount',
        {
          'X-TOON-Payer': EVM_KEY,
          'X-TOON-Amount': '1.5',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'a negative amount',
        {
          'X-TOON-Payer': EVM_KEY,
          'X-TOON-Amount': '-1',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'a non-numeric amount',
        {
          'X-TOON-Payer': EVM_KEY,
          'X-TOON-Amount': 'free',
          'X-TOON-Chain': 'evm',
        },
      ],
      [
        'an empty payer',
        { 'X-TOON-Payer': '', 'X-TOON-Amount': '1', 'X-TOON-Chain': 'evm' },
      ],
    ];

    it.each(cases)('%s', async (_name, headers) => {
      expect(await read(headers)).toBeUndefined();
    });
  });
});
