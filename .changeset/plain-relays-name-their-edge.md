---
'@toon-protocol/relay': minor
---

A relay says where its writes are paid for.

`GET` a relay's read port with `Accept: application/nostr+json` and it answers
a NIP-11 relay information document whose `toon` object names its paid write
edge — `ilp_address`, `connector_url`, `connector_seal_key` and the carriage
that route pins — the same three facts a Provider Profile pins about a
provider (TOON_Network §4.5, ADR 0011 and ADR 0024). A client holding only a
relay's URL can now buy a write to it instead of being configured out of band.
The refusal a WebSocket write gets back names the same edge, so a client can
recover from the refusal alone.

None of it is written down in the relay. The edge is read from the connector's
own free `GET /ilp` and re-read in the background, and the NIP-11 `limitation`
numbers come from the very config object the connection handler enforces them
from, so the advertisement cannot drift from the enforcement. Two new settings
say where to ask and which route to ask about (`TOON_CONNECTOR_URL`,
`TOON_WRITE_ILP_ADDRESS`); set neither and the relay publishes no edge and
behaves exactly as before. A relay whose route is priced `0` still works and
says so.

Every other plain HTTP request to the read port still answers `426 Upgrade
Required`, and the WebSocket handshake is unchanged.
