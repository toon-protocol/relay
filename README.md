# relay

TOON Protocol Nostr relay node — @toon-protocol/relay (NIP-01 relay + BLS + startRelay launcher) and @toon-protocol/bls

> Extracted from the TOON monorepo with full git history preserved. npm publishing is done by CI (changesets + `pnpm`, authed by the org `NPM_TOKEN` secret). Docker image-publish workflows (where applicable) are a follow-up carved from the monorepo `publish-townhouse-images.yml`.

## Getting started with Devbox

[Devbox](https://www.jetify.com/devbox) pins Node + pnpm to the exact versions used by CI, giving every contributor and every CI run an identical toolchain.

**Prerequisites:** [install Devbox](https://www.jetify.com/devbox/docs/installing_devbox/) (`curl -fsSL https://get.jetify.com/devbox | bash`).

```sh
# Enter the reproducible shell (installs Node 20 + pnpm 8.15.0 automatically)
devbox shell

# Then install dependencies and run tests as normal
pnpm install
pnpm -r build
pnpm -r test
```

Without Devbox the repo still works with any Node ≥ 20 + pnpm 8.15.0 installed globally.
