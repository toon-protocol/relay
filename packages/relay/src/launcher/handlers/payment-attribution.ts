/**
 * The connector's payment statement, as read off a delivery to `POST /write`.
 *
 * A terminating connector states three headers on a delivery whose payment it
 * verified at its OWN client edge (`toon-protocol/connector` ADR 0040):
 *
 * | header          | value                                                       |
 * | --------------- | ----------------------------------------------------------- |
 * | `X-TOON-Payer`  | `evm:0x<64 hex>` or `solana:<base58>` -- the client CHANNEL  |
 * |                 | key whose covering claim that connector verified             |
 * | `X-TOON-Amount` | the route's flat price (ADR 0020), decimal, base units       |
 * | `X-TOON-Chain`  | that key's namespace -- `evm` or `solana`                    |
 *
 * ! ABSENCE IS NOT "UNPAID" ! The headers are present ONLY when this
 * connector was the hop that took the payment and the route's price is
 * non-zero. They are absent -- not empty -- on a peer-wire arrival, a
 * forwarded packet, and every `price = 0` route (which is why the free
 * ephemeral lane never sees them). A handler that read absence as "nobody
 * paid" would reject exactly the deliveries a longer path produced.
 *
 * The relay does not, and cannot, re-validate any of this: it holds no chain
 * state and speaks no ILP. The connector's statement IS the trust model. What
 * this module adds is that a malformed statement is treated as no statement
 * at all -- a garbled value from a stale or hostile caller becomes `undefined`
 * rather than something the relay records and echoes as fact.
 *
 * History worth not repeating: relay#122 removed the earlier reading of these
 * same header NAMES, correctly, because the TypeScript-era connector set
 * `X-TOON-Payer` to the PREVIOUS HOP -- which on any path longer than one hop
 * named the wrong party. ADR 0040's successor is a chain-verified channel key
 * that is never stated by a hop that did not take the payment, so the value
 * now means what its name says (relay#133).
 *
 * @module
 */

import type { Context } from 'hono';

/** A payment the terminating connector states it verified. */
export interface PaymentAttribution {
  /** The client channel key, namespaced: `evm:0x<64 hex>` / `solana:<base58>`. */
  payer: string;
  /** The route's flat price in base units, decimal. */
  amount: string;
  /** The payer key's namespace. */
  chain: 'evm' | 'solana';
}

/** `evm:` + exactly 64 lower-case hex characters, `0x`-prefixed. */
const EVM_PAYER = /^evm:0x[0-9a-f]{64}$/;
/** `solana:` + a base58 public key (no 0, O, I or l in the alphabet). */
const SOLANA_PAYER = /^solana:[1-9A-HJ-NP-Za-km-z]{32,44}$/;
/** Base units are a decimal integer -- no sign, no exponent, no separators. */
const AMOUNT = /^[0-9]+$/;

/**
 * Read the connector's payment statement off a request.
 *
 * Returns `undefined` unless ALL THREE headers are present, individually
 * well-formed, and mutually consistent (the payer's namespace must be the
 * chain it claims). Anything else -- one header, two headers, a payer from a
 * chain the `X-TOON-Chain` header disagrees with, a non-decimal amount -- is
 * discarded whole. Partial attribution is worse than none: it would record
 * half a fact as if it were the whole one.
 *
 * This never rejects the request. A caller that states nothing, or states
 * nonsense, still gets its write handled on the merits of the event itself;
 * the payment gate is upstream and has already run by the time anything
 * reaches this process.
 */
export function readPaymentAttribution(
  c: Context
): PaymentAttribution | undefined {
  const payer = c.req.header('X-TOON-Payer');
  const amount = c.req.header('X-TOON-Amount');
  const chain = c.req.header('X-TOON-Chain');

  if (!payer || !amount || !chain) return undefined;
  if (chain !== 'evm' && chain !== 'solana') return undefined;
  if (!AMOUNT.test(amount)) return undefined;

  const payerMatchesChain =
    chain === 'evm' ? EVM_PAYER.test(payer) : SOLANA_PAYER.test(payer);
  if (!payerMatchesChain) return undefined;

  return { payer, amount, chain };
}
