/**
 * NIP-11: the relay information document, and the one refusal built from it.
 *
 * A TOON relay refuses every write that arrives on its WebSocket, because a
 * write to it is a PAID packet that reaches `POST /write` through the
 * terminating connector (ADR 0007). Before this module the refusal said only
 * `restricted: writes require ILP payment` and the relay served no NIP-11
 * document at all, so every writer had to be configured out of band: the
 * provider's publisher by `RELAY_WRITE_ROUTES`, the console by its own
 * connector's `GET /ilp`. Neither could pay a relay it merely knew the URL of
 * (TOON_Network#121).
 *
 * So a relay now pins the same three facts about itself that a Provider
 * Profile pins about a provider — `ilp_address`, `connector_url` and
 * `connector_seal_key` (TOON_Network §4.1, ADR 0011) — plus the carriage its
 * route pins (TOON_Network#111). A client holding only the relay's URL reads
 * them and buys a write.
 *
 * ── Where each field comes from, and why it cannot drift ────────────────────
 * Nothing here is written down twice.
 *
 *   - The NIP-11 `limitation` numbers are read off the SAME
 *     `RelayServerConfig` object `ConnectionHandler` enforces them from, so a
 *     changed cap changes the advertisement in the same breath.
 *   - The `toon` object is the relay's connector's OWN self-description
 *     (`GET /ilp`), narrowed to the one route that terminates at this relay —
 *     see `launcher/connector-edge.ts`. The relay restates a price, a key and
 *     a carriage it did not decide and cannot edit; when its connector says
 *     nothing, this document says nothing.
 *   - `writeRefusalMessage` is built from that same edge value, so the
 *     refusal and the document name one address or neither does.
 *
 * ── Why the TOON facts are one namespaced object ────────────────────────────
 * NIP-11 is an open JSON document and unknown keys are ignored, so four new
 * top-level keys would have worked. They are grouped under `toon` anyway:
 *
 *   1. NIP-11 has grown top-level keys over time (`limitation`, `fees`,
 *      `retention`, `payments_url`). A bare `carriage` or `settlement` beside
 *      them is a name a later NIP could take for something else; `toon` is a
 *      name only this network can take.
 *   2. Its presence is the ONE test a client makes — "can I pay this relay the
 *      TOON way" — rather than four presence checks that can half-succeed.
 *   3. Its field names are byte-identical to the Provider Profile's (§4.1), so
 *      one parser reads a relay's edge and a provider's.
 *
 * Everything NIP-11 already has words for stays in NIP-11's own words:
 * `limitation.payment_required`, `fees.publication`, `supported_nips`,
 * `software` and `version` are honest, so a Nostr client that has never heard
 * of TOON still learns that this relay is paid and how much it costs.
 *
 * @module
 */

import { VERSION } from '../version.js';

/** The media type NIP-11 gives the relay information document. */
export const NOSTR_JSON_CONTENT_TYPE = 'application/nostr+json';

/**
 * The repository the running code came from, reported as NIP-11 `software`.
 * A literal rather than a package.json read: `version` is already injected at
 * build time, and a second build-time substitution buys nothing.
 */
export const RELAY_SOFTWARE_URL = 'https://github.com/toon-protocol/relay';

/**
 * The NIPs this relay implements, reported verbatim as `supported_nips`.
 *
 * NIP-01 (the WebSocket protocol), NIP-09 (author-signed deletion), NIP-11
 * (this document), NIP-16 (the ephemeral-kind range the free lane carries)
 * and NIP-40 (expiration, when `enforceExpiration` is on — which is the
 * default and the only configuration this claim is made under; see
 * `buildRelayInformationDocument`).
 */
const BASE_SUPPORTED_NIPS = [1, 9, 11, 16] as const;

/** A carriage a connector can pin on a route: which transport a write rides. */
export type Carriage = 'http' | 'btp';

/**
 * Where a write to this relay is paid for: the facts a party needs in order
 * to buy one, and nothing else.
 *
 * The field names are the Provider Profile's (TOON_Network §4.1) on purpose.
 * Every value is read from the terminating connector's own self-description;
 * none of them is a relay setting.
 */
export interface RelayWriteEdge {
  /** The ILP address a write to this relay is addressed to, e.g. `g.toon.relay`. */
  ilp_address: string;
  /**
   * The connector's self-description URL, as the connector itself advertises
   * it — not the address the relay happens to reach it at on a private
   * network. A location hint only: the sealing key below is the pin.
   */
  connector_url: string;
  /**
   * The connector's sealing public key. A client seals its packet to this key
   * and refuses if the self-description at `connector_url` reports another
   * one (ADR 0011).
   */
  connector_seal_key: string;
  /**
   * The carriage this route pins, when the connector states one
   * (TOON_Network#111). Absent means the connector states no pin: a client
   * dials whichever endpoint it prefers and honours a `TRANSPORT_REQUIRED`
   * refusal as before. Never `"both"` — a carriage that is not a pin is
   * silence.
   */
  carriage?: Carriage;
  /**
   * What one write costs, in µUSDC (TOON_Network §2). `0` is a real value and
   * means this relay charges nothing, which is not the same as not saying.
   */
  price: number;
  /** The chains and tokens the connector settles claims in. */
  settlement: RelaySettlement[];
}

/** One chain a relay's connector settles in. */
export interface RelaySettlement {
  /** `solana` or `evm:<chainId>`. */
  chain: string;
  /** The token's mint or contract address. */
  token: string;
  /** The token's own decimals. */
  decimals: number;
}

/** The operator's free-text description of the relay (all optional). */
export interface RelayDescription {
  /** A short name for the relay. */
  name?: string;
  /** A sentence about what it carries. */
  description?: string;
  /** An operator contact: a mailto:, an npub, or a URL. */
  contact?: string;
}

/** NIP-11's `limitation` object, as this relay fills it. */
export interface RelayLimitation {
  /** True when a write to this relay costs something. */
  payment_required: boolean;
  /**
   * Always true on a TOON relay: no write is ever accepted on the WebSocket,
   * whatever it costs. A free TOON relay is still a restricted one.
   */
  restricted_writes: true;
  /** Subscriptions one connection may hold. */
  max_subscriptions: number;
  /** Filters one subscription may carry. */
  max_filters: number;
  /** This relay never asks for NIP-42 AUTH. */
  auth_required: false;
}

/** The NIP-11 relay information document this relay serves. */
export interface RelayInformationDocument {
  name?: string;
  description?: string;
  /** The relay's own Nostr public key (64-char hex). */
  pubkey: string;
  contact?: string;
  supported_nips: number[];
  software: string;
  version: string;
  limitation: RelayLimitation;
  /** Present only when a write costs something. */
  fees?: { publication: { amount: number; unit: string }[] };
  /** Where a write is paid for. Absent when the relay publishes no edge. */
  toon?: RelayWriteEdge;
}

/** Everything `buildRelayInformationDocument` needs. */
export interface RelayInformationInput {
  /** The relay's Nostr public key. */
  pubkey: string;
  /**
   * The very limits the connection handler enforces. This is the enforcing
   * object itself, not a copy of its numbers.
   */
  limits: {
    maxSubscriptionsPerConnection: number;
    maxFiltersPerSubscription: number;
    enforceExpiration: boolean;
  };
  /** The paid write edge, or `null` when the relay publishes none. */
  edge: RelayWriteEdge | null;
  /** The operator's free text. */
  description?: RelayDescription;
}

/**
 * Whether a request asked for the relay information document.
 *
 * NIP-11 says a client asks by sending `Accept: application/nostr+json`, and
 * that is the only thing this relay answers with the document: every other
 * plain HTTP request to the read port keeps answering `426 Upgrade Required`
 * exactly as it did before, so nothing that works today changes.
 *
 * @param accept - The request's `Accept` header, if any.
 * @returns True when the header names the NIP-11 media type.
 */
export function acceptsRelayInformation(accept: string | undefined): boolean {
  if (!accept) return false;
  return accept
    .split(',')
    .some(
      (part) =>
        part.split(';')[0]?.trim().toLowerCase() === NOSTR_JSON_CONTENT_TYPE
    );
}

/**
 * Build the NIP-11 document.
 *
 * Pure: every value comes from the input, so the document a test builds is
 * the document the wire carries.
 *
 * @param input - The relay's identity, its live limits, and its write edge.
 * @returns The document, ready to serialize.
 */
export function buildRelayInformationDocument(
  input: RelayInformationInput
): RelayInformationDocument {
  const { pubkey, limits, edge, description } = input;

  // NIP-40 is claimed only while it is enforced. A relay serving events past
  // their own expiration tag that advertised NIP-40 would be telling a client
  // its `expiration` was honoured when it was not -- and the kill switch
  // exists precisely so an operator can stop honouring it (docs/retention.md).
  const supported_nips = [
    ...BASE_SUPPORTED_NIPS,
    ...(limits.enforceExpiration ? [40] : []),
  ];

  const paid = edge !== null && edge.price > 0;

  return {
    ...(description?.name !== undefined && { name: description.name }),
    ...(description?.description !== undefined && {
      description: description.description,
    }),
    pubkey,
    ...(description?.contact !== undefined && { contact: description.contact }),
    supported_nips,
    software: RELAY_SOFTWARE_URL,
    version: VERSION,
    limitation: {
      payment_required: paid,
      restricted_writes: true,
      max_subscriptions: limits.maxSubscriptionsPerConnection,
      max_filters: limits.maxFiltersPerSubscription,
      auth_required: false,
    },
    // NIP-11's own words for the price, in the unit TOON prices everything in
    // (§2: integers in µUSDC). Omitted rather than reported as 0 on a relay
    // that charges nothing -- `payment_required: false` already says that, and
    // a `fees` object is what a client reads to find out what it will be
    // billed.
    ...(paid && {
      fees: { publication: [{ amount: edge.price, unit: 'uusdc' }] },
    }),
    ...(edge !== null && { toon: edge }),
  };
}

/**
 * The NIP-01 `OK` message a WebSocket write is refused with.
 *
 * Built from the same edge the document carries, so a client can recover from
 * the refusal alone: it names the address, the connector and the price, and
 * points at the document for the sealing key (which is too long to put in an
 * `OK` message a client may well be logging one line at a time).
 *
 * The `restricted:` prefix is NIP-01's machine-readable one and does not
 * change, nor do the words that follow it on a paid relay -- anything already
 * matching on `restricted: writes require ILP payment` keeps matching.
 *
 * @param edge - The relay's paid write edge, or null when it publishes none.
 * @returns The message for `["OK", <id>, false, <message>]`.
 */
export function writeRefusalMessage(edge: RelayWriteEdge | null): string {
  const document =
    `this relay's NIP-11 document (GET its URL with ` +
    `Accept: ${NOSTR_JSON_CONTENT_TYPE})`;

  if (edge === null) {
    return (
      'restricted: writes require ILP payment, and this relay does not ' +
      `publish where — ask its operator, then see ${document}`
    );
  }

  const carriage = edge.carriage === undefined ? '' : ` over ${edge.carriage}`;

  // A free TOON relay still refuses the WebSocket write: the lane is the
  // restriction, not the price. Saying "requires payment" there would be a
  // lie, and a client that believed it would go looking for a channel it does
  // not need.
  const lead =
    edge.price > 0
      ? `restricted: writes require ILP payment — send this event to ` +
        `${edge.ilp_address} through ${edge.connector_url}${carriage}, ` +
        `${edge.price} uusdc per write`
      : `restricted: writes arrive as TOON packets and this one is free — ` +
        `send this event to ${edge.ilp_address} through ` +
        `${edge.connector_url}${carriage}`;

  return `${lead}; the sealing key is in ${document}`;
}
