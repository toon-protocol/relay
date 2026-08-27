---
'@toon-protocol/relay': minor
---

Record the payment the connector states on a delivery, and make `deploy/` the deployment of record.

`POST /write` reads `X-TOON-Payer` / `X-TOON-Amount` / `X-TOON-Chain` again (connector ADR 0040, relay#133). #128 removed them on the strength of ADR 0036's "no successor header is coming"; one came, and it is a different value: the payer is now the chain-verified client channel key whose claim the terminating connector checked at its own edge, not the previous hop. A well-formed triple is echoed on the 200 as `payment` and carried on the write's log line; a partial or malformed one is discarded whole rather than half-recorded, and **absence is never read as "unpaid"** — the headers are stated only by the hop that took the payment, so they are absent on a forwarded packet and on every free route.

`deploy/` now runs the connector this contract comes from (`rust-sha-6ea6009`, up from `rust-sha-440eab7`) and carries the whole node rather than half of it: Caddy terminates TLS for both public hostnames, a Watchtower overlay follows the two `:release` tags, and a local overlay runs the stack without TLS. The connector config drops `[announce]` — the current connector refuses that section by name and serves the same facts on `GET /ilp` from a `[node]` table instead.

Also: `packages/bls` is removed (nothing depended on it, no CI built its images, and it was the only ILP-speaking code in a repo that documents itself as speaking none), the unused TOON codec re-export is gone, `DEFAULT_RELAY_CONFIG.port` now matches the launcher's actual default of 7100, and `GET /health` reports the version the package actually shipped instead of a hardcoded `0.1.0`.
