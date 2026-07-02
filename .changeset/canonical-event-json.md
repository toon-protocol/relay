---
'@toon-protocol/relay': patch
---

Serve canonical NIP-01 JSON on outbound EVENT frames (#46). The WebSocket read surface previously encoded the event as a TOON-text string inside the frame (`["EVENT", subId, "<toon text>"]`), which standard nostr clients could neither parse nor signature-verify from the wire. Both the stored-query (REQ) and live-subscription (broadcast) paths now emit `["EVENT", subId, {id, pubkey, created_at, kind, tags, content, sig}]` with the event as a plain JSON object, byte-compatible with vanilla NIP-01 libraries. The TOON codec remains exported for library consumers but is no longer used on the wire.
