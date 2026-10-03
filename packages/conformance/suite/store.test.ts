import { generateSecretKey } from 'nostr-tools/pure';
import { afterAll, beforeAll, describe, expect } from 'vitest';
import {
  removeVolume,
  startRelay,
  type RunningRelay,
} from './harness/relay-container.js';
import {
  now,
  pubkeyOf,
  publish,
  publishOk,
  query,
  settle,
  sign,
  storedIds,
  subscribe,
  until,
} from './harness/wire.js';
import {
  conformanceTest,
  conformanceTestEach,
  imageUnderTest,
} from './implementation.js';

let relay: RunningRelay;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
});

afterAll(async () => {
  await relay?.stop();
});

/** Every test uses its own author, so tests cannot see each other's events. */
const author = () => {
  const secretKey = generateSecretKey();
  return { secretKey, pubkey: pubkeyOf(secretKey) };
};

const ids = (...events: { id: string }[]) => events.map((e) => e.id).sort();

describe('relay image conformance: replaceable kinds', () => {
  conformanceTestEach(
    [10000, 10002, 19999],
    (kind) => `kind ${kind} keeps the latest event per author and kind`,
    async (kind) => {
      const { secretKey, pubkey } = author();
      const t = now();
      const older = sign(secretKey, { kind, created_at: t - 20, content: 'a' });
      const newer = sign(secretKey, { kind, created_at: t - 10, content: 'b' });
      // Newest first, so keeping it is not just "last write wins".
      await publishOk(relay, newer);
      await publishOk(relay, older);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [kind] })
      ).toEqual(ids(newer));

      const newest = sign(secretKey, { kind, created_at: t, content: 'c' });
      await publishOk(relay, newest);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [kind] })
      ).toEqual(ids(newest));
    }
  );

  conformanceTestEach(
    [true, false],
    (higherFirst) =>
      `a tie on created_at keeps the lower id (${higherFirst ? 'higher' : 'lower'} id first)`,
    async (higherFirst) => {
      const { secretKey, pubkey } = author();
      const created_at = now() - 10;
      const a = sign(secretKey, { kind: 10002, created_at, content: 'a' });
      const b = sign(secretKey, { kind: 10002, created_at, content: 'b' });
      const [lower, higher] = a.id < b.id ? [a, b] : [b, a];
      const order = higherFirst ? [higher, lower] : [lower, higher];
      for (const event of order) await publishOk(relay, event);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [10002] })
      ).toEqual(ids(lower));
    }
  );

  conformanceTestEach(
    [0, 3],
    (kind) => `kind ${kind} is replaceable`,
    async (kind) => {
      const { secretKey, pubkey } = author();
      const t = now();
      const older = sign(secretKey, { kind, created_at: t - 10, content: 'a' });
      const newer = sign(secretKey, { kind, created_at: t, content: 'b' });
      await publishOk(relay, older);
      await publishOk(relay, newer);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [kind] })
      ).toEqual(ids(newer));
    }
  );
});

describe('relay image conformance: addressable kinds', () => {
  conformanceTestEach(
    [30000, 30023, 39999, 10032, 10050, 10099],
    (kind) => `kind ${kind} keeps the latest event per author, kind and d tag`,
    async (kind) => {
      const { secretKey, pubkey } = author();
      const t = now();
      const aOld = sign(secretKey, {
        kind,
        created_at: t - 20,
        tags: [['d', 'a']],
      });
      const aNew = sign(secretKey, {
        kind,
        created_at: t - 10,
        tags: [['d', 'a']],
      });
      const b = sign(secretKey, {
        kind,
        created_at: t - 30,
        tags: [['d', 'b']],
      });
      await publishOk(relay, aNew);
      await publishOk(relay, aOld);
      await publishOk(relay, b);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [kind] })
      ).toEqual(ids(aNew, b));
    }
  );

  conformanceTest('a missing d tag is the empty d tag', async () => {
    const { secretKey, pubkey } = author();
    const t = now();
    const bare = sign(secretKey, { kind: 30023, created_at: t - 10 });
    const empty = sign(secretKey, {
      kind: 30023,
      created_at: t,
      tags: [['d', '']],
    });
    await publishOk(relay, bare);
    await publishOk(relay, empty);
    expect(
      await storedIds(relay, { authors: [pubkey], kinds: [30023] })
    ).toEqual(ids(empty));
  });

  conformanceTest(
    'a kind just outside 10032-10099 is replaceable, not addressable',
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const older = sign(secretKey, {
        kind: 10100,
        created_at: t - 10,
        tags: [['d', 'a']],
      });
      const newer = sign(secretKey, {
        kind: 10100,
        created_at: t,
        tags: [['d', 'b']],
      });
      await publishOk(relay, older);
      await publishOk(relay, newer);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [10100] })
      ).toEqual(ids(newer));
    }
  );

  conformanceTest(
    'a d tag containing % or _ replaces only its own event',
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const plain = sign(secretKey, {
        kind: 30023,
        created_at: t - 30,
        tags: [['d', 'abc']],
      });
      const underscore = sign(secretKey, {
        kind: 30023,
        created_at: t - 20,
        tags: [['d', 'a_c']],
      });
      const percent = sign(secretKey, {
        kind: 30023,
        created_at: t - 10,
        tags: [['d', 'a%']],
      });
      for (const event of [plain, underscore, percent]) {
        await publishOk(relay, event);
      }
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [30023] })
      ).toEqual(ids(plain, underscore, percent));

      const underscoreNew = sign(secretKey, {
        kind: 30023,
        created_at: t,
        tags: [['d', 'a_c']],
      });
      await publishOk(relay, underscoreNew);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [30023] })
      ).toEqual(ids(plain, underscoreNew, percent));
    }
  );

  conformanceTest(
    'd tags differing only by case are different addresses',
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const lower = sign(secretKey, {
        kind: 30023,
        created_at: t - 10,
        tags: [['d', 'foo']],
      });
      const upper = sign(secretKey, {
        kind: 30023,
        created_at: t,
        tags: [['d', 'FOO']],
      });
      await publishOk(relay, lower);
      await publishOk(relay, upper);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [30023] })
      ).toEqual(ids(lower, upper));
    }
  );
});

describe('relay image conformance: tag filters', () => {
  conformanceTest(
    'a tag filter value containing % or _ matches only that exact value',
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const values = ['100%', '1000', 'a_c', 'abc', 'ABC'];
      const idOf: Record<string, string> = {};
      for (const [i, value] of values.entries()) {
        const event = sign(secretKey, {
          kind: 1,
          created_at: t - i,
          tags: [['t', value]],
        });
        idOf[value] = event.id;
        await publishOk(relay, event);
      }
      for (const value of ['100%', 'a_c']) {
        expect(
          await storedIds(relay, { authors: [pubkey], '#t': [value] })
        ).toEqual([idOf[value] ?? '']);
      }
    }
  );

  conformanceTest(
    'a multi-letter tag key behaves the same for stored results and live delivery',
    async () => {
      const { secretKey, pubkey } = author();
      const filter = { authors: [pubkey], '#ab': ['x'] };
      const subscription = await subscribe(relay, filter);
      try {
        const t = now();
        const tagged = (value: string, i: number) =>
          sign(secretKey, {
            kind: 1,
            created_at: t + i,
            content: String(i),
            tags: [['ab', value]],
          });
        const first = tagged('x', 0);
        const miss = tagged('y', 1);
        const second = tagged('x', 2);
        for (const event of [first, miss, second]) {
          await publishOk(relay, event);
        }
        const matching = ids(first, second);
        expect(await storedIds(relay, filter)).toEqual(matching);
        await until(() => subscription.delivered.length >= matching.length);
        await settle();
        const live = subscription.delivered.map((e) => e.id).sort();
        expect(live).toEqual(matching);
      } finally {
        subscription.close();
      }
    }
  );
});

describe('relay image conformance: multi-letter tag keys and limit', () => {
  conformanceTest(
    'a multi-letter tag key is applied before the limit, past the 500 cap',
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const make = (i: number, tags: string[][]) =>
        sign(secretKey, {
          kind: 1,
          created_at: t - 1000 + i,
          content: String(i),
          tags,
        });
      // Three tagged events, older than more than the cap of untagged ones.
      const tagged = [0, 1, 2].map((i) => make(i, [['ab', 'x']]));
      const newestTagged = tagged.slice(-1);
      const untagged = Array.from({ length: 501 }, (_, i) => make(10 + i, []));
      for (const event of tagged) await publishOk(relay, event);
      for (let i = 0; i < untagged.length; i += 50) {
        await Promise.all(
          untagged.slice(i, i + 50).map((event) => publishOk(relay, event))
        );
      }
      expect(
        await storedIds(relay, { authors: [pubkey], '#ab': ['x'] })
      ).toEqual(ids(...tagged));
      const newest = await query(relay, {
        authors: [pubkey],
        '#ab': ['x'],
        limit: 1,
      });
      expect(newest.map((e) => e.id)).toEqual(ids(...newestTagged));
    }
  );
});

describe('relay image conformance: deletion (kind 5)', () => {
  conformanceTest(
    "a kind 5 removes the author's own event by id, and it stays removed",
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const target = sign(secretKey, { kind: 1, created_at: t - 10 });
      const keeper = sign(secretKey, {
        kind: 1,
        created_at: t - 10,
        content: 'k',
      });
      await publishOk(relay, target);
      await publishOk(relay, keeper);
      await publishOk(
        relay,
        sign(secretKey, { kind: 5, created_at: t, tags: [['e', target.id]] })
      );
      expect(await storedIds(relay, { authors: [pubkey], kinds: [1] })).toEqual(
        ids(keeper)
      );

      // Re-submission does not bring it back, however the write is answered.
      await publish(relay, target);
      expect(await storedIds(relay, { authors: [pubkey], kinds: [1] })).toEqual(
        ids(keeper)
      );
    }
  );

  conformanceTest(
    "a kind 5 removes the author's own addressable event by address, and it stays removed",
    async () => {
      const { secretKey, pubkey } = author();
      const t = now();
      const target = sign(secretKey, {
        kind: 30023,
        created_at: t - 10,
        tags: [['d', 'doomed']],
      });
      const other = sign(secretKey, {
        kind: 30023,
        created_at: t - 10,
        tags: [['d', 'kept']],
      });
      await publishOk(relay, target);
      await publishOk(relay, other);
      await publishOk(
        relay,
        sign(secretKey, {
          kind: 5,
          created_at: t,
          tags: [['a', `30023:${pubkey}:doomed`]],
        })
      );
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [30023] })
      ).toEqual(ids(other));

      await publish(relay, target);
      expect(
        await storedIds(relay, { authors: [pubkey], kinds: [30023] })
      ).toEqual(ids(other));
    }
  );

  conformanceTest(
    "a kind 5 cannot delete another author's events",
    async () => {
      const victim = author();
      const attacker = author();
      const t = now();
      const byId = sign(victim.secretKey, { kind: 1, created_at: t - 10 });
      const byAddress = sign(victim.secretKey, {
        kind: 30023,
        created_at: t - 10,
        tags: [['d', 'mine']],
      });
      await publishOk(relay, byId);
      await publishOk(relay, byAddress);
      await publishOk(
        relay,
        sign(attacker.secretKey, {
          kind: 5,
          created_at: t,
          tags: [
            ['e', byId.id],
            ['a', `30023:${victim.pubkey}:mine`],
          ],
        })
      );
      expect(await storedIds(relay, { authors: [victim.pubkey] })).toEqual(
        ids(byId, byAddress)
      );

      // Nor does the attacker's request stop the victim publishing it again.
      const later = sign(victim.secretKey, {
        kind: 30023,
        created_at: t,
        tags: [['d', 'mine']],
      });
      await publishOk(relay, later);
      expect(
        await storedIds(relay, { authors: [victim.pubkey], kinds: [30023] })
      ).toEqual(ids(later));
    }
  );
});

describe('relay image conformance: duplicates', () => {
  conformanceTest(
    'a regular event submitted twice is stored once',
    async () => {
      const { secretKey, pubkey } = author();
      const event = sign(secretKey, { kind: 1, content: 'twice' });
      await publishOk(relay, event);
      await publish(relay, event);
      const found = await query(relay, { authors: [pubkey] });
      expect(found.map((e) => e.id)).toEqual([event.id]);
    }
  );
});

describe('relay image conformance: expiration enforced', () => {
  conformanceTest(
    'an expired event is neither returned nor delivered',
    async () => {
      const { secretKey, pubkey } = author();
      const subscription = await subscribe(relay, { authors: [pubkey] });
      try {
        const t = now();
        const expired = sign(secretKey, {
          kind: 1,
          created_at: t - 200,
          tags: [['expiration', String(t - 100)]],
        });
        const live = sign(secretKey, {
          kind: 1,
          created_at: t - 200,
          content: 'live',
          tags: [['expiration', String(t + 3600)]],
        });
        await publish(relay, expired);
        await publishOk(relay, live);
        await until(() => subscription.delivered.length > 0);
        await settle();
        expect(await storedIds(relay, { authors: [pubkey] })).toEqual(
          ids(live)
        );
        expect(await storedIds(relay, { ids: [expired.id] })).toEqual([]);
        expect(subscription.delivered.map((e) => e.id)).toEqual([live.id]);
      } finally {
        subscription.close();
      }
    }
  );
});

describe('relay image conformance: an unexpiring event is never reaped', () => {
  const lax = { TOON_ENFORCE_EXPIRATION: 'false' };
  const reaping = {
    TOON_ENFORCE_EXPIRATION: 'true',
    TOON_EXPIRATION_REAP_INTERVAL_SECONDS: '1',
    TOON_EXPIRATION_REAP_GRACE_SECONDS: '0',
  };

  conformanceTest(
    'survives sweeps and a restart over the same volume',
    async () => {
      const volume = `conformance-reaper-${process.pid}-${Date.now()}`;
      const { secretKey, pubkey } = author();
      const t = now();
      const event = (d: string, created_at: number, tags: string[][] = []) =>
        sign(secretKey, {
          kind: 30078,
          created_at,
          content: d,
          tags: [['d', d], ...tags],
        });
      // No expiration tag at all, including very old ones (#160).
      const unexpiring = [
        event('recent', t - 10),
        event('y2020', 1_600_000_000),
        event('y1970', 1_000),
      ];
      // Parsing fails open: none of these is a plain non-negative integer.
      const malformed = ['abc', '-5', '1.5', '', '1e3', '+5', ' 12'].map(
        (value, i) => event(`malformed-${i}`, t - 10, [['expiration', value]])
      );
      const kept = ids(...unexpiring, ...malformed);
      const expiredBeforeBoot = event('expired-before-boot', t - 200, [
        ['expiration', String(t - 100)],
      ]);
      const expiredWhileSweeping = event('expired-while-sweeping', t - 200, [
        ['expiration', String(t - 100)],
      ]);

      let running: RunningRelay | undefined;
      const stored = () => {
        if (!running) throw new Error('no relay running');
        return storedIds(running, { authors: [pubkey] });
      };
      // One relay at a time over the volume, stopped before the next starts.
      const restart = async (env: Record<string, string>) => {
        const previous = running;
        running = undefined;
        await previous?.stop();
        running = await startRelay(imageUnderTest(), { volume, env });
        return running;
      };
      try {
        // Not enforcing, so an expired event is still held and returned.
        let current = await restart(lax);
        await publishOk(current, expiredBeforeBoot);
        expect(await stored()).toEqual(ids(expiredBeforeBoot));

        // Enforcing, and sweeping every second with no grace. Everything is
        // written after the boot sweep, while the interval sweeps run.
        current = await restart(reaping);
        await settle(1_500);
        for (const e of [...unexpiring, ...malformed, expiredWhileSweeping]) {
          await publishOk(current, e);
        }
        await settle(4_000);
        expect(await stored()).toEqual(kept);

        // Look again with the reaper off and expiry no longer hiding: both
        // expired events are gone from disk, the one written after boot
        // included, so the interval sweeps really ran, and nothing else went
        // with them.
        await restart(lax);
        expect(await stored()).toEqual(kept);

        // A restart with the reaper running: the boot sweep included.
        await restart(reaping);
        await settle(2_500);
        expect(await stored()).toEqual(kept);
      } finally {
        await running?.stop();
        await removeVolume(volume);
      }
    }
  );
});

describe('relay image conformance: expiration not enforced', () => {
  let lax: RunningRelay;

  beforeAll(async () => {
    lax = await startRelay(imageUnderTest(), {
      env: { TOON_ENFORCE_EXPIRATION: 'false' },
    });
  });

  afterAll(async () => {
    await lax?.stop();
  });

  conformanceTest('an expired event is returned and delivered', async () => {
    const { secretKey, pubkey } = author();
    const subscription = await subscribe(lax, { authors: [pubkey] });
    try {
      const t = now();
      const expired = sign(secretKey, {
        kind: 1,
        created_at: t - 200,
        tags: [['expiration', String(t - 100)]],
      });
      await publishOk(lax, expired);
      await until(() => subscription.delivered.length > 0);
      expect(await storedIds(lax, { authors: [pubkey] })).toEqual(ids(expired));
      expect(subscription.delivered.map((e) => e.id)).toEqual([expired.id]);
    } finally {
      subscription.close();
    }
  });
});

describe('relay image conformance: operator blocklist', () => {
  let blocking: RunningRelay;
  // The block names an id, so the event must exist before the relay starts.
  const secretKey = generateSecretKey();
  const blocked = sign(secretKey, { kind: 1, content: 'blocked' });

  beforeAll(async () => {
    blocking = await startRelay(imageUnderTest(), {
      env: { TOON_BLOCKED_EVENT_IDS: blocked.id },
    });
  });

  afterAll(async () => {
    await blocking?.stop();
  });

  conformanceTest(
    'a blocklisted event id is never stored or returned',
    async () => {
      const pubkey = pubkeyOf(secretKey);
      const other = sign(secretKey, { kind: 1, content: 'not blocked' });
      // Refused or silently dropped: either way, it must not come back.
      await publish(blocking, blocked);
      await publishOk(blocking, other);
      expect(await storedIds(blocking, { ids: [blocked.id] })).toEqual([]);
      expect(await storedIds(blocking, { authors: [pubkey] })).toEqual(
        ids(other)
      );
    }
  );
});
