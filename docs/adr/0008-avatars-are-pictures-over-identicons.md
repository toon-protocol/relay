# Avatars are pictures over identicons

**Status:** Accepted (2026-11-02). Supersedes rule 4 of [ADR 0005](0005-social-pages-load-https-media.md). Amended for profile previews by [ADR 0010](0010-a-profiles-preview-is-its-own-picture.md).

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/repo` page that loads a profile `picture`; a `/social/avatar/<npub>` response that redirects to anything but an `https` URL the key's newest kind 0 names; or the relay making an outbound request to answer it.

**On `/social`, a person's avatar is their kind 0 `picture`, drawn over an identicon made from their npub. On `/repo`, it is the identicon alone.** Two letters in a circle made people look alike and left a profile's own image unused, which is what a visitor expects a social page to show.

## Decision

1. **Every key has an identicon.** `chrome::identicon` draws eight by eight mirrored pixels from the SHA-256 of the npub, as inline SVG. It needs no profile and no request, and a key looks the same on every page.
2. **`/social` lays the picture over it.** The image is `/social/avatar/<npub>`, which reads the key's newest kind 0 and answers `302` to its `https` `picture`, or `404`. The image has an empty `alt`, so a picture that fails draws nothing and the identicon shows.
3. **ADR 0005's protections stay.** The relay still fetches nothing, only `https` URLs are followed, and no referrer is sent. Images load lazily.
4. **`/repo` keeps its stricter policy**, as ADR 0005 rule 6 says, so it draws no pictures.
5. **Link preview cards draw the identicon.** Under ADR 0006 they hold no picture, so TOON's or anyone's `picture` is not on a card, but the identicon is plain shapes from the key and needs no fetch. Git commit authors on `/repo` have a name but no key, so they keep initials too.

## Consequences

- Reading a `/social` page now tells the hosts its authors chose the visitor's IP address, which ADR 0005 rule 4 existed to prevent. The visitor's own Nostr client would do the same.
- Each avatar without a cached answer costs one request and one store query. The answer is cached for five minutes, so a changed picture can take that long to appear.
- The same person shows their picture on `/social` and their identicon on `/repo`.
