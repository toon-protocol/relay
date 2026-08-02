---
'@toon-protocol/relay': patch
---

Lift the paid-write pipeline's ~150 events/s global admission ceiling (connector#685): ephemeral events (NIP-16, kinds 20000-29999) are now broadcast-only and never hit the disk, the SQLite event store runs in WAL mode with synchronous=NORMAL instead of the default rollback journal with two fsyncs per insert, and the regular-event INSERT statement is prepared once instead of on every write. The synchronous per-event disk write on the single Node event loop was the serialization point that capped aggregate paid-write throughput at ~150 frames/s across all BTP sessions while CPU stayed idle.
