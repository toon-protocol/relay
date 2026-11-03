# /repo draws a repository's Markdown as GitHub does

**Status:** Accepted (2026-11-03). Supersedes rule 6 of [ADR 0005](0005-social-pages-load-https-media.md) for README and Markdown file images on `/repo` git pages; profile pictures stay off `/repo` as [ADR 0008](0008-avatars-are-pictures-over-identicons.md) says.

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/repo` page that keeps a raw HTML tag outside `ui_markdown`'s list, or any attribute but `href`, `src` and the presentational ones (`align`, `width`, `height`, `alt`, `title`, `open`, `colspan`, `rowspan`); an `<img>` on a `/repo` page whose `src` is neither `https` nor a `blob:` URL over bytes that hashed to the object the tree names; an issue, comment or `/social` post whose raw HTML becomes markup; or the relay making an outbound request to draw any of it.

**A README or Markdown file on `/repo` is drawn the way GitHub draws it: the raw HTML tags GitHub's sanitizer keeps are kept with their presentational attributes, images in the repo are drawn from its own verified objects, and `https` images are loaded by the visitor's browser.** Before this, raw HTML was shown as text and every image as a link, so a README's centred header, logo and badges read as a column of words and the same repo looked unlike itself on GitHub.

## Decision

1. **Two readings.** `ui_markdown::parse` stays as it was, for text that comes from events: issues, comments, `/social` posts and articles. `ui_markdown::parse_rich` reads a file in a repository. Only `/repo`'s browser half calls it, for READMEs and Markdown file pages.
2. **A fixed list of tags.** `parse_rich` keeps the tags in `ui_markdown::TAGS` (`div`, `p`, `img`, `picture`, `details`, `summary`, `sub`, `sup`, `kbd`, the heading, list, table and inline tags). Any other tag is dropped and its text kept, as `parse` does. Attributes are kept only when their names are in the list above and their values are plain: `align` is one of four words, and sizes are numbers. `href` goes through the same link policy Markdown links use. No `style`, `class`, `id` or event handler survives. A tag left open is closed with the Markdown block it opened in.
3. **Repo images come from the repo.** An image whose `src` is a relative path is looked up in the tree at the page's commit. Its blob is fetched and verified like every other object (ADR 0001) and drawn from a `blob:` URL typed by its extension. One that is missing, or of an unknown type, shows its alt text.
4. **Other images are `https` only.** The visitor's browser loads an `https` `src` itself, with no referrer. `http:`, `data:` and any other scheme are never drawn. The relay still fetches nothing.
5. **The policy.** Git pages send `img-src 'self' https: blob:` beside their existing `connect-src 'self' https:`. Script stays `'self'` and WebAssembly only, so an image or tag cannot become script.

## Consequences

- Opening a repo page whose README names `https` images (shields.io badges, for one) tells those hosts the visitor's IP address, as viewing it on GitHub would. No referrer is sent.
- A README now looks the same on `/repo` as on GitHub, apart from what GitHub draws that this list does not: `<source>` in `<picture>` (the fallback `<img>` is drawn), inline `style`, and math.
- A repo image is only shown if a Blossom or Arweave URL for its object is listed in the page's layout.
