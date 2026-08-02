---
'@toon-protocol/relay': patch
---

Lift the post-#84 write-path CPU ceiling (relay#85, part 1 of the ticket): `POST /write` now verifies BIP-340 event signatures with WASM libsecp256k1 (`tiny-secp256k1`, ~0.20ms/verify) instead of pure-JS noble (~1.3ms/verify) — about 7x faster per event — with a load-time self-test and a transparent fallback to nostr-tools' noble verify if the WASM module cannot load, so the relay never hard-fails on an unsupported platform. The active implementation is logged once at startup. Additionally, the per-write `[write] event=...` console line is now off by default and opt-in via `--log-writes` / `TOON_LOG_WRITES=true`: per-event console I/O through docker's json-file log driver was residual write-path disk I/O and event-loop tail jitter that #84 did not remove (connector#685 Phase G).
