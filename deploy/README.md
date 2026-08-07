# relay deploy — the relay behind the TOON connector (payment proxy)

The production-faithful deployment of this relay: the **connector (payment proxy,
"nginx for payments")** runs in front of the **oblivious Nostr relay**. The
connector **monetizes WRITES**; **READS are free** and hit the relay's WS
directly. Settlement runs against the **shared live devnet**. **TLS is terminated
by the deployment environment** (no Caddy here).

```
payer  ──paid POST /ilp──▶ connector ──paid write (POST /write)──▶ relay :3100  (store; PRIVATE)
reader ──wss free REQ──────────────────────────────────────────▶ relay :7100  (Nostr reads; PUBLIC)
```

The connector's config is **baked into the `relay-connector` image** (see
`Dockerfile` — `FROM ghcr.io/toon-protocol/connector` + `COPY connector.toml`).
The relay app image (`ghcr.io/toon-protocol/relay`) is published separately and
referenced as a sibling service.

> **This bundle runs the Rust connector** (connector#755). It used to run the
> TypeScript node pinned at `3.28.0`, reading a `connector.yaml`. The TOON devnet
> cut over to the Rust connector on 2026-08-04 and stopped both TypeScript
> containers, so that pin points at a node nobody runs. See
> [Migrating from `3.28.0`](#migrating-from-3280) if you have an existing `.env`.

## Files

| file                 | purpose                                                                                   |
| -------------------- | ----------------------------------------------------------------------------------------- |
| `Dockerfile`         | `relay-connector` image: pinned Rust connector + baked `connector.toml`                    |
| `connector.toml`     | connector config (route `g.toon.relay` → `http://relay:3100/write`), devnet RPC baked in   |
| `docker-compose.yml` | connector (payment proxy) + relay; only the edge `:3000` and free-read WS `:7100` public   |
| `.env.example`       | copy to `.env`; `RELAY_NOSTR_SECRET_KEY` (required) + image pins + ports                   |

## Images

| image                                   | what it is                                                |
| --------------------------------------- | --------------------------------------------------------- |
| `ghcr.io/toon-protocol/relay`           | the normal relay app (built by `publish-relay-image.yml`) |
| `ghcr.io/toon-protocol/relay-connector` | connector + this repo's `connector.toml` baked in         |

The `relay-connector` image bakes a **pinned** connector (`deploy/Dockerfile`'s
`CONNECTOR_TAG` ARG, currently `rust-sha-440eab7`) so the config schema is frozen
against a known connector. The image's own version tracks this repo's release
(`vX.Y.Z` / `latest` / `sha`); bump `CONNECTOR_TAG` deliberately to adopt a newer
connector.

**The `.env` variable of the same name is not a production control.** It only
feeds `docker compose up --build` (a local build). The published image on GHCR
is built by CI from `deploy/Dockerfile`'s own ARG default, with no build-arg
override — see the workflow's header — so pulling `RELAY_CONNECTOR_IMAGE` and
running `up` without `--build` (the documented production path below) ignores
`.env`'s `CONNECTOR_TAG` entirely. To adopt a newer connector in production,
bump the ARG default in `deploy/Dockerfile` and cut a new release.

**Read the tag carefully.** The `connector` package carries two different
programs under one name. `rust-sha-<short>` and `rust-main` are the Rust
connector, which reads `connector.toml`. Plain semver tags (`3.28.0`) and
`latest` are the **retired** TypeScript node, which reads `connector.yaml` and
will not start on this bundle's config. Always pin an exact `rust-sha-`, never
the floating `rust-main`: the parser is `deny_unknown_fields` and startup is
fail-closed, so a schema drift under you is a refuse-to-start.

## Drop-in steps

1. **Generate the connector's two keys.** These are files, not environment
   variables — there is no env layer on the Rust connector, and no
   `TOON_MNEMONIC`.

   ```bash
   # This node's ILP signing identity (ADR 0012). Holds no money. Fresh random
   # material per box — it must NOT collide with any other node's.
   openssl rand -hex 32 > signer.key

   # The settlement identity. This one spends real testnet value and is what
   # clients open their payment channels AGAINST, so derive it from a seed you
   # can reproduce rather than from `openssl rand`. The TOON fleet uses NIP-06
   # m/44'/1237'/0'/0/0 — the NOSTR coin type, NOT the standard m/44'/60'.
   # Deriving at m/44'/60' yields a valid address no channel was ever opened
   # against, and a node that cannot resolve a single one.
   #   ...derive it, then:
   # printf '%s' "<64 hex chars>" > settlement.key

   chmod 600 signer.key settlement.key
   sudo chown 10001:10001 signer.key settlement.key   # the image runs as uid 10001
   ```

   Verify the derived settlement address **before** the first `up -d`. Both
   files are gitignored (`deploy/*.key`).

   > A bind-mounted file keeps its **host** ownership inside the container, so a
   > root-owned `0600` key is unreadable to uid 10001 and the container
   > restart-loops on "Permission denied". The `chown` is the fix — do not reach
   > for `chmod 644`.

2. **Set the relay's identity.**

   ```bash
   cp .env.example .env
   # RELAY_NOSTR_SECRET_KEY is REQUIRED (the relay won't boot without it):
   #   openssl rand -hex 32   → paste into RELAY_NOSTR_SECRET_KEY
   ```

3. **Bring it up.**

   ```bash
   docker compose up --build -d      # builds relay-connector locally; pulls the relay app image
   docker compose ps                 # only :3000 (edge) and :7100 (free WS) are host-bound
   docker compose logs -f connector  # watch it load the route + connect to settlement
   ```

   Production: pin `RELAY_CONNECTOR_IMAGE` to a published tag and run
   `docker compose up -d` (no `--build`).

   **Startup is fail-closed.** A missing key file, an unwritable `/app/state`, or
   a settlement registry that will not resolve the token is `exit 1` with the
   reason — never a degraded run. If the connector exits immediately, the log
   line names which of those it was.

## Verify the paid round-trip

Reuse the connector repo's acceptance probe against this compose (run from the
**connector repo root** — it needs the repo + native `libsql`):

```bash
CONNECTOR_ILP_URL=http://localhost:3000/ilp \
RELAY_WS_URL=ws://localhost:7100 \
EVM_RPC_URL=https://evm-rpc.devnet.toonprotocol.dev \
FAUCET_URL=https://faucet.devnet.toonprotocol.dev \
RELAY_STORE_PROBE_URL=http://localhost:3100/write \
  npx ts-node --project packages/connector/tsconfig.json \
    scripts/app/ci-acceptance-probe.ts
```

It funds a fresh wallet from the devnet faucet, opens an on-chain USDC channel
toward the connector, signs a per-packet claim, and asserts: paid `POST /ilp` →
FULFILL carrying the relay store's response; the event is returned over the free
WS read; an unpaid `POST /ilp` → REJECT; and the relay store (`:3100`) is NOT
publicly reachable. (Against a public edge, point the URLs at the env's HTTPS
hostnames instead of `localhost`.)

## Migrating from `3.28.0`

If you have an existing `.env` and a running stack, these are the breaking
differences. None of them fail quietly — a leftover YAML-ism is a config-load
error by name, because the TOML parser is `deny_unknown_fields`.

| was                                                | now                                                               |
| -------------------------------------------------- | ----------------------------------------------------------------- |
| `deploy/connector.yaml`                            | `deploy/connector.toml` — a different config language             |
| `CONNECTOR_TAG=3.28.0`                             | `CONNECTOR_TAG=rust-sha-…`; a semver tag is the retired node      |
| `TOON_MNEMONIC` derives the settlement key at boot | derive it yourself; mount `settlement.key`                        |
| `CONFIG_FILE=/app/config/connector.yaml`           | nothing — the image's `CMD` already names the path                |
| `NODE_TLS_REJECT_UNAUTHORIZED=0`                   | no equivalent; use an RPC with a real chain of trust              |
| connector health `:8080`, admin `:8081`            | one port: `:3000` carries the edge, the operator surface, metrics |
| route prefix `g.proxy.relay`                       | `g.toon.relay` (the apex was renamed)                             |
| `selfAnnounce` block (kind:10032)                  | `[announce]` + `connector announce` — see below                   |
| replay watermarks lived in process memory          | `state_dir = "/app/state"`, on a named volume                     |

That last row is the one worth dwelling on: without a durable claim journal, a
restart resets every channel's replay watermark, a channel with no watermark
accepts any nonce, and every claim a payer already spent becomes free service
again (connector#605). That is why `docker-compose.yml` gained a
`connector_state` named volume.

### The kind:10032 self-announce, differently

The old `connector.yaml` carried a `selfAnnounce` block — the `IlpPeerInfo`
emitter that let a client holding only the genesis seed discover this node's
routes out of band ([relay#37](https://github.com/toon-protocol/relay/issues/37),
[store#22](https://github.com/toon-protocol/store/issues/22)). It shipped in
TypeScript connector v3.28.0 and has no field-for-field counterpart in the
Rust connector — but connector#784 gave it a real one, and the pin above
carries it: an optional `[announce]` section in `connector.toml` plus a
`connector announce` CLI verb. It is deliberately NOT the same shape:
`selfAnnounce` was a background daemon that republished on a timer;
`connector announce` is a one-shot **operator action** the node never runs on
its own — you invoke it against the relay URL you want to publish *through*, by
hand or from your own cron/sidecar, whenever you want the announce refreshed:

```bash
connector announce --config /app/config/connector.toml <through-url> \
  --to <that node's terminating prefix>
```

It pays that URL like any other client, so the section also needs a
`pay_channel` naming a funded channel to pay from. `--to` is required and is
**not** discoverable from the through-URL — write it once as `publish_to` if you
always publish to the same address. See `connector.toml`'s commented
`[announce]` block for the fields this bundle's route needs (`addresses`,
`http_endpoint`, `btp_endpoint`, and `relay_url` since this box fronts a relay)
and fill in your own public hostnames before uncommenting it — none of them are
inferrable from inside the container.

A deployment that leaves `[announce]` unset can still publish its own
`kind:10032` event as an ordinary paid write through this same edge; on any
connector predating connector#784 that is the only option.

## Privacy invariant

- **relay `:3100` (paid-write store) is never host-published** — the only way in
  is a paid `POST /ilp` to the connector. Enforcement is by construction
  (`expose`, not `ports`).
- **The connector publishes exactly one port, `:3000`.** There is no separate
  health or admin port to leak. The operator surface that shares `:3000` is
  omitted from `connector.toml` entirely, so there is nothing there to
  authenticate against — read that file's `[operator]` comment before enabling it
  on a **baked** config.
- The only host-bound ports are the edge **`:3000`** and the free-read WS
  **`:7100`** — both fronted by the environment's TLS terminator.
- **Since relay#85 this is a security precondition, not just privacy**: the
  relay skips schnorr verification for paid ephemeral kinds because `:3100`
  is reachable only through the payment-gating connector. If you change the
  topology so `:3100` is reachable any other way, set
  `RELAY_VERIFY_EPHEMERAL=true`. Fleet gotcha: docker `ports:` publishes are
  internet-reachable even with ufw locked down to 22/80/443 — never convert
  `:3100` from `expose:` to `ports:`.
