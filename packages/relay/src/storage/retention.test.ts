/**
 * Retention behaviour shared by every EventStore: NIP-40 expiry enforcement,
 * NIP-09 deletion, and the operator blocklist (relay#137).
 *
 * The two stores are exercised through the same table so they cannot drift:
 * the relay serves reads from SqliteEventStore in production and
 * InMemoryEventStore in embedded/test topologies, and a retention rule that
 * held on only one of them would be a rule that does not hold.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { existsSync, unlinkSync } from 'node:fs';
import Database from 'better-sqlite3';
import type { NostrEvent } from 'nostr-tools/pure';
import { InMemoryEventStore } from './InMemoryEventStore.js';
import type { EventStore, EventStoreOptions } from './InMemoryEventStore.js';
import { SqliteEventStore } from './SqliteEventStore.js';

const AUTHOR = 'a'.repeat(64);
const STRANGER = 'b'.repeat(64);
// Real wall-clock seconds: the stores compare `expiration` against Date.now()
// internally, so a frozen constant would drift out from under the "not yet
// expired" cases as the suite runs.
const NOW = Math.floor(Date.now() / 1000);

let eventCounter = 0;

function makeEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  eventCounter += 1;
  return {
    id: eventCounter.toString(16).padStart(64, '0'),
    pubkey: AUTHOR,
    kind: 1,
    content: 'hello',
    tags: [],
    created_at: NOW - 60,
    sig: 's'.repeat(128),
    ...overrides,
  };
}

/** A kind:10032 node announce, the event class this work exists for. */
function makeAnnounce(
  pubkey: string,
  createdAt: number,
  expiresAt?: number
): NostrEvent {
  return makeEvent({
    pubkey,
    kind: 10032,
    created_at: createdAt,
    content: JSON.stringify({ ilpAddress: 'g.toon.example' }),
    tags: expiresAt === undefined ? [] : [['expiration', String(expiresAt)]],
  });
}

const SQLITE_DB_PATH = './test-retention.db';

function cleanupSqliteFiles(): void {
  for (const path of [
    SQLITE_DB_PATH,
    `${SQLITE_DB_PATH}-wal`,
    `${SQLITE_DB_PATH}-shm`,
  ]) {
    if (existsSync(path)) unlinkSync(path);
  }
}

interface StoreFactory {
  name: string;
  create(options?: EventStoreOptions): EventStore;
}

const openStores: EventStore[] = [];

const FACTORIES: StoreFactory[] = [
  {
    name: 'InMemoryEventStore',
    create(options) {
      const store = new InMemoryEventStore(options);
      openStores.push(store);
      return store;
    },
  },
  {
    name: 'SqliteEventStore',
    create(options) {
      const store = new SqliteEventStore(':memory:', options);
      openStores.push(store);
      return store;
    },
  },
];

afterEach(() => {
  for (const store of openStores.splice(0)) store.close?.();
  cleanupSqliteFiles();
});

describe.each(FACTORIES)('$name — NIP-40 expiry enforcement', ({ create }) => {
  it('does not serve an expired event from history', () => {
    const store = create();
    store.store(makeAnnounce(AUTHOR, NOW - 700, NOW - 100));

    expect(store.query([{ kinds: [10032] }])).toEqual([]);
  });

  it('does not serve an expired event by id', () => {
    const store = create();
    const expired = makeAnnounce(AUTHOR, NOW - 700, NOW - 100);
    store.store(expired);

    expect(store.get(expired.id)).toBeUndefined();
  });

  it('still serves an event that has not expired yet', () => {
    const store = create();
    const live = makeAnnounce(AUTHOR, NOW, NOW + 600);
    store.store(live);

    expect(store.query([{ kinds: [10032] }]).map((e) => e.id)).toEqual([
      live.id,
    ]);
    expect(store.get(live.id)?.id).toBe(live.id);
  });

  // The dead swap maker on devnet (g.toon.swap.sol at ws://127.0.0.1:3401)
  // carries NO expiration tag. NIP-40 is not the mechanism that removes it,
  // and pretending otherwise would be worse than leaving it: an event with
  // no expiration must never be treated as expired.
  it('serves an event that carries no expiration tag, however old', () => {
    const store = create();
    const permanent = makeAnnounce(AUTHOR, 0);
    store.store(permanent);

    expect(store.get(permanent.id)?.id).toBe(permanent.id);
    expect(store.query([{ kinds: [10032] }])).toHaveLength(1);
  });

  // The exact devnet symptom: a retired identity's announce outliving its
  // stated ten-minute lifetime while the live node's announce sits beside it.
  it('serves only the live announce when a stale one shares the address', () => {
    const store = create();
    const retired = makeAnnounce(STRANGER, NOW - 600_000, NOW - 599_400);
    const live = makeAnnounce(AUTHOR, NOW - 60, NOW + 540);
    store.store(retired);
    store.store(live);

    expect(store.query([{ kinds: [10032] }]).map((e) => e.pubkey)).toEqual([
      AUTHOR,
    ]);
  });

  it('applies the expiry filter before a filter limit', () => {
    const store = create();
    // Three expired announces newer than the one live announce: a
    // post-filtering implementation would spend the whole limit on events it
    // then discards and return nothing.
    for (let i = 0; i < 3; i++) {
      store.store(makeAnnounce(STRANGER, NOW - 10 + i, NOW - 5));
    }
    const live = makeAnnounce(AUTHOR, NOW - 100, NOW + 600);
    store.store(live);

    expect(
      store.query([{ kinds: [10032], limit: 1 }]).map((e) => e.id)
    ).toEqual([live.id]);
  });

  // The kill switch. Enforcement is a serve-time decision, so turning it off
  // makes every still-stored event readable again with no data recovery.
  it('serves expired events again when enforcement is off', () => {
    const store = create({ enforceExpiration: false });
    const expired = makeAnnounce(AUTHOR, NOW - 700, NOW - 100);
    store.store(expired);

    expect(store.get(expired.id)?.id).toBe(expired.id);
    expect(store.query([{ kinds: [10032] }])).toHaveLength(1);
  });
});

describe.each(FACTORIES)('$name — NIP-40 reaper', ({ create }) => {
  it('deletes events past their expiration', () => {
    const store = create();
    store.store(makeAnnounce(AUTHOR, NOW - 700, NOW - 100));

    expect(store.reapExpired?.(NOW)).toBe(1);
    expect(store.query([{ kinds: [10032] }])).toEqual([]);
  });

  it('leaves unexpired events alone', () => {
    const store = create();
    store.store(makeAnnounce(AUTHOR, NOW, NOW + 600));

    expect(store.reapExpired?.(NOW)).toBe(0);
    expect(store.query([{ kinds: [10032] }])).toHaveLength(1);
  });

  it('never touches events without an expiration tag', () => {
    const store = create();
    store.store(makeEvent({ created_at: 0 }));

    expect(store.reapExpired?.(NOW)).toBe(0);
    expect(store.query([{ kinds: [1] }])).toHaveLength(1);
  });

  // The grace window is what makes flipping enforcement back off a real
  // recovery: a DELETE cannot be undone by a config change.
  it('keeps recently expired events inside the grace window', () => {
    const store = create();
    store.store(makeAnnounce(AUTHOR, NOW - 700, NOW - 100));

    expect(store.reapExpired?.(NOW, 86_400)).toBe(0);
  });

  it('deletes once an event is expired for longer than the grace', () => {
    const store = create();
    store.store(makeAnnounce(AUTHOR, NOW - 200_000, NOW - 100_000));

    expect(store.reapExpired?.(NOW, 86_400)).toBe(1);
  });

  it('an event still inside the grace window is recoverable by the kill switch', () => {
    const store = create({ enforceExpiration: false });
    const expired = makeAnnounce(AUTHOR, NOW - 700, NOW - 100);
    store.store(expired);
    store.reapExpired?.(NOW, 86_400);

    expect(store.get(expired.id)?.id).toBe(expired.id);
  });
});

describe.each(FACTORIES)('$name — NIP-09 deletion', ({ create }) => {
  function deletionRequest(
    pubkey: string,
    tags: string[][],
    createdAt = NOW
  ): NostrEvent {
    return makeEvent({ pubkey, kind: 5, created_at: createdAt, tags });
  }

  it('retracts the author’s own event', () => {
    const store = create();
    const note = makeEvent({ pubkey: AUTHOR, created_at: NOW - 100 });
    store.store(note);
    store.store(deletionRequest(AUTHOR, [['e', note.id]]));

    expect(store.get(note.id)).toBeUndefined();
    expect(store.query([{ ids: [note.id] }])).toEqual([]);
  });

  // The property that keeps a public relay from becoming a takedown surface.
  it('does NOT retract an event signed by a different key', () => {
    const store = create();
    const note = makeEvent({ pubkey: STRANGER, created_at: NOW - 100 });
    store.store(note);
    store.store(deletionRequest(AUTHOR, [['e', note.id]]));

    expect(store.get(note.id)?.id).toBe(note.id);
  });

  it('stores the deletion request itself so it can propagate', () => {
    const store = create();
    const request = deletionRequest(AUTHOR, [['e', 'f'.repeat(64)]]);
    store.store(request);

    expect(store.get(request.id)?.kind).toBe(5);
  });

  it('refuses a re-publication of a deleted event', () => {
    const store = create();
    const note = makeEvent({ pubkey: AUTHOR, created_at: NOW - 100 });
    store.store(note);
    store.store(deletionRequest(AUTHOR, [['e', note.id]]));
    store.store(note);

    expect(store.get(note.id)).toBeUndefined();
  });

  // A tombstone recorded for an unseen id must not become a way to pre-block
  // somebody else's future event: it only bites the pubkey that asked.
  it('a tombstone from one key does not block another key’s event', () => {
    const store = create();
    const targetId = 'd'.repeat(64);
    store.store(deletionRequest(AUTHOR, [['e', targetId]]));

    const strangerEvent = makeEvent({
      id: targetId,
      pubkey: STRANGER,
      created_at: NOW,
    });
    store.store(strangerEvent);

    expect(store.get(targetId)?.pubkey).toBe(STRANGER);
  });

  // An id tombstone is permanent for that id, and correctly so: an event id
  // is a hash over the author's own kind/created_at/tags/content, so "the
  // same id, published later" is not a thing that exists. The
  // created_at-ordering rule is meaningful only on the coordinate path (see
  // the next test), where one address is reused over time.
  it('keeps an id retracted even when the author re-sends it later', () => {
    const store = create();
    const targetId = 'e'.repeat(64);
    store.store(deletionRequest(AUTHOR, [['e', targetId]], NOW - 100));
    store.store(makeEvent({ id: targetId, pubkey: AUTHOR, created_at: NOW }));

    expect(store.get(targetId)).toBeUndefined();
  });

  it('a coordinate retraction does not silence the author’s later announces', () => {
    const store = create();
    store.store(
      deletionRequest(AUTHOR, [['a', `10032:${AUTHOR}:`]], NOW - 100)
    );
    const later = makeAnnounce(AUTHOR, NOW);
    store.store(later);

    expect(store.get(later.id)?.id).toBe(later.id);
  });

  // The route a live node would actually use to retract its own announce
  // without having kept the event id: an `a` coordinate. Live announces carry
  // no `d` tag, so the empty identifier is the case that matters.
  it('retracts the author’s announce by addressable coordinate', () => {
    const store = create();
    const announce = makeAnnounce(AUTHOR, NOW - 100);
    store.store(announce);
    store.store(deletionRequest(AUTHOR, [['a', `10032:${AUTHOR}:`]]));

    expect(store.get(announce.id)).toBeUndefined();
  });

  it('ignores a coordinate naming another key', () => {
    const store = create();
    const announce = makeAnnounce(STRANGER, NOW - 100);
    store.store(announce);
    store.store(deletionRequest(AUTHOR, [['a', `10032:${STRANGER}:`]]));

    expect(store.get(announce.id)?.id).toBe(announce.id);
  });

  it('lets the author publish a fresh announce after retracting the old one', () => {
    const store = create();
    store.store(makeAnnounce(AUTHOR, NOW - 100));
    store.store(deletionRequest(AUTHOR, [['a', `10032:${AUTHOR}:`]], NOW - 50));
    const fresh = makeAnnounce(AUTHOR, NOW);
    store.store(fresh);

    expect(store.get(fresh.id)?.id).toBe(fresh.id);
  });

  it('is a no-op when the request names nothing', () => {
    const store = create();
    const note = makeEvent({ pubkey: AUTHOR });
    store.store(note);
    store.store(deletionRequest(AUTHOR, []));

    expect(store.get(note.id)?.id).toBe(note.id);
  });
});

describe.each(FACTORIES)('$name — operator blocklist', ({ create }) => {
  it('refuses to store a blocked event', () => {
    const blocked = makeEvent();
    const store = create({ blockedEventIds: [blocked.id] });
    store.store(blocked);

    expect(store.get(blocked.id)).toBeUndefined();
    expect(store.query([{ ids: [blocked.id] }])).toEqual([]);
  });

  it('leaves every other event alone', () => {
    const blocked = makeEvent();
    const kept = makeEvent();
    const store = create({ blockedEventIds: [blocked.id] });
    store.store(blocked);
    store.store(kept);

    expect(store.query([{ kinds: [1] }]).map((e) => e.id)).toEqual([kept.id]);
  });

  // The scope guarantee: the list matches event IDS. Blocking a pubkey would
  // silence an identity's whole output, which is exactly the power this
  // mechanism is drawn to withhold.
  it('does not block by pubkey even when the value looks like one', () => {
    const store = create({ blockedEventIds: [AUTHOR] });
    const event = makeEvent({ pubkey: AUTHOR });
    store.store(event);

    expect(store.get(event.id)?.pubkey).toBe(AUTHOR);
  });
});

describe('SqliteEventStore — blocklist sweeps events already on disk', () => {
  it('purges a blocked event that predates the configuration', () => {
    const litter = makeEvent();
    const keeper = makeEvent();

    const before = new SqliteEventStore(SQLITE_DB_PATH);
    before.store(litter);
    before.store(keeper);
    before.close();

    const after = new SqliteEventStore(SQLITE_DB_PATH, {
      blockedEventIds: [litter.id],
    });
    try {
      expect(after.get(litter.id)).toBeUndefined();
      expect(after.get(keeper.id)?.id).toBe(keeper.id);
    } finally {
      after.close();
    }
  });
});

describe('SqliteEventStore — migration of a pre-NIP-40 database', () => {
  it('adds expires_at and backfills it from stored tags', () => {
    const expired = makeAnnounce(AUTHOR, NOW - 700, NOW - 100);
    const live = makeAnnounce(AUTHOR, NOW, NOW + 600);
    const plain = makeEvent();

    // Build the old schema by hand: exactly the table shape a running devnet
    // relay has on disk today, with no expires_at column.
    const db = new Database(SQLITE_DB_PATH);
    db.exec(`
      CREATE TABLE events (
        id TEXT PRIMARY KEY, pubkey TEXT NOT NULL, kind INTEGER NOT NULL,
        content TEXT NOT NULL, tags TEXT NOT NULL, created_at INTEGER NOT NULL,
        sig TEXT NOT NULL, received_at INTEGER NOT NULL
      )
    `);
    const insert = db.prepare(
      'INSERT INTO events (id, pubkey, kind, content, tags, created_at, sig, received_at) VALUES (?,?,?,?,?,?,?,?)'
    );
    for (const event of [expired, live, plain]) {
      insert.run(
        event.id,
        event.pubkey,
        event.kind,
        event.content,
        JSON.stringify(event.tags),
        event.created_at,
        event.sig,
        event.created_at
      );
    }
    db.close();

    const store = new SqliteEventStore(SQLITE_DB_PATH);
    try {
      // The pre-existing expired announce stops being served on first boot
      // after the upgrade -- no republish required.
      expect(store.get(expired.id)).toBeUndefined();
      expect(store.get(live.id)?.id).toBe(live.id);
      expect(store.get(plain.id)?.id).toBe(plain.id);
    } finally {
      store.close();
    }
  });
});
