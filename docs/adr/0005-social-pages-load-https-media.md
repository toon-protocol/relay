# /social pages let the browser load https media

**Status:** Accepted (2026-10-23, #307). Built by the `/social` app UI (`crates/ui-social`).

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/social` page whose `Content-Security-Policy` allows script, or allows images or media from anything but `'self'` and `https:`; a `/social` page served without `Referrer-Policy: no-referrer`; an `<img>` or `<video>` on a `/social` page whose URL is not `https`; an avatar drawn from a profile's `picture`; or the relay making an outbound request to produce or check any of it.

**`/social` pages allow the visitor's browser to load `https` images and video that events name. The relay still fetches nothing.** This loosens ADR 0001's "strict CSP for pages built only from events" for one app. It does not touch ADR 0001's rule that the relay never fetches.

## Context

ADR 0001 says pages built only from events stay server-rendered with a strict CSP. `/repo`'s event pages send `default-src 'self'` (plus `'self'` WebAssembly), and its git pages add `connect-src 'self' https:` so the browser can fetch git objects it verifies by hash.

`/social` is for building in public, and media carries much of what people show there. A ship post's hero is a screenshot or a demo video (NIP-92 `imeta`). NIP-68 and NIP-71 events are pictures and videos. A NIP-82 app listing has an icon and screenshots. Under `img-src 'self'` none of these could be drawn, and the relay cannot serve them itself without fetching them, which ADR 0001 forbids: that would make the relay an open proxy and expose it to SSRF from event content.

Loading third-party media in the visitor's browser has costs. The media host learns the visitor's IP address and can see that the image was loaded. A `Referer` header would also tell it which page asked. And any script on the page would turn that open media list into an injection risk.

## Decision

1. **The policy.** Every `/social` response is sent with
   `Content-Security-Policy: default-src 'self'; script-src 'none'; img-src 'self' https:; media-src 'self' https:; form-action 'self'; base-uri 'none'`
   (`ui_social::CONTENT_SECURITY_POLICY_VALUE`). The pages run no script at all.
2. **No referrer.** Every `/social` response carries `Referrer-Policy: no-referrer`, and each drawn `<img>` also has `referrerpolicy="no-referrer"`. A media host is not told which page or relay asked.
3. **Only `https` URLs named in events.** A page draws media only from an `https` URL that an event names (an `imeta` `url` or `image`, a NIP-68/71 event, a NIP-82 `icon` or `image`). `http:` and `data:` URLs are never drawn. A video is not fetched until it is played (`preload="none"`).
4. **Avatars are initials.** A person's avatar is the first two letters of their name, never their kind 0 `picture`. A page loads no image a person chose to stand for themselves, so following a by-line does not ping a host that person picked.
5. **The relay still never fetches.** It does not proxy, cache, resize or check any media, and it makes no outbound request for a `/social` page. ADR 0001 stands unchanged on this point.
6. **`/repo` keeps its stricter policy.** Its event pages stay `default-src 'self'` and its git pages add only `connect-src 'self' https:`. This decision applies to `/social` alone.

`TOON_UI_APPS` defaults to every app UI the build includes, which is now `repo,social`. A relay that sets nothing serves `/social` with this policy.

## Consequences

- A visitor's browser contacts the media hosts named in the events on a page: the visitor's IP address reaches whoever wrote a post's `imeta` URL. The visitor's own client would do the same, and no referrer goes with the request.
- A post with a broken, slow or removed URL shows a broken image, not a server error. The relay cannot repair it, as with ADR 0001's missing git objects.
- The CSP blocks script, so an image URL cannot become script injection. Escaping in the server-rendered HTML stays the only guard against markup injection.
- **The devnet box serves `/social` on its next `:release`.** The box follows `:release` and sets no `TOON_UI_APPS`, so once `/social` merges to `main`, the published image serves `/social` there by default. An operator who wants only `/repo` sets `TOON_UI_APPS=repo` in the relay's environment before pulling.
- Allowing media from a list of hosts, proxying media through the relay, or drawing kind 0 pictures is a new decision and needs its own record.
