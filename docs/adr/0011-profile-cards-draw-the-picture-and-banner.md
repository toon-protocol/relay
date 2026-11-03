# Profile cards draw the picture and banner, fetched under bounds

**Status:** Accepted (2026-11-03). Supersedes [ADR 0010](0010-a-profiles-preview-is-its-own-picture.md). Makes one exception to [ADR 0001](0001-app-uis-live-on-the-relay.md)'s never-fetch rule and to [ADR 0006](0006-the-relay-draws-link-preview-cards.md)'s no-picture rule, for profile cards only. Amends rule 5 of [ADR 0008](0008-avatars-are-pictures-over-identicons.md).

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** any of these.
- The relay fetches anything other than a profile card's `picture` or `banner`, or fetches either for a page rather than for `/social/og/p/<npub>.png`.
- A fetch that is not `https` on port 443, or that goes to a host resolving to any loopback, private, link-local, shared, reserved, documentation or multicast address.
- A fetch that follows more than three redirects, runs past 5 s, reads more than 5 MiB, or runs while four others are already in flight.
- A card that draws an image other than a PNG, JPEG, GIF or WebP no larger than 4096 pixels a side, or draws one from anything but a `data:` URL of bytes fetched this way.
- A card that fails to draw because a picture could not be had.

**A profile card (`/social/og/p/<npub>.png`) draws the `https` `picture` and `banner` from the key's newest kind 0. The picture fills the round avatar and the banner spans the top. The name, NIP-05 claim, about, repo count and follower count stay as before, and the profile page names this card again.** ADR 0010's alternative showed the bare picture instead of the card: X drew it as a small square, without the counts and without TOON's layout. A card can only hold a picture the relay has the bytes for, so the relay fetches them.

## Decision

1. **One fetcher, one use.** `ui_preview::Pictures` is the only outbound HTTP client in the relay. Only the profile card route calls it. Pages never do, and nothing else ever names it a URL.
2. **The URL is guarded before any request.**
   - The URL must be `https`, on port 443, with no user info and no IPv6 literal.
   - The host is resolved first, and every address it resolves to must be public. If any address is not, nothing is sent.
   - The connection is made to one of those checked addresses, so a second DNS answer cannot redirect it.
   - TLS is verified against the Mozilla roots compiled into the image.
   - A redirect is followed only to another `https` URL, and is checked the same way. At most three are followed.
3. **The request is bounded.** It gets 5 s in all and 5 MiB of body, and four fetches may run at once. The relay sends no cookies or credentials, and its `User-Agent` names it.
4. **Bytes are checked before they are decoded.** The format is read from the magic number, and the size from the header: PNG, JPEG, GIF or WebP, up to 4096 × 4096. SVG and anything else are discarded. The renderer resolves an `<image>` only from a raster `data:` URL, never from a file, a URL or a nested SVG.
5. **Answers are cached.** A picture is kept for an hour and a failure for ten minutes, in a cache of at most 512 entries and 64 MiB. A card's ETag covers the digest of each picture it draws, so a changed picture is a new card.
6. **The card's URL changes with the card.** The page's `og:image` is `/social/og/p/<npub>.png?v=<version>`, the version a hash of what the card is drawn from, its pictures named by URL, so the page fetches nothing to name it. Crawlers keep an image by its URL (Discord's media proxy does, and re-posting the page does not refresh it), so an unchanging URL would leave a stale card in an unfurl. The query is not read when the card is served.
7. **A missing picture leaves the identicon.** A failed, slow or refused fetch draws the card without it. The card still draws, and its counts are unaffected.

## Consequences

- When a crawler fetches a profile's card, the image host the person chose learns the relay's IP address. Visitors to the page are not involved.
- A kind 0 author can make the relay send one GET, at most every ten minutes per URL, to a public `https` host of their choosing. The response is only kept if it is an image. This is the SSRF surface the guards in rule 2 bound: no private address is reachable, and nothing in the request or the reply is passed back anywhere but into pixels.
- The relay image now carries a TLS client (rustls with ring) and webpki roots.
- Repo, post and article cards still draw no picture, and `/repo` still loads none.
