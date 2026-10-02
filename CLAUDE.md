# relay

The TOON Protocol **Nostr relay node**, in Rust (`crates/relay`, the `relay` binary and
image) — a NIP-01 WebSocket read surface, an HTTP `POST /write` surface and a
NIP-11 relay information document on the read port.

Part of the **TOON Protocol** — pay-to-write Nostr over Interledger (ILP),
split into per-team repos. The relay is a plain HTTP/WebSocket app: it speaks
**no** ILP and contains no connector, settlement, or pricing logic. Payment is
enforced upstream by the connector; a request reaching `POST /write` is already
proven paid, so the relay verifies the event signature, stores it, and serves
free reads. It records the payment the connector states on the delivery
(`X-TOON-Payer` / `-Amount` / `-Chain`, connector ADR 0040) without
re-validating it — that statement is the whole trust model.

That rule is also why the relay does not WRITE DOWN where its writes are paid
for. Its NIP-11 document names an ILP address, a connector URL, a sealing key,
a carriage and a price, and every one of those is read from the connector's own
free `GET /ilp` at runtime (`crates/relay/src/edge.rs`), never held here. The
relay is told exactly one thing it cannot read — which of that connector's
routes reaches its own `POST /write` — and refuses to advertise an address the
connector does not terminate. See TOON_Network#121 and its ADR 0024.

## Build & test

```
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm install
pnpm -r build
pnpm -r test
```

The relay is Rust (rebuilt as a drop-in for the original TypeScript relay, spec
#185; cut over in #205; the TypeScript source, changesets and npm publishing
were removed in #206). The pnpm workspace that remains holds only the
conformance suite, the soak tooling (`packages/conformance/`) and the guards in
`deploy/*.test.ts`; nothing is published to npm.

## The Rust relay

The relay is a Cargo workspace: `Cargo.toml`, `rust-toolchain.toml` (the one
toolchain pin; rustup installs it) and `crates/relay`, with its image in
`crates/relay/Dockerfile`; its commands are the `cargo` lines under Build & test.

Rust code follows `docs/rust-coding-standards.md` (the connector's standards,
plus unsafe forbidden, the pinned toolchain, invariants as types with
compile-fail tests, and NIP-01 handling that is the relay's own, on the `nostr`
protocol crate alone).

What decides whether a Rust change is correct is the **conformance suite**
(`packages/conformance/`): it starts an image and talks only to its ports, and
CI runs it against the image. A case the Rust relay does not pass yet is
marked `expectedFailureFor: ['rust']` and goes red once it passes, so a slice
that builds a surface removes that surface's markers in the same change.

## Deployment

`deploy/` is **the deployment of record** — the live devnet relay box runs it
from this repo (Caddy → connector → relay). One image is published to GHCR on
every green merge to `main`, with a moving `:release` tag a box follows and
immutable `:rust-<handle>` and `:rust-sha-*` tags:

- `ghcr.io/toon-protocol/relay` — the app (`crates/relay/Dockerfile`)

The rollback to the last TypeScript build (an image that is no longer built from
anything here) is in `deploy/README.md`.

The connector is the **stock** `ghcr.io/toon-protocol/connector` image on an
immutable pin, with `deploy/connector.toml` mounted read-only — the same shape
the store and gas-station bundles use. This repo publishes no connector image.
The pin lives in exactly one place: `deploy/docker-compose.yml`'s `connector`
service `image:`. Because it is immutable, the connector does **not**
auto-deploy: changing it or `connector.toml` needs `git pull && docker compose
up -d` on the box. `deploy/bundle.test.ts` fails the build if a second copy of
the pin appears, if the privacy invariant breaks, or if prices/settlement
drift.

## Cross-repo dependencies

The ILP payment engine is the separate
**[toon-protocol/connector](https://github.com/toon-protocol/connector)** repo
(GHCR image + config reference). **All payment-claim validation lives ONLY in
the connector — never re-implement it here.**

## Shared skills, docs & project context

Cross-cutting agent skills, docs and project context once lived in
toon-protocol/toon-meta, which is being retired. Nothing this repo needs to
build, test or ship depends on it; CI, including the no-op merge guard
(`.github/scripts/no-op-merge-guard.sh`), is self-contained.

## Releasing

Merging to `main` publishes the Rust image through `publish-relay-image.yml`
once the conformance suite passes against it; that is the whole release scheme.
There are no changesets and no npm package. **Never run `npm publish`.**

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues (`toon-protocol/relay`, via the `gh` CLI).
See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage labels, names unchanged. `ready-for-agent` is the AFK factory's queue.
See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: this file and `README.md`, with decisions in the connector repo's `docs/adr/`.
See `docs/agents/domain.md`.
