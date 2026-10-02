-- The schema the TypeScript relay creates in a new events.db: every statement
-- SQLite recorded in sqlite_master, in the order it was created. It stands in
-- for a database an operator already has, so the Rust store can be tested
-- against one (#193).
--
-- Not edited by hand: it was captured from the TypeScript relay, removed in
-- #206, and is frozen as the schema of the databases that relay left behind.
CREATE TABLE events (
  id TEXT PRIMARY KEY,
  pubkey TEXT NOT NULL,
  kind INTEGER NOT NULL,
  content TEXT NOT NULL,
  tags TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  sig TEXT NOT NULL,
  received_at INTEGER NOT NULL,
  expires_at INTEGER
);
CREATE TABLE deleted_events (
  event_id TEXT PRIMARY KEY,
  pubkey TEXT NOT NULL,
  deleted_at INTEGER NOT NULL
);
CREATE TABLE deleted_addresses (
  coordinate TEXT PRIMARY KEY,
  deleted_at INTEGER NOT NULL
);
CREATE INDEX idx_events_pubkey ON events(pubkey);
CREATE INDEX idx_events_kind ON events(kind);
CREATE INDEX idx_events_created_at ON events(created_at);
CREATE INDEX idx_events_pubkey_kind ON events(pubkey, kind);
CREATE INDEX idx_events_expires_at ON events(expires_at) WHERE expires_at IS NOT NULL;
