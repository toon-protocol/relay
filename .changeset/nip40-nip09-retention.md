---
'@toon-protocol/relay': minor
---

Enforce NIP-40 expiration, implement NIP-09 deletion, and add an operator event blocklist.

Until now the relay had no way to stop serving an event. Expiry tags were parsed by nothing, kind:5 was stored as an ordinary note, and the only retraction path was a newer event signed by the same key — so a node whose key was lost advertised itself forever.

- **NIP-40**: events past their `expiration` tag are no longer served from history or fanned out live, and a background reaper deletes them after a grace period. On by default; `TOON_ENFORCE_EXPIRATION=false` is the kill switch back to the old behaviour.
- **NIP-09**: a kind:5 request retracts the author's own events by `e` id or `a` coordinate, and a tombstone stops a re-publish from resurrecting them. A request naming another key's event is a no-op.
- **Operator blocklist** (`TOON_BLOCKED_EVENT_IDS`): startup-configured event ids this relay refuses to store or serve — the escape hatch for litter whose author key is gone, so neither NIP can reach it. Ids only, never pubkeys; logged loudly on every boot.

Existing SQLite databases are migrated in place (an `expires_at` column, backfilled from stored tags), so a pre-existing expired event stops being served on the first boot after upgrading with no republish required.
