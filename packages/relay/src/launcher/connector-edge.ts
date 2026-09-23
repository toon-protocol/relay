/**
 * Where the relay learns where its own writes are paid for.
 *
 * The relay speaks no ILP and holds no price (CLAUDE.md): payment is enforced
 * entirely by the connector in front of it. So the relay must not WRITE DOWN
 * an ILP address, a sealing key, a price or a carriage — a second copy of the
 * enforcer's facts is a copy that drifts, and the failure mode is a relay
 * advertising a price nobody charges or a key nobody holds.
 *
 * It asks instead. A connector serves its own facts, free and
 * unauthenticated, on `GET /ilp` (connector ADR 0050) — the same document the
 * provider's publisher calls "the whole of bootstrapping". This module fetches
 * that document, narrows it to the ONE route that terminates at this relay,
 * and hands back a `RelayWriteEdge`. The NIP-11 document and the WebSocket
 * refusal are both rendered from that value and from nothing else
 * (TOON_Network#121).
 *
 * ── The one thing the relay must be told, and why ───────────────────────────
 * A connector's self-description publishes its routes as `{prefix, price}` and
 * deliberately NOT their `handler_url` (connector rule ND-08: a handler is the
 * app's fact, not the network's). The devnet relay's connector terminates four
 * prefixes — `g.toon.relay`, `g.toon.relay.ephemeral`, `g.toon.relay.gas`,
 * `g.toon.relay.store` — and from the document alone there is no way to tell
 * which of them arrives at THIS relay's `POST /write`. So the relay is told
 * one thing, `TOON_WRITE_ILP_ADDRESS`, and everything else is read.
 *
 * That pin cannot drift either, in two independent ways:
 *
 *   - At runtime, this module REFUSES to build an edge for an address its
 *     connector does not terminate, and says which prefixes it does. A relay
 *     mispointed at `g.toon.relay.store` advertises nothing and logs why,
 *     rather than sending clients to somebody else's route.
 *   - At build time, `deploy/bundle.test.ts` holds the env the canonical
 *     bundle sets equal to the prefix whose `handler_url` is this relay's
 *     `/write`, in the very connector.toml the box mounts.
 *
 * ── Carriage ────────────────────────────────────────────────────────────────
 * `requiredTransport` is the connector's word for the carriage a route pins.
 * The per-node scalar ships today; the per-route key is TOON_Network#111,
 * which the devnet relay needs because its two addresses disagree (one pins
 * BTP, the free ephemeral lane pins nothing) and a node-wide scalar is
 * therefore unemittable. This module reads the per-route key first and the
 * per-node scalar second, so #111 lands with no change here.
 *
 * Until it lands, an operator may state the pin (`TOON_WRITE_CARRIAGE`). That
 * statement fills SILENCE only: a connector that names a carriage always
 * wins, so the stopgap can never contradict the thing doing the enforcing.
 *
 * @module
 */

import type {
  Carriage,
  RelaySettlement,
  RelayWriteEdge,
} from '../nips/relay-information.js';

/** How often a known edge is re-read, in ms. */
export const DEFAULT_EDGE_REFRESH_MS = 300_000;

/**
 * How often an UNKNOWN edge is retried, in ms.
 *
 * Much shorter than the refresh above, because the ordinary case for "not yet
 * known" is the first seconds of a boot: the canonical compose bundle starts
 * the connector only once the relay is healthy, so the relay's first fetch
 * always fails. Backing off to five minutes there would leave a freshly
 * deployed relay publishing no edge for five minutes after its connector came
 * up.
 */
export const DEFAULT_EDGE_RETRY_MS = 5_000;

/** A connector's self-description, narrowed to what a relay reads from it. */
interface SelfDescription {
  httpEndpoint?: unknown;
  edgeIdentity?: { keyId?: unknown; publicKey?: unknown };
  settlements?: unknown;
  routes?: unknown;
  requiredTransport?: unknown;
}

/** What `edgeFromSelfDescription` answers: an edge, or why there is none. */
export type EdgeReading =
  | { edge: RelayWriteEdge; error?: undefined }
  | { edge?: undefined; error: string };

function isCarriage(value: unknown): value is Carriage {
  // `both` is the connector's permissive default and is NOT a pin. It reaches
  // the document as silence, never as a carriage a client should honour.
  return value === 'http' || value === 'btp';
}

function readSettlements(value: unknown): RelaySettlement[] {
  if (!Array.isArray(value)) return [];
  const settlements: RelaySettlement[] = [];
  for (const entry of value) {
    if (typeof entry !== 'object' || entry === null) continue;
    const { chain, tokenAddress, decimals } = entry as Record<string, unknown>;
    if (
      typeof chain === 'string' &&
      typeof tokenAddress === 'string' &&
      typeof decimals === 'number'
    ) {
      settlements.push({ chain, token: tokenAddress, decimals });
    }
  }
  return settlements;
}

/**
 * Narrow a connector's self-description to this relay's write edge.
 *
 * Pure, and the only place the connector's wire shape is understood. Every
 * failure is a sentence naming what was missing, because the caller's job on
 * a failure is to say so and advertise nothing.
 *
 * @param document - The parsed body of the connector's `GET /ilp`.
 * @param ilpAddress - The prefix whose route terminates at this relay.
 * @param carriagePin - The operator's carriage, used only when the connector
 *   states none.
 * @returns The edge, or the reason there is not one.
 */
export function edgeFromSelfDescription(
  document: unknown,
  ilpAddress: string,
  carriagePin?: Carriage
): EdgeReading {
  if (typeof document !== 'object' || document === null) {
    return {
      error: 'the connector answered something that is not a JSON object',
    };
  }
  const self = document as SelfDescription;

  const connectorUrl = self.httpEndpoint;
  if (typeof connectorUrl !== 'string' || connectorUrl.length === 0) {
    return {
      error:
        'the connector publishes no `httpEndpoint`, so it has no URL to send ' +
        'clients to (its [node] section is unset)',
    };
  }

  const sealKey = self.edgeIdentity?.publicKey;
  if (typeof sealKey !== 'string' || sealKey.length === 0) {
    return {
      error:
        'the connector publishes no `edgeIdentity.publicKey`, so there is no ' +
        'key for a client to seal a write to',
    };
  }

  const routes = Array.isArray(self.routes) ? self.routes : [];
  const prefixes = routes
    .map((route) =>
      typeof route === 'object' && route !== null
        ? (route as Record<string, unknown>)['prefix']
        : undefined
    )
    .filter((prefix): prefix is string => typeof prefix === 'string');

  const route = routes.find(
    (candidate) =>
      typeof candidate === 'object' &&
      candidate !== null &&
      (candidate as Record<string, unknown>)['prefix'] === ilpAddress
  ) as Record<string, unknown> | undefined;

  if (route === undefined) {
    // The drift guard. Advertising an address the connector does not
    // terminate would send every client's money to a route that refuses it.
    return {
      error:
        `the connector does not terminate \`${ilpAddress}\` — it terminates ` +
        `${prefixes.length > 0 ? prefixes.join(', ') : 'nothing'}. Point ` +
        'TOON_WRITE_ILP_ADDRESS at the prefix whose route reaches this ' +
        "relay's POST /write",
    };
  }

  // A price is a decimal STRING on the wire: a route's price is a u64 and does
  // not survive a JSON number in every client. It is µUSDC (§2), so it fits a
  // JS number comfortably once parsed, and a value that does not is refused
  // rather than rounded.
  const rawPrice = route['price'];
  const price = typeof rawPrice === 'string' ? Number(rawPrice) : rawPrice;
  if (typeof price !== 'number' || !Number.isSafeInteger(price) || price < 0) {
    return {
      error: `the connector prices \`${ilpAddress}\` at ${JSON.stringify(rawPrice)}, which is not a whole number of uusdc`,
    };
  }

  const routeCarriage = route['requiredTransport'];
  const nodeCarriage = self.requiredTransport;
  const carriage = isCarriage(routeCarriage)
    ? routeCarriage
    : isCarriage(nodeCarriage)
      ? nodeCarriage
      : carriagePin;

  return {
    edge: {
      ilp_address: ilpAddress,
      connector_url: connectorUrl,
      connector_seal_key: sealKey,
      ...(carriage !== undefined && { carriage }),
      price,
      settlement: readSettlements(self.settlements),
    },
  };
}

/** A running reader of the connector's self-description. */
export interface ConnectorEdgeWatcher {
  /** The edge as last read, or null while it is unknown. */
  current(): RelayWriteEdge | null;
  /** Read it once, now. Never rejects. */
  refresh(): Promise<void>;
  /** Stop re-reading. */
  stop(): void;
}

/** How to build a `ConnectorEdgeWatcher`. */
export interface ConnectorEdgeOptions {
  /** The connector's self-description URL, as this relay can reach it. */
  connectorUrl: string;
  /** The prefix whose route terminates at this relay's `POST /write`. */
  ilpAddress: string;
  /** The operator's carriage, used only where the connector states none. */
  carriage?: Carriage;
  /** How often a known edge is re-read (default: 5 minutes). */
  refreshMs?: number;
  /** How often an unknown one is retried (default: 5 seconds). */
  retryMs?: number;
  /** Injected for tests. */
  fetchImpl?: typeof fetch;
  /** Injected for tests; defaults to a console line per state change. */
  log?: (message: string) => void;
}

/**
 * Start reading the connector's self-description in the background.
 *
 * Deliberately non-blocking and never fatal. The canonical bundle starts the
 * connector only after the relay reports healthy, so a relay that waited for
 * its connector would deadlock its own deployment; and a connector that goes
 * away later must not take the relay's free reads with it. While the edge is
 * unknown the relay serves a NIP-11 document with no `toon` object and refuses
 * writes saying it does not publish one — which is the truth.
 *
 * It logs on CHANGE only: a first success, a first failure, and any later
 * transition. A five-second retry that logged every attempt would be a line
 * every five seconds for as long as a connector stayed down.
 *
 * @param options - Where to ask, and what to ask about.
 * @returns The watcher. Call `stop()` to end it.
 */
export function createConnectorEdgeWatcher(
  options: ConnectorEdgeOptions
): ConnectorEdgeWatcher {
  const {
    connectorUrl,
    ilpAddress,
    carriage,
    refreshMs = DEFAULT_EDGE_REFRESH_MS,
    retryMs = DEFAULT_EDGE_RETRY_MS,
    fetchImpl = fetch,
    log = (message: string) => console.log(message),
  } = options;

  let edge: RelayWriteEdge | null = null;
  let lastReport: string | null = null;
  let timer: NodeJS.Timeout | undefined;
  let stopped = false;

  const report = (message: string): void => {
    if (message === lastReport) return;
    lastReport = message;
    log(message);
  };

  const readOnce = async (): Promise<void> => {
    let reading: EdgeReading;
    try {
      const response = await fetchImpl(connectorUrl, {
        headers: { accept: 'application/json' },
      });
      if (!response.ok) {
        reading = { error: `${connectorUrl} answered HTTP ${response.status}` };
      } else {
        reading = edgeFromSelfDescription(
          await response.json(),
          ilpAddress,
          carriage
        );
      }
    } catch (error) {
      reading = {
        error: `${connectorUrl} could not be read: ${
          error instanceof Error ? error.message : String(error)
        }`,
      };
    }

    if (reading.edge === undefined) {
      edge = null;
      report(
        `[relay] paid write edge UNKNOWN: ${reading.error}. Until this is ` +
          'fixed the NIP-11 document names no edge and a refused write cannot ' +
          'say where to pay.'
      );
      return;
    }

    edge = reading.edge;
    report(
      `[relay] paid write edge: ${edge.ilp_address} at ${edge.connector_url}` +
        `${edge.carriage === undefined ? ' (no carriage pinned)' : ` over ${edge.carriage}`}` +
        `, ${edge.price} uusdc per write, sealed to ${edge.connector_seal_key.slice(0, 18)}…`
    );
  };

  const schedule = (): void => {
    if (stopped) return;
    // Unref'd: this poll must never be the reason a process stays alive.
    timer = setTimeout(
      () => {
        void readOnce().finally(schedule);
      },
      edge === null ? retryMs : refreshMs
    );
    timer.unref();
  };

  void readOnce().finally(schedule);

  return {
    current: () => edge,
    refresh: () => readOnce(),
    stop: () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    },
  };
}
