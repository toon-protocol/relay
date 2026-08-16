/**
 * A kind:10032 announce with no NIP-40 `expiration` tag is permanent litter.
 *
 * This relay now refuses to SERVE an expired announce (#137). That fix has a
 * hole while any publisher still emits announces that never expire — and this
 * node's own genesis-peer self-announce was one of them, written straight into
 * the event store it serves from. kind:10032 is a replaceable event whose only
 * retraction path is a newer event signed by the same key, so once that key is
 * gone the advertisement is permanent by construction.
 *
 * The expiry and the refresh loop are two halves of one fix, and both are
 * pinned here: half one alone would take a LIVE node out of its own discovery
 * one TTL after boot, which is worse than the litter it removes.
 */
import { describe, it, expect, vi } from 'vitest';
import { generateSecretKey, type Event as NostrEvent } from 'nostr-tools/pure';
import type { IlpPeerInfo } from '@toon-protocol/core';

import {
  DEFAULT_ANNOUNCE_REFRESH_SECONDS,
  DEFAULT_ANNOUNCE_TTL_SECONDS,
  resolveAnnounceSchedule,
  startSelfAnnounce,
  type AnnounceLogger,
} from './announce.js';

const INFO: IlpPeerInfo = {
  pubkey: 'a'.repeat(64),
  ilpAddress: 'g.toon.test',
  btpEndpoint: 'wss://example.invalid/btp',
  assetCode: 'USDC',
  assetScale: 6,
};

/** Collects every diagnostic by severity, so a test can assert on one. */
function captureLogger(): AnnounceLogger & {
  logs: string[];
  warns: string[];
  errors: string[];
} {
  const logs: string[] = [];
  const warns: string[] = [];
  const errors: string[] = [];
  return {
    logs,
    warns,
    errors,
    log: (m) => logs.push(m),
    warn: (m) => warns.push(m),
    error: (m) => errors.push(m),
  };
}

/** The NIP-40 `expiration` tag value, or `undefined` when there is none. */
function expirationOf(event: NostrEvent): number | undefined {
  const tag = event.tags.find((t) => t[0] === 'expiration');
  return tag?.[1] === undefined ? undefined : Number(tag[1]);
}

describe('resolveAnnounceSchedule', () => {
  it('[P0] defaults to the fleet convention with NOTHING configured — the TTL must never become a required env var, because this fleet auto-deploys on green main', () => {
    const logger = captureLogger();
    expect(resolveAnnounceSchedule({}, logger)).toEqual({
      ttlSeconds: DEFAULT_ANNOUNCE_TTL_SECONDS,
      refreshSeconds: DEFAULT_ANNOUNCE_REFRESH_SECONDS,
    });
    expect(DEFAULT_ANNOUNCE_TTL_SECONDS).toBe(600);
    expect(DEFAULT_ANNOUNCE_REFRESH_SECONDS).toBe(240);
    // The healthy default configuration is silent.
    expect(logger.warns).toEqual([]);
    expect(logger.errors).toEqual([]);
  });

  it('[P0] the default refresh comfortably beats the default TTL — otherwise a live node expires out of discovery between its own announces', () => {
    const { ttlSeconds, refreshSeconds } = resolveAnnounceSchedule(
      {},
      captureLogger()
    );
    expect(refreshSeconds).toBeLessThan(ttlSeconds);
    // ~6 minutes of continuous headroom, the margin measured live for #137.
    expect(ttlSeconds - refreshSeconds).toBeGreaterThanOrEqual(300);
  });

  it('[P0] honours explicit overrides', () => {
    expect(
      resolveAnnounceSchedule(
        { ANNOUNCE_TTL_SECONDS: '1800', ANNOUNCE_REFRESH_SECONDS: '600' },
        captureLogger()
      )
    ).toEqual({ ttlSeconds: 1800, refreshSeconds: 600 });
  });

  it.each([['not-a-number'], ['-1'], ['12.5'], ['   ']])(
    '[P0] falls back to the default on the unusable value %j rather than throwing — a rejected value here would crash-loop a live box, not fail a build',
    (raw) => {
      const logger = captureLogger();
      const schedule = resolveAnnounceSchedule(
        { ANNOUNCE_TTL_SECONDS: raw },
        logger
      );
      expect(schedule.ttlSeconds).toBe(DEFAULT_ANNOUNCE_TTL_SECONDS);
    }
  );

  it('[P0] warns that a zero TTL is unretractable — the pre-existing behaviour, kept only as a deliberate escape hatch', () => {
    const logger = captureLogger();
    expect(
      resolveAnnounceSchedule({ ANNOUNCE_TTL_SECONDS: '0' }, logger).ttlSeconds
    ).toBe(0);
    expect(logger.warns.join('\n')).toMatch(/served forever/);
  });

  it('[P0] reports a refresh that does not beat the TTL at error — the one misconfiguration worse than the litter', () => {
    const logger = captureLogger();
    resolveAnnounceSchedule(
      { ANNOUNCE_TTL_SECONDS: '60', ANNOUNCE_REFRESH_SECONDS: '60' },
      logger
    );
    expect(logger.errors.join('\n')).toMatch(
      /expire out of discovery between its own republishes/
    );
  });

  it('[P0] warns when the refresh is disabled but the TTL is not', () => {
    const logger = captureLogger();
    resolveAnnounceSchedule({ ANNOUNCE_REFRESH_SECONDS: '0' }, logger);
    expect(logger.warns.join('\n')).toMatch(/published once and never renewed/);
  });
});

describe('startSelfAnnounce', () => {
  it('[P0] stamps expiration = created_at + ttl on the very first publish — the tag that used to be absent entirely', () => {
    const published: NostrEvent[] = [];
    const announce = startSelfAnnounce({
      info: INFO,
      secretKey: generateSecretKey(),
      schedule: { ttlSeconds: 600, refreshSeconds: 0 },
      publish: (e) => published.push(e),
      logger: captureLogger(),
    });
    try {
      expect(published).toHaveLength(1);
      const event = published[0] as NostrEvent;
      expect(event.kind).toBe(10032);
      expect(expirationOf(event)).toBe(event.created_at + 600);
    } finally {
      announce.stop();
    }
  });

  it('[P0] a zero TTL publishes no expiration tag — the escape hatch, and the only way back to the old behaviour', () => {
    const published: NostrEvent[] = [];
    const announce = startSelfAnnounce({
      info: INFO,
      secretKey: generateSecretKey(),
      schedule: { ttlSeconds: 0, refreshSeconds: 0 },
      publish: (e) => published.push(e),
      logger: captureLogger(),
    });
    try {
      expect(expirationOf(published[0] as NostrEvent)).toBeUndefined();
    } finally {
      announce.stop();
    }
  });

  it('[P0] republishes on the refresh interval, RE-SIGNING each round so the expiry slides forward instead of receding into the past', () => {
    vi.useFakeTimers();
    // Fixed wall clock, advanced explicitly: `created_at` has second
    // granularity, so a re-signed event inside the same second is legitimately
    // byte-identical and would prove nothing about re-signing.
    vi.setSystemTime(new Date('2026-08-16T00:00:00Z'));
    try {
      const published: NostrEvent[] = [];
      const announce = startSelfAnnounce({
        info: INFO,
        secretKey: generateSecretKey(),
        schedule: { ttlSeconds: 600, refreshSeconds: 240 },
        publish: (e) => published.push(e),
        logger: captureLogger(),
      });
      try {
        expect(published).toHaveLength(1);
        vi.advanceTimersByTime(240_000);
        vi.advanceTimersByTime(240_000);
        expect(published).toHaveLength(3);

        const [first, , third] = published as [
          NostrEvent,
          NostrEvent,
          NostrEvent,
        ];
        expect(third.created_at).toBe(first.created_at + 480);
        expect(third.id).not.toBe(first.id);
        // Every round's expiry is measured from ITS OWN publish time, and every
        // round lands well inside the previous round's window.
        for (const event of published) {
          expect(expirationOf(event)).toBe(event.created_at + 600);
        }
        expect(third.created_at).toBeLessThan(
          expirationOf(first) as number // the node never went dark in between
        );
      } finally {
        announce.stop();
      }
    } finally {
      vi.useRealTimers();
    }
  });

  it('[P0] stop() ends the loop — a stopped node must stop renewing its own liveness signal, or the TTL means nothing', () => {
    vi.useFakeTimers();
    try {
      const published: NostrEvent[] = [];
      const announce = startSelfAnnounce({
        info: INFO,
        secretKey: generateSecretKey(),
        schedule: { ttlSeconds: 600, refreshSeconds: 240 },
        publish: (e) => published.push(e),
        logger: captureLogger(),
      });
      vi.advanceTimersByTime(240_000);
      expect(published).toHaveLength(2);

      announce.stop();
      announce.stop(); // idempotent
      vi.advanceTimersByTime(240_000 * 10);
      expect(published).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('[P0] a refresh of 0 publishes once and never again, but still expiring — the TTL is independent of who renews it', () => {
    vi.useFakeTimers();
    try {
      const published: NostrEvent[] = [];
      const announce = startSelfAnnounce({
        info: INFO,
        secretKey: generateSecretKey(),
        schedule: { ttlSeconds: 600, refreshSeconds: 0 },
        publish: (e) => published.push(e),
        logger: captureLogger(),
      });
      try {
        vi.advanceTimersByTime(600_000);
        expect(published).toHaveLength(1);
        expect(expirationOf(published[0] as NostrEvent)).toBeDefined();
      } finally {
        announce.stop();
      }
    } finally {
      vi.useRealTimers();
    }
  });

  it('[P0] a throwing publish does not kill the loop — the next refresh is the retry, and it still lands inside the TTL', () => {
    vi.useFakeTimers();
    try {
      const published: NostrEvent[] = [];
      let failNext = false;
      const logger = captureLogger();
      const announce = startSelfAnnounce({
        info: INFO,
        secretKey: generateSecretKey(),
        schedule: { ttlSeconds: 600, refreshSeconds: 240 },
        publish: (e) => {
          if (failNext) throw new Error('store unavailable');
          published.push(e);
        },
        logger,
      });
      try {
        failNext = true;
        vi.advanceTimersByTime(240_000);
        expect(published).toHaveLength(1);
        expect(logger.errors.join('\n')).toMatch(/refresh failed/);

        failNext = false;
        vi.advanceTimersByTime(240_000);
        expect(published).toHaveLength(2);
      } finally {
        announce.stop();
      }
    } finally {
      vi.useRealTimers();
    }
  });
});
