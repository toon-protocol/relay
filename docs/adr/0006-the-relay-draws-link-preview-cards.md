# The relay draws link preview cards

**Status:** Accepted (owner decision, 2026-10-23, #314). Built by `crates/ui-preview` and the `/social` and `/repo` app UIs.

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a preview image drawn from anything but this relay's store (an outbound request, a remote image, a system font); an `<image>` element, a link or script in a card's SVG; a card route that needs authentication or loosens a page's `Content-Security-Policy`; a card cache with no bound on entries or bytes; more than a fixed number of cards drawn at once; or a card that calls a NIP-05 claim verified.

**The relay draws a 1200×630 PNG card for a repo, a profile, a ship post, an article and each app, from its own events, and every app UI page names one in Open Graph and X card tags.** It still fetches nothing (ADR 0001) and still never shows a picture a person chose for themselves (ADR 0005, rule 4).

## Context

The point of `/social` is builders sharing repos on X. A link to a `/social` or `/repo` page posted on X, Discord, Slack or Bluesky unfurled into a bare URL: the page shell had no Open Graph or X card tags, and there was no image to name.

An unfurler wants an absolute `https` image URL that answers a PNG or JPEG, usually 1200×630. The relay could name an image an event already links to (an `imeta` screenshot), but most repos and profiles have none, ADR 0005 forbids drawing a profile's `picture`, and an image hosted elsewhere is one the relay cannot vouch for. So the relay draws the card itself, GitHub-style, from text it already shows on the page.

Drawing an image on request is new work for a relay: it costs CPU and memory per request, a crawler can ask for every repo's card, and an image renderer that could load a URL or a system font would be an outbound fetch by another name.

## Decision

1. **Text and plain shapes from the store.** A card's content is read from the same events, by the same rules, as the page that names it: owner and name, description, up to four topics and the stars, followers and forks the repo page counts; a profile's name, NIP-05 claim, about, repos kept and followers, and a `bot` chip; a ship post's headline over its repo; an article's title, author and summary. An avatar is initials. No card holds a picture.
2. **SVG built in Rust, rasterised by resvg.** `ui-preview` lays the text out (wrapping and cutting with an ellipsis against the font's glyph widths) and writes SVG of `<text>`, `<rect>`, `<circle>` and the pages' own Octicon paths, every string escaped. `resvg` (pure Rust, with `usvg` and `tiny-skia`; no `unsafe` in this workspace) draws it with `default-features = false, features = ["text"]`: no system fonts, no memory-mapped fonts, no raster image decoders. The one font is Inter 4.1 regular and bold, compiled in, under the SIL Open Font License 1.1 (`crates/ui-preview/fonts/OFL.txt`). A character Inter has no glyph for is left out.
3. **Bounded cost.** Every text is cut to 400 characters before it is measured. A card is drawn on a blocking thread, at most two at once; a request past that is `503` with `Retry-After`, not queued. A drawn card is kept in a cache of at most 512 cards and 32 MiB, least recently used out first. Its `ETag` is a hash of what it shows (so it changes when a count does), a request holding it gets `304` without a draw, and `Cache-Control: public, max-age=600` lets a crawler keep it. A card takes about 20 ms to draw and is 40–70 KB.
4. **Routes under each app, served like a page.** `/social/og/r/<npub>/<d>.png`, `/social/og/p/<npub>.png`, `/social/og/e/<id>.png` (ship posts only), `/social/og/a/<npub>/<d>.png` and `/social/og/site.png`; `/repo/og/<npub>/<d>.png` and `/repo/og/site.png`. A `/repo` page names `/social`'s repo card while `/social` is on, so both apps share one image counted one way. Anything the store does not hold is `404`. The routes answer `GET`/`HEAD` with `image/png`, need no authentication, and carry the default app UI policy; no page's policy changes.
5. **Absolute URLs need a stated origin.** `TOON_UI_PUBLIC_URL` is the `https` origin the app UIs are reached at; unset, the origin of a `wss://` or `https://` `TOON_UI_RELAY_URL` stands in. With neither, a page states its title and description and names no image and no `og:url`. The relay never contacts the origin.

## Consequences

- A shared repo, profile, ship post or article link unfurls into a card on X (`summary_large_image`), Discord, Slack, Bluesky and anything else that reads Open Graph. Pages without a card of their own name their app's site card.
- The binary carries a renderer, a text shaper and a font. Measured against the published image of the commit before this one (`rust-sha-7f52c76`, linux/amd64): the static `relay` binary grew from 13.4 MB to 18.6 MB (+5.2 MB, of which the font is 0.8 MB), and the compressed image from 10.8 MB to 12.9 MB (+2.1 MB).
- A crawler costs the relay the store queries the page itself would cost, plus at most two concurrent draws; a flood of distinct cards is answered `503` past that, and the cache's memory is fixed.
- A name in a script Inter does not cover (CJK, for one) is drawn without those characters, and a name made only of them is left blank on the card (the page's own `og:title` still carries it). Adding fonts is a size trade-off, not a new decision.
- Drawing a person's `picture` or a post's media into a card, fetching anything to draw one, or drawing cards for other kinds is a new decision and needs its own record.
