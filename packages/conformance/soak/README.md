# Soaking a relay image

What the conformance suite cannot say about an image: that it works behind a
real connector on the paid path, and how it compares with the image it is
meant to replace. Six things, all run from this machine against published
images (#203). Nothing here runs in CI.

## 1. The infra sandbox, on the image under test

The sandbox (`toon-protocol/infra`, `sandbox/`) pins its own relay image.
`sandbox.override.yml` swaps the image from outside the infra repo. It also
sets the two connector-edge variables, which a sandbox older than infra#51
does not set itself.
From the infra checkout's `sandbox/`, with `RELAY_REPO` the absolute path of
this repo:

```
export COMPOSE_FILE=docker-compose.yml:$RELAY_REPO/packages/conformance/soak/sandbox.override.yml
export RELAY_IMAGE=ghcr.io/toon-protocol/relay:rust-candidate
make up-payments                 # PROVIDER_CONTEXT=… if ../../provider is not on client 4.x
make smoke-payments              # paid write, free read-back, x402 deposit
make smoke-directory             # the provider's paid, replaceable directory events
```

`COMPOSE_FILE` has to stay exported for every later `make` or `docker compose`
in that shell: the smokes drive compose themselves. To compare, repeat with
`RELAY_IMAGE` set to the other image. `docker compose --profile payments up -d
relay` swaps the relay alone, over the same `/data` volume, which is what a
cutover or a rollback does. `make clean` afterwards removes the sandbox's
volumes and state.

## 2. Paid writes through the hub

From `packages/conformance`, with the sandbox up:

```
SANDBOX=/path/to/infra/sandbox node soak/paid-writes.mjs
```

`WRITES` (default 300, at most 500) paid writes from the sandbox's funded
buyer, one voucher each, then a read of all of them. It reports writes per
second and the latency a payer sees. Nearly all of that is the connector and
the client's signing, so it says whether the relay holds the paid path up, not
how fast the relay is.

## 3. The benchmark

```
BENCH_IMAGES="typescript=ghcr.io/toon-protocol/relay:release,rust=ghcr.io/toon-protocol/relay:rust-candidate" \
  pnpm --filter @toon-protocol/relay-conformance bench
```

Starts each image alone and measures memory at idle, memory with live
subscriptions open, fan-out of one stored write to all of them, and deliveries
to `POST /write` carrying the connector's payment statement; see the head of
`bench.mjs` for the settings. The images alternate from round to round and the
medians are reported, as a Markdown table on stdout. `BENCH_CPUS=1` gives
every image the same single core.

## 4. The differences

```
PROBE_IMAGES="typescript=ghcr.io/toon-protocol/relay:release,rust=ghcr.io/toon-protocol/relay:rust-candidate" \
  pnpm --filter @toon-protocol/relay-conformance probe
```

Sends the same malformed and edge inputs to every image, on both ports and
over WebSocket, and prints the ones they answer differently as a Markdown
table. Each row is either a divergence that is written down (#185's
compatibility contract, the suite's `expectedFailureFor: ['typescript']`
cases) or a bug to file.

## 5. The suite on both images

Against each image in turn, with the image-swap case, inside a
Docker-in-Docker daemon: for a host whose firewall keeps a relay from the
suite's stub connector.

```
TYPESCRIPT_IMAGE=ghcr.io/toon-protocol/relay:release \
RUST_IMAGE=ghcr.io/toon-protocol/relay:rust-candidate \
  packages/conformance/soak/dind-conformance.sh
```

## 6. A live node, across an image swap

What sections 1 to 5 cannot say: that the image opens a node's own database,
with what earlier builds left in it. `box.mjs` reads a running node
from outside, through its two public hostnames (`READ_URL` and `EDGE_URL`,
the devnet node's by default), and writes nothing (#204).

```
pnpm --filter @toon-protocol/relay-conformance --silent box baseline > baseline.json
```

before the swap records the Relay Information Document and every stored
event. Then, on the node, pin the image in `deploy/.env` and recreate the
relay alone:

```
RELAY_IMAGE=ghcr.io/toon-protocol/relay:rust-<handle>    # in deploy/.env
docker compose up -d relay
```

`.env` is not in the repository, so `auto-apply.sh` leaves the pin alone, and
Watchtower has nothing to follow on a tag that never moves. `:release` is not
involved. After the swap:

```
EXPECT_VERSION=<handle> pnpm --filter @toon-protocol/relay-conformance --silent box check baseline.json
```

exits 1 unless the read host answers `426`, the connector's `/ilp/identity`
`200`, the document is the baseline's apart from `version` and the two limits
of #233, an `EVENT` over WebSocket is refused with the write address, and every
baseline event is still served as it was (or has expired, been replaced or
been deleted since). The fleet's own verdict on the node is the connector
repository's `fleet-health.yml`.

A rollback is the same three steps with `RELAY_IMAGE` on the last TypeScript
`sha-<short>` tag and `EXPECT_VERSION` the package version. Take a new
baseline first, so that what the candidate stored is checked too.
