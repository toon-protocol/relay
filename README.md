# relay

A **Nostr relay you get paid to write to.** Reads are free and speak plain
NIP-01, so any Nostr client can use it. Writes arrive through a payment proxy
— the [TOON connector](https://github.com/toon-protocol/connector) — which
settles the payment before the relay ever sees the request.

The relay itself contains no payment code at all. It stores signed events and
serves them; by the time a write reaches it, it is already paid for. That
separation is the whole design, and it is what makes this repo a reference for
putting **any** app behind the connector: everything below the dashed line is
the same for a relay, a file store, or an inference endpoint.

```
                          ╔═══════════════════════════════════╗
  payer ──── POST /ilp ──▶║  Caddy  ──▶  connector            ║  pays, verifies
   (paid write)           ║   :443        :3000               ║  ─────────────
                          ║                 │                 ║
                          ║ ─ ─ ─ ─ ─ ─ ─ ─ │ ─ ─ ─ ─ ─ ─ ─ ─ ║
                          ║                 ▼                 ║
  reader ─── wss:// ─────▶║  Caddy  ──▶  relay  :3100 (write) ║  stores, serves
   (free read)            ║   :443        │     :7100 (read)  ║  ─────────────
                          ╚═══════════════════════════════════╝
                                          └── events.db
```

Only Caddy is reachable from the internet. The relay's write port is not
published on any interface — the only route to it is a paid packet through the
connector.

**Live on the TOON devnet:**

|                           |                                                                                      |
| ------------------------- | ------------------------------------------------------------------------------------ |
| Free reads                | `wss://relay-ws.devnet.toonprotocol.dev`                                             |
| Paid writes               | `https://proxy.relay.devnet.toonprotocol.dev/ilp`                                    |
| What that node is         | `curl https://proxy.relay.devnet.toonprotocol.dev/ilp`                               |
| Where a write is paid for | `curl -H 'Accept: application/nostr+json' https://relay-ws.devnet.toonprotocol.dev/` |

To _use_ the network rather than run a node, start with the
[toon-client rig](https://github.com/toon-protocol/toon-client/blob/main/packages/rig/README.md).

---

## Run a node

Everything below runs on one box with Docker. The whole deployment is
[`deploy/`](deploy/) — five short files, no orchestrator, no build step.

### 1. Point two names at the box

| Record                              | Serves                             |
| ----------------------------------- | ---------------------------------- |
| `proxy.relay.example.com` → your IP | paid writes (the connector's edge) |
| `relay-ws.example.com` → your IP    | free reads (the relay's WebSocket) |

Caddy gets certificates for both on first boot, so DNS must resolve before you
start. Open ports 22, 80 and 443 and nothing else.

### 2. Generate three keys, and the operator surface's two files

```bash
git clone https://github.com/toon-protocol/relay.git
cd relay/deploy

# The connector's ILP identity. Holds no money. Fresh random per box.
openssl rand -hex 32 > signer.key

# The settlement identity — this one spends value, and is what clients open
# their payment channels AGAINST. Derive it from a seed you can reproduce
# (the TOON fleet uses NIP-06 m/44'/1237'/0'/0/0 — the NOSTR coin type, not
# m/44'/60'), then write the 64-hex secret:
printf '%s' "<64 hex chars>" > settlement.key
printf '%s' "<64 hex chars>" > settlement-solana.key   # or drop Solana, see below

chmod 600 *.key
sudo chown 10001:10001 *.key    # the connector image runs as uid 10001

# The operator surface: a bearer token for reads, and the allowlist of keys
# that may sign a WRITE -- establishing a peering, funding a channel,
# originating a packet. The private half (operator-write.key) stays wherever
# you sign from; only its public half goes in the allowlist.
openssl rand -hex 32 > operator-bearer.token
openssl rand -hex 32 > operator-write.key
docker run --rm -v "$PWD:/w" \
  "$(grep -oP 'ghcr\.io/toon-protocol/connector:\S+' docker-compose.yml)" \
  send --operator-key /w/operator-write.key --print-keyid > operator-write.keys
chmod 600 operator-bearer.token operator-write.key
sudo chown 10001:10001 operator-bearer.token operator-write.keys
```

> **Verify the settlement address before you start**, not after. Deriving at
> `m/44'/60'` instead of `m/44'/1237'` yields a perfectly valid address that no
> channel was ever opened against — a node that boots, looks healthy, and
> cannot resolve a single payment.
>
> A bind-mounted file keeps its **host** ownership inside the container, so a
> root-owned key is unreadable to uid 10001 and the container restart-loops on
> "Permission denied". The `chown` is the fix — not `chmod 644`.

Both `.key` files are gitignored. To run EVM-only, delete
`settlement-solana.key`, `[settlement.solana]` and its two sub-tables from
`connector.toml`, and its mount from `docker-compose.yml` — but note that a node only accepts claims on
chains it settles, so an EVM-only node refuses every Solana-paid write.

### 3. Fill in `.env`

```bash
cp .env.example .env
```

Four values are required: `EDGE_HOST`, `READ_HOST`, `ACME_EMAIL`, and
`RELAY_NOSTR_SECRET_KEY` (`openssl rand -hex 32` — the relay's own Nostr
identity, not money). Everything else has a working default.

### 4. Start it

```bash
docker compose up -d
docker compose logs -f connector
```

Startup is **fail-closed**. A missing key file, an unwritable state volume, or
a settlement chain the connector cannot reach is `exit 1` with the reason in
the log — never a degraded node that looks fine and cannot take payment.

### 5. Prove it works

```bash
# What this node is: its addresses, endpoints, route prices, the key a packet
# is sealed to, and the chains it settles on. Free, unauthenticated, and the
# only thing a stranger needs to start paying you.
curl -s https://$EDGE_HOST/ilp | jq

# The edge is serving and has read its signer key.
curl -s -o /dev/null -w '%{http_code}\n' https://$EDGE_HOST/ilp/identity   # 200

# Free reads. A plain GET answers 426 (upgrade required); with a client:
websocat wss://$READ_HOST
["REQ","probe",{"kinds":[1],"limit":1}]

# Where a write to THIS relay is paid for, in the relay's own words: the
# NIP-11 relay information document, on the read host, answered only to a
# request that asks for it by media type. Its `toon` object is the ILP
# address, the connector URL, the sealing key and the carriage — everything a
# client holding nothing but this URL needs in order to buy a write.
curl -s -H 'Accept: application/nostr+json' https://$READ_HOST/ | jq
```

`POST /ilp` speaks binary ILP packets, so curl is not the tool for it — an
arbitrary body is answered `400 invalid packet type byte`, which tells you the
edge is up but nothing about payment. A well-formed _unpaid_ packet is what
gets the `402` payment terms back.

For the real round trip — open a channel, sign a claim, write an event — use
the [toon-client rig](https://github.com/toon-protocol/toon-client/blob/main/packages/rig/README.md).
That is the client side of this protocol, and it is not in this repo.

### On your own machine, without TLS

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d
```

Caddy drops out and free reads appear on `127.0.0.1:7100`; the connector's
edge is already on `127.0.0.1:3000`, published there in the base file. The
relay's write port stays unpublished even here.

---

## How it fits together

| Service     | Image                                      | Job                                                                                                    |
| ----------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| `caddy`     | `caddy:2-alpine`                           | TLS for both hostnames, certificates and renewal. The only service that publishes a port.              |
| `connector` | `ghcr.io/toon-protocol/connector` (pinned) | Terminates payment, delivers the paid request to the relay, answers `GET /ilp` with what this node is. |
| `relay`     | `ghcr.io/toon-protocol/relay`              | Verifies the event signature, stores it, serves NIP-01 reads.                                          |

### The connector's config

[`deploy/connector.toml`](deploy/connector.toml) is mounted read-only into the
stock connector image, whose immutable pin sits in the same compose file — so
the build and the config it was validated against are one commit, and the box
takes both with one `git pull`. It is about twenty lines of actual settings:

| Section            | What it says                                                                                                                                 |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `client_edge_addr` | where `POST /ilp` is served — one listener, no admin or health port                                                                          |
| `state_dir`        | the claim journal, on a named volume, so a restart cannot re-accept a spent claim                                                            |
| `[signer]`         | this node's ILP identity — a key _file_, never a value                                                                                       |
| `[node]`           | the node's own public addresses and endpoints: the facts a container cannot introspect about itself, served on `GET /ilp`                    |
| `[[routes]]`       | a prefix, the URL it delivers to, and a price. `g.toon.relay` → `/write` at 1 micro-USDC; `g.toon.relay.ephemeral` → `/write-ephemeral` at 0 |
| `[settlement.*]`   | the chains this node accepts payment on, and the key it settles with                                                                         |

There is no environment-variable layer: every connector value lives in that
file. Every table is `deny_unknown_fields`, so a stale key is a startup error
that names it rather than a setting silently ignored.

### What the connector sends the relay

The connector replays the payer's own request beneath the route's
`handler_url`, and adds three headers when **it** was the hop that verified the
payment:

| Header          | Value                                                          |
| --------------- | -------------------------------------------------------------- |
| `X-TOON-Payer`  | the client channel key — `evm:0x<64 hex>` or `solana:<base58>` |
| `X-TOON-Amount` | the route's price, in base units                               |
| `X-TOON-Chain`  | `evm` or `solana`                                              |

The relay records a well-formed triple on the write's response and its log
line. **Absence means "this hop did not take the payment" — never "unpaid".**
They are absent on a forwarded packet and on every free route, which is why
the ephemeral lane never sees them.

Whatever the relay answers — `200`, `404`, `422` — is delivered back to the
payer as a fulfilled payment: the payer paid for an answer, not for the answer
they hoped for. Only an unreachable app is a rejection.

### How anyone finds it

By its URL. This node publishes **no announce** and registers with nothing:
the two hostnames in step 1 are the whole of its public identity.

- Hand someone `READ_HOST` and they can read from it with any Nostr client,
  and `GET` it with `Accept: application/nostr+json` for the NIP-11 relay
  information document — which names where a **write** to this relay is paid
  for: the ILP address, the connector URL, the sealing key and the carriage
  that route pins (TOON_Network#121). The relay does not hold those facts; it
  reads them from the connector beside it and republishes what that connector
  says about itself, so the advertisement cannot drift from the enforcement.
- Hand someone `EDGE_HOST` and `GET /ilp` tells them everything they need to
  pay it — its ILP addresses, both endpoints, every route and price, the key a
  packet is sealed to, and the chains and contracts it settles on. No account,
  no registry, no prior knowledge of the protocol.

There used to be a kind:10032 announce that carried a subset of those facts
into the relay corpus on a timer. It is gone (connector ADR 0046 / 0050): a
node that answers for itself at a known URL does not need to advertise, and
the announce could go stale in ways the node itself never could.

### The privacy invariant

- **Caddy is the only service reachable from off the box.** The relay's write
  and read ports are `expose`d on the compose network and nowhere else. The
  connector's edge is published to `127.0.0.1` only — enough for an on-box
  operator call, not reachable from the internet.
- **A docker `ports:` publish with no host IP is internet-reachable even with
  ufw locked to 22/80/443** — Docker's iptables chain runs ahead of ufw. Never
  convert an `expose:` in `docker-compose.yml` into a bare `ports:`, and never
  drop the `127.0.0.1:` from the connector's.
- **This is a security precondition, not just privacy.** The relay skips
  signature verification for _paid_ ephemeral kinds because that port is
  reachable only through the payment gate. If you front it any other way, set
  `RELAY_VERIFY_EPHEMERAL=true`.

[`deploy/bundle.test.ts`](deploy/bundle.test.ts) fails the build if any of
that stops being true.

---

## Operate it

**Relay app updates arrive on their own; connector updates do not.** A green
merge to `main` publishes the relay app image, once the conformance suite has
passed against it, and moves its `:release` tag; the Watchtower overlay
recreates that container, usually within a minute:

```bash
docker compose -f docker-compose.yml -f docker-compose.watchtower.yml up -d
```

It never touches Caddy, which holds the certificates. Every relay build also
keeps two immutable tags, `:rust-<handle>` (the version it reports, a date and
that day's ordinal) and `:rust-sha-<short>`, so holding a build or putting one
back is pinning `RELAY_IMAGE` to one and running `up -d relay`.

`:release` has been the Rust relay since #205. The last TypeScript build is
still `ghcr.io/toon-protocol/relay:sha-7b6bab5`, and pinning `RELAY_IMAGE` to it is
the whole rollback: both relays open the same database. The steps are in
[`deploy/README.md`](deploy/README.md#rolling-back-to-the-typescript-relay).

The **connector** is pinned to an immutable tag in `docker-compose.yml`, which
by definition never moves, so Watchtower has nothing to follow for it. Its
enable label is kept only so the service block matches the other TOON node
bundles, and is inert. What moves the connector instead is a release, and the
box applies it itself — see below.

```bash
# The manual form, still there when you want it:
cd /path/to/relay && git pull && cd deploy
docker compose -f docker-compose.yml -f docker-compose.watchtower.yml up -d connector
```

**To peer with another node**, sign a `POST /peers` naming its URL with
`operator-write.key` — the connector repo's `docs/operators/sign-write.sh`
does the RFC 9421 signing:

```bash
sign-write.sh -k operator-write.key -X POST -p /peers -u https://proxy.relay.<domain> \
  -b '{"id":"store","url":"https://proxy.ario.<domain>/ilp","fee":1,"max_packet_amount":100000,"chain":"solana"}'
sign-write.sh -k operator-write.key -X POST -p /routes/peers -u https://proxy.relay.<domain> \
  -b '{"prefix":"g.toon.relay.store","peer_id":"store","price":1010}'
```

The first reads the counterparty's self-description, derives the payment
channel from the two settlement addresses and opens it if absent (connector
ADR 0058); the second puts a route through it in the table. Both survive a
restart — they live in `connector_state`, not in `connector.toml`.

**Adopting a newer connector happens on its own, from a release.** When the
connector repo cuts a release — one human dispatch, stamping an immutable
`connector:rust-<handle>` that nothing ever moves — this repo notices within
half an hour and opens the pin bump itself
([`.github/workflows/adopt-connector-release.yml`](.github/workflows/adopt-connector-release.yml)).
Before it opens anything it **boots that candidate image against this repo's
own committed `deploy/connector.toml`** and requires the build to accept it;
a build that refused a key by name, renamed a field or newly required one
fails there and no pull request appears. That is connector ADR 0041's
Decision 1 — an image a box follows unattended may only move to a build that
still accepts the committed config — asked at the one moment the candidate
and this node's config are in front of the same machine.

Once that PR merges, the box applies it within five minutes:
[`deploy/auto-apply.sh`](deploy/auto-apply.sh) on a systemd timer
fast-forwards `main`, runs `compose up -d`, and requires the connector to come
back **healthy**. It is pull-based deliberately — no CI job anywhere holds SSH
into a node, which is the posture connector ADR 0068 settled — and it refuses
to touch a box whose working tree is dirty, so a human mid-operation is never
overwritten. Install it once per box:

```bash
sudo cp /root/relay/deploy/toon-auto-apply-relay.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now toon-auto-apply-relay.timer
systemctl list-timers toon-auto-apply-relay.timer     # when it next fires
journalctl -u toon-auto-apply-relay.service -n 50     # what it last did
```

The unit names are per-node (`toon-auto-apply-relay.*`, shared contract v2)
so several nodes can share one host once each has its own timer — see
[`deploy/README.md`](deploy/README.md#migrating-an-existing-box-to-the-per-node-unit-names)
for the one-time migration on a box that already runs the old
`toon-auto-apply.*` names.

The pin is still the only place a connector build is named here, and it is
still immutable — a `rust-sha-` build or a `rust-<handle>` release, never a
moving tag. The config parser is `deny_unknown_fields` and startup is
fail-closed, which is exactly why the gate above runs before the pin moves
rather than after.

**Retention.** What the relay stops serving — NIP-40 expiry, NIP-09 deletion,
and the operator blocklist for events whose author key is gone — is
[`docs/retention.md`](docs/retention.md). Read it before turning expiry
enforcement off on a live node; `RELAY_ENFORCE_EXPIRATION=false` is the kill
switch, and it only recovers events still inside the reap grace window.

### Every setting the relay reads

CLI flags override environment variables. `deploy/` sets these through
`.env`; each setting's flag is named beside it in
[`crates/relay/src/config.rs`](crates/relay/src/config.rs).

| Variable                                        | Default   | What it does                                                        |
| ----------------------------------------------- | --------- | ------------------------------------------------------------------- |
| `TOON_SECRET_KEY` / `NOSTR_SECRET_KEY`          | —         | 64-hex identity key. One of this or `TOON_MNEMONIC` is **required** |
| `TOON_MNEMONIC`                                 | —         | BIP-39 mnemonic, NIP-06 derivation                                  |
| `TOON_RELAY_PORT`                               | `7100`    | WebSocket read port                                                 |
| `TOON_BLS_PORT`                                 | `3100`    | HTTP write / health / metrics port                                  |
| `TOON_HOST`                                     | `0.0.0.0` | read-port bind address                                              |
| `TOON_WRITE_HOST`                               | `0.0.0.0` | write-port bind address                                             |
| `TOON_DATA_DIR`                                 | `./data`  | where `events.db` lives                                             |
| `TOON_DEV_MODE`                                 | `false`   | `true` is refused at startup: there is no mode that skips verifying |
| `TOON_VERIFY_EPHEMERAL`                         | `false`   | full verification on paid ephemeral kinds too                       |
| `TOON_VERIFY_WORKERS`                           | —         | accepted and has no effect; logged once when set                    |
| `TOON_MAX_CONNECTIONS`                          | `4096`    | concurrent WS reads (one file descriptor each)                      |
| `TOON_LOG_WRITES`                               | `false`   | one log line per accepted write                                     |
| `TOON_ENFORCE_EXPIRATION`                       | `true`    | stop serving events past their NIP-40 `expiration`                  |
| `TOON_EXPIRATION_REAP_GRACE_SECONDS`            | `86400`   | how long an expired event stays on disk                             |
| `TOON_EXPIRATION_REAP_INTERVAL_SECONDS`         | `3600`    | how often the reaper sweeps; `0` never                              |
| `TOON_BLOCKED_EVENT_IDS`                        | —         | comma-separated 64-hex event ids to refuse. Ids only, never pubkeys |
| `TOON_NIP42_AUTH`                               | `false`   | Rust relay: `true` sends every connection a NIP-42 challenge and lists 42 in `supported_nips` |
| `TOON_AUTH_REQUIRED_KINDS`                      | —         | Rust relay: comma-separated kinds a connection must `AUTH` to read (a `REQ` that could return one is closed `auth-required:`); implies `TOON_NIP42_AUTH` |
| `TOON_NIP29_GROUPS`                             | `false`   | Rust relay: `true` keeps NIP-29 relay groups (membership and roles enforced on writes, metadata kinds 39000–39003 published, closed/private groups read only by an authenticated member), lists 29 in `supported_nips` and implies `TOON_NIP42_AUTH` |
| `TOON_CONNECTOR_URL`                            | —         | the connector's `GET /ilp` this relay reads its write edge from     |
| `TOON_WRITE_ILP_ADDRESS`                        | —         | which of that connector's routes reaches this relay's `POST /write` |
| `TOON_WRITE_CARRIAGE`                           | —         | the carriage that route pins; fills silence only (TOON_Network#111) |
| `TOON_SUBSCRIBE_ILP_ADDRESS`                    | —         | sells the live feed: the connector route that reaches `POST /subscribe` ([docs/paid-feed.md](docs/paid-feed.md)) |
| `TOON_BROADCAST_PRICE`                          | —         | what one broadcast event debits from a subscriber (with the two around it) |
| `TOON_RELAY_URL`                                | —         | the URL clients reach this relay at: its host is checked in NIP-42 and NIP-98 |
| `TOON_OPERATOR_PUBKEYS`                         | —         | comma-separated hex keys that follow the live feed without paying (the relay's own key always does) |
| `TOON_RELAY_NAME` / `_DESCRIPTION` / `_CONTACT` | —         | NIP-11 free text; an empty value is left out of the document        |
| `TOON_EPHEMERAL_RATE_LIMIT`                     | `200`     | free-lane requests per key per window                               |
| `TOON_EPHEMERAL_RATE_WINDOW_MS`                 | `10000`   | free-lane rate-limit window                                         |
| `TOON_EPHEMERAL_MAX_BODY_BYTES`                 | `8192`    | free-lane request body cap                                          |
| `TOON_READ_RATE_LIMIT`                          | `1200`    | REQs a minute one read connection is answered (Rust relay)          |
| `TOON_READ_SOURCE_RATE_LIMIT`                   | `6000`    | REQs a minute all connections of one source address share (Rust relay) |

A REQ over either limit is `CLOSED` with `rate-limited: … slow down, or subscribe`
(an open subscription is free: it streams live events and is not counted again),
and both limits are in the NIP-11 `limitation` as `max_req_per_minute_per_connection`
and `max_req_per_minute_per_source`. The source is the TCP peer's address, so behind
a reverse proxy every client is the proxy and the source limit is one limit for all of
them: raise it to suit, or leave the proxy's own limits to tell clients apart.

---

## Develop

```bash
pnpm install
pnpm -r build
pnpm test
pnpm lint && pnpm typecheck
```

Node 22 and pnpm 8.15.9. [Devbox](https://www.jetify.com/devbox/docs/installing_devbox/)
pins both to the versions CI uses — `devbox shell`, then `devbox run build`,
`devbox run test`, `devbox run lint`.

The relay is a Rust Cargo workspace (`Cargo.toml`, `crates/relay`), on the toolchain
`rust-toolchain.toml` pins — rustup installs it on the first `cargo` call, and
devbox provides rustup. It follows
[`docs/rust-coding-standards.md`](docs/rust-coding-standards.md) and is gated
by the [conformance suite](packages/conformance/README.md), which CI runs
against the image. The pnpm workspace holds only that suite, the soak tooling
and the guards in `deploy/`.

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Merging to `main` publishes the Rust image and moves `:release`. There is no
npm package and no changeset.
The agent factory that opens many of the PRs here is described in
[`docs/agents/triage-labels.md`](docs/agents/triage-labels.md).

## Running the relay without a container

Each publish from `main` also attaches the relay as a bare binary to a GitHub
release tagged `rust-<handle>` (the same handle as the image's `rust-<handle>`
tag): `relay-linux-x86_64` and `relay-linux-aarch64`, static musl builds with
nothing to install beside them, and `SHA256SUMS` over both.

```bash
TAG=rust-2026.10.02.1   # a tag from the Releases page
ARCH=$(uname -m)        # x86_64 or aarch64
BASE=https://github.com/toon-protocol/relay/releases/download/$TAG
curl -fsSLO "$BASE/relay-linux-$ARCH" -O "$BASE/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
chmod +x "relay-linux-$ARCH"
```

It is the same program the image runs, configured by the same `TOON_*`
environment variables and the same flags (`./relay-linux-$ARCH --help`;
[every setting the relay reads](#every-setting-the-relay-reads)). The ports
default to the image's (3100 and 7100); the one image default the binary does
not share is the data directory, `/data` in the image and `./data` (under the
working directory) outside it. Only an identity is required:

```bash
TOON_SECRET_KEY=<64 hex> ./relay-linux-$ARCH
```

## Where to go next

|                                                                       |                                                                       |
| --------------------------------------------------------------------- | --------------------------------------------------------------------- |
| [`packages/conformance/README.md`](packages/conformance/README.md)    | the black-box suite a relay image must pass, and the soak tooling     |
| [`deploy/README.md`](deploy/README.md)                                | the deployment files, one by one                                      |
| [`docs/retention.md`](docs/retention.md)                              | what the relay stops serving, and how to stop it                      |
| [toon-protocol/connector](https://github.com/toon-protocol/connector) | the payment proxy: config reference, operator surface, protocol specs |

MIT
