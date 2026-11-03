# A profile's link preview is its own picture

**Status:** Accepted (2026-11-03). Amends [ADR 0006](0006-the-relay-draws-link-preview-cards.md) for `/social` profile pages and rule 5 of [ADR 0008](0008-avatars-are-pictures-over-identicons.md).

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/social` profile page whose newest kind 0 names an `https` `picture` but whose `og:image` is anything else; an `og:image` that is not `https`; or the relay making an outbound request to fetch, check or draw the picture.

**A profile page whose newest kind 0 names an `https` `picture` gives that URL as its `og:image` and `twitter:image`, with X's `summary` card. A profile without one keeps its drawn identicon card.** A shared profile link showed only an identicon, though the person had chosen a picture to stand for them, which is what a reader of the link expects to see.

## Decision

1. **The unfurler fetches the picture, not the relay.** The page names the URL from the kind 0 as it is. The relay reads only its own store, as ADR 0001 requires, and draws nothing new.
2. **Only `https` URLs are named**, by the same check as `/social/avatar/<npub>`. A `picture` that is not `https` leaves the drawn card in place.
3. **A small square.** The picture's size and type are not known without fetching it, so no `og:image:width`, `og:image:height` or `og:image:type` is given, and the X card is `summary` rather than `summary_large_image`.
4. **Cards still draw no picture.** `/social/og/p/<npub>.png` still draws the identicon (ADR 0008 rule 5). It is still served, and still named by a profile without a picture.

## Consequences

- When a profile link is shared, the platform (X, Slack, Discord, iMessage) fetches the picture from the host the person chose. Visitors are not involved, and the relay is not contacted for it.
- A profile preview shows the picture and the page title and description, without the TOON card layout. A broken or removed picture gives a preview with no image. The relay cannot repair that.
- Repo, post and article previews are unchanged.
