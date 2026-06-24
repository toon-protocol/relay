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
`Dockerfile` — `FROM ghcr.io/toon-protocol/connector` + `COPY connector.yaml`).
The relay app image (`ghcr.io/toon-protocol/relay`) is published separately and
referenced as a sibling service.

## Files

| file                 | purpose                                                                                  |
| -------------------- | ---------------------------------------------------------------------------------------- |
| `Dockerfile`         | `relay-connector` image: pinned connector + baked `connector.yaml`                       |
| `connector.yaml`     | connector config (route `g.connector.relay` → `http://relay:3100`), devnet RPC baked in  |
| `docker-compose.yml` | connector (payment proxy) + relay; only the edge `:3000` and free-read WS `:7100` public |
| `.env.example`       | copy to `.env`; `RELAY_NOSTR_SECRET_KEY` (required) + `TOON_MNEMONIC` + image pins        |

## Images

| image                                       | what it is                                              |
| ------------------------------------------- | ------------------------------------------------------- |
| `ghcr.io/toon-protocol/relay`               | the normal relay app (built by `publish-relay-image.yml`)|
| `ghcr.io/toon-protocol/relay-connector`     | connector + this repo's `connector.yaml` baked in        |

The `relay-connector` image bakes a **pinned** connector (`CONNECTOR_TAG`, default
`3.24.2`) so the config schema and the HTTP-envelope contract are frozen against a
known connector. The image's own version tracks this repo's release (`vX.Y.Z` /
`latest` / `sha`); bump `CONNECTOR_TAG` deliberately to adopt a newer connector.

## Drop-in steps

1. **Set identities.**

   ```bash
   cp .env.example .env
   # RELAY_NOSTR_SECRET_KEY is REQUIRED (the relay won't boot without it):
   #   openssl rand -hex 32   → paste into RELAY_NOSTR_SECRET_KEY
   # TOON_MNEMONIC is optional (empty → pre-funded anvil account-0 devnet fallback).
   ```

   If you set `TOON_MNEMONIC`, also set `routes[].settlementAddresses.evm` in
   `connector.yaml` to the EVM address the connector prints at boot.

2. **Bring it up.**

   ```bash
   docker compose up --build -d      # builds relay-connector locally; pulls the relay app image
   docker compose ps                 # only :3000 (edge) and :7100 (free WS) are host-bound
   docker compose logs -f connector  # watch it register the route + chain provider
   ```

   Production: pin `RELAY_CONNECTOR_IMAGE` to a published tag and run
   `docker compose up -d` (no `--build`).

## Verify the paid round-trip

Reuse the connector repo's acceptance probe against this compose (run from the
**connector repo root** — it needs the repo + native `libsql`):

```bash
NODE_TLS_REJECT_UNAUTHORIZED=0 \
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

## Privacy invariant

- **relay `:3100` (paid-write store) is never host-published** — the only way in
  is a paid `POST /ilp` to the connector. Enforcement is by construction
  (`expose`, not `ports`).
- **connector `:8080` / admin `:8081` are never host-published.**
- The only host-bound ports are the edge **`:3000`** and the free-read WS
  **`:7100`** — both fronted by the environment's TLS terminator.
