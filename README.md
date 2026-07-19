# relay

TOON Protocol Nostr relay node — @toon-protocol/relay (NIP-01 relay + BLS + startRelay launcher) and @toon-protocol/bls

In the TOON stack this is the Nostr relay (built on @toon-protocol/core) that towns run: reads are free over plain NIP-01, while paid writes reach it through the connector payment proxy sitting in front. The live devnet relay is `wss://relay-ws.devnet.toonprotocol.dev`. To use the network as a client (rather than run a relay), start with the toon-client [rig README](https://github.com/toon-protocol/toon-client/blob/main/packages/rig/README.md).

> Extracted from the TOON monorepo with full git history preserved. npm publishing is done by CI (changesets + `pnpm`, authed by the org `NPM_TOKEN` secret). Docker image published to ghcr.io/toon-protocol/relay:latest by publish-relay-image.yml.

### Getting started with Devbox

[Devbox](https://github.com/jetify-com/devbox) pins the local toolchain to the exact
versions CI uses — Node `22` and pnpm `8.15.9` — so `pnpm build`,
`pnpm test`, and `pnpm lint` run in a reproducible shell without touching your system
packages.

**Prerequisites:** [Install devbox](https://www.jetify.com/devbox/docs/installing_devbox/) (one-liner).

```bash
# Enter the pinned shell (downloads packages on first run via Nix)
devbox shell

# Inside the devbox shell, all tools are on PATH:
node --version    # v22.x
pnpm --version    # 8.15.9

# Run the standard targets (defined as devbox scripts)
devbox run build  # pnpm install --no-frozen-lockfile && pnpm build
devbox run lint
devbox run test
```

`.devbox/` (the Nix symlink/cache dir) is gitignored; `devbox.json` and `devbox.lock`
are committed.
