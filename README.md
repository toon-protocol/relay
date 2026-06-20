# relay

TOON Protocol Nostr relay node — @toon-protocol/relay (NIP-01 relay + BLS + startRelay launcher) and @toon-protocol/bls

> Extracted from the TOON monorepo with full git history preserved. npm publishing is done by CI (changesets + `pnpm`, authed by the org `NPM_TOKEN` secret). Docker image-publish workflows (where applicable) are a follow-up carved from the monorepo `publish-townhouse-images.yml`.

## Getting started with Devbox

[Devbox](https://github.com/jetify-com/devbox) provides a reproducible shell with Node 22 and pnpm 8.15.x pinned.

```sh
# Install devbox (one-time)
curl -fsSL https://get.jetify.com/devbox | bash

# Enter the dev shell
devbox shell

# Then inside the shell:
pnpm install
pnpm -r build
pnpm -r test
```
