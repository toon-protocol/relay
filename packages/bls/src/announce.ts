/**
 * This node's own kind:10032 announce: how long it lives, and what keeps it
 * alive.
 *
 * A kind:10032 is how TOON clients discover peers — which node terminates a
 * destination, its BTP endpoint, its settlement addresses. It is a
 * **replaceable** event: a relay keeps the latest one per author, and the only
 * retraction path is a newer event signed by the same key. An announce with no
 * NIP-40 `expiration` tag is therefore served forever, outliving the node it
 * describes; and once that signing key is gone it can be neither replaced nor
 * NIP-09-deleted, so the litter is permanent by construction and clients keep
 * faithfully dialing a dead endpoint. Devnet is carrying exactly that today
 * (`b23599a6…` / `g.toon.swap.sol`, key gone, advertising a
 * `ws://127.0.0.1:3401` loopback literal that resolves to whatever machine
 * reads it).
 *
 * The expiry and the refresh loop are inseparable. Before this module the
 * announce was published ONCE at boot with no refresh of any kind, so an expiry
 * on its own would have taken a live node out of its own discovery one TTL
 * after start-up — strictly worse than the litter. {@link startSelfAnnounce}
 * therefore owns both halves: it publishes immediately and re-signs on a
 * cadence that comfortably beats the TTL.
 */
import { buildIlpPeerInfoEvent, type IlpPeerInfo } from '@toon-protocol/core';
import type { Event as NostrEvent } from 'nostr-tools/pure';

/**
 * Default NIP-40 time-to-live stamped on this node's own announce, in seconds.
 *
 * 600s is the fleet-wide convention, not a new number: it is what the Rust
 * connector's `[announce] ttl_secs` defaults to, and what every live devnet
 * announce already carries.
 */
export const DEFAULT_ANNOUNCE_TTL_SECONDS = 600;

/**
 * Default interval between republishes, in seconds.
 *
 * 240s against the 600s TTL above, matching every `connector announce` loop
 * overlay on the fleet (`REFRESH_SECS="${..._REFRESH_SECS:-240}"`) and leaving
 * the same ~6 minutes of continuous headroom that was measured, not assumed,
 * when this relay's NIP-40 enforcement landed. The ratio is the point.
 */
export const DEFAULT_ANNOUNCE_REFRESH_SECONDS = 240;

/** How long this node's announce lives, and how often it is renewed. */
export interface AnnounceSchedule {
  /**
   * NIP-40 TTL in seconds. `0` means "no expiration tag" — the pre-existing
   * behaviour, kept only as a deliberate escape hatch.
   */
  ttlSeconds: number;
  /**
   * Republish interval in seconds. `0` means "publish once at boot and never
   * again", which is only sane when `ttlSeconds` is also 0 or some OTHER
   * publisher on this same identity owns the refresh.
   */
  refreshSeconds: number;
}

/** Where diagnostics go. Defaults to the console; tests capture instead. */
export interface AnnounceLogger {
  log?: (message: string) => void;
  warn?: (message: string) => void;
  error?: (message: string) => void;
}

/**
 * Read an optional non-negative integer from an environment map.
 *
 * Never throws. Every service on this fleet auto-deploys on green main, so a
 * value rejected loudly here would be a crash loop on a live box rather than a
 * build failure; an unusable value falls back to the documented default with a
 * warning instead.
 */
function optionalNonNegativeInt(
  env: Record<string, string | undefined>,
  name: string,
  fallback: number,
  logger: AnnounceLogger
): number {
  const raw = env[name];
  if (raw === undefined || raw.trim() === '') return fallback;
  const parsed = Number(raw);
  if (!Number.isInteger(parsed) || parsed < 0) {
    logger.warn?.(
      `⚠️  ${name}="${raw}" is not a non-negative integer — using ${fallback}`
    );
    return fallback;
  }
  return parsed;
}

/**
 * Resolve the announce schedule from the environment.
 *
 * `ANNOUNCE_TTL_SECONDS` and `ANNOUNCE_REFRESH_SECONDS` are both OPTIONAL, both
 * default to the fleet convention, and neither can fail boot — see
 * {@link optionalNonNegativeInt} for why that is a hard requirement rather than
 * a preference. Combinations that would silently remove a live node from
 * discovery are reported loudly and then honoured.
 */
export function resolveAnnounceSchedule(
  env: Record<string, string | undefined>,
  logger: AnnounceLogger = console
): AnnounceSchedule {
  const ttlSeconds = optionalNonNegativeInt(
    env,
    'ANNOUNCE_TTL_SECONDS',
    DEFAULT_ANNOUNCE_TTL_SECONDS,
    logger
  );
  const refreshSeconds = optionalNonNegativeInt(
    env,
    'ANNOUNCE_REFRESH_SECONDS',
    DEFAULT_ANNOUNCE_REFRESH_SECONDS,
    logger
  );

  if (ttlSeconds === 0) {
    logger.warn?.(
      '⚠️  ANNOUNCE_TTL_SECONDS=0 — this announce carries no NIP-40 expiration and will be served forever. It is a replaceable event: once this key is gone, nothing can retract it.'
    );
  } else if (refreshSeconds === 0) {
    logger.warn?.(
      `⚠️  ANNOUNCE_REFRESH_SECONDS=0 — the kind:10032 is published once and never renewed, so it expires after ${ttlSeconds}s and this node vanishes from discovery.`
    );
  } else if (refreshSeconds >= ttlSeconds) {
    // The one failure mode worse than the litter: a LIVE node that expires out
    // of discovery in the gap between two of its own announces.
    logger.error?.(
      `⚠️  ANNOUNCE_REFRESH_SECONDS=${refreshSeconds} is not shorter than ANNOUNCE_TTL_SECONDS=${ttlSeconds} — this node will expire out of discovery between its own republishes (fleet convention: 240s against a 600s TTL).`
    );
  }

  return { ttlSeconds, refreshSeconds };
}

/** A running self-announce, stopped on shutdown. */
export interface SelfAnnounce {
  /** The first announce, published synchronously by `startSelfAnnounce`. */
  readonly initialEvent: NostrEvent;
  /** Stop refreshing. Idempotent. */
  stop(): void;
}

/**
 * Publish this node's kind:10032 now, then keep republishing it inside its own
 * expiry window.
 *
 * Each round RE-SIGNS rather than re-sending: the NIP-40 tag is
 * `created_at + ttlSeconds`, so a cached event would advertise an expiry that
 * recedes into the past however often the loop ran — the node would drop out of
 * discovery while still cheerfully publishing.
 *
 * @param publish - Where the signed event goes. The genesis-peer path stores
 *   straight into this node's own event store.
 */
export function startSelfAnnounce(params: {
  info: IlpPeerInfo;
  secretKey: Uint8Array;
  schedule: AnnounceSchedule;
  publish: (event: NostrEvent) => void;
  logger?: AnnounceLogger;
}): SelfAnnounce {
  const { info, secretKey, schedule, publish } = params;
  const logger = params.logger ?? console;

  const signAndPublish = (): NostrEvent => {
    const event = buildIlpPeerInfoEvent(info, secretKey, {
      // Non-positive → core omits the tag entirely, which is the documented
      // "never expires" escape hatch `resolveAnnounceSchedule` already warned
      // about.
      ttlSeconds: schedule.ttlSeconds,
    });
    publish(event);
    return event;
  };

  const initialEvent = signAndPublish();

  let timer: ReturnType<typeof setInterval> | undefined;
  if (schedule.refreshSeconds > 0) {
    timer = setInterval(() => {
      try {
        signAndPublish();
      } catch (error) {
        // A failed round is not retried out of band — the next refresh is the
        // retry, and it lands well inside the TTL.
        logger.error?.(`⚠️  kind:10032 refresh failed: ${String(error)}`);
      }
    }, schedule.refreshSeconds * 1000);
    // Never hold the process (or a test runner) open on an advertisement
    // refresh.
    timer.unref?.();
  }

  return {
    initialEvent,
    stop(): void {
      if (timer !== undefined) {
        clearInterval(timer);
        timer = undefined;
      }
    },
  };
}
