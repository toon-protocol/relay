# Soaking a relay image

What the conformance suite cannot say about an image: that it works behind a
real connector on the paid path, and how it compares with the image it is
meant to replace. Three things, all run from this machine against published
images (#203). Nothing here runs in CI.

## 1. The infra sandbox, on the image under test

The sandbox (`toon-protocol/infra`, `sandbox/`) pins its own relay image and
does not tell it where its Write Edge is. `sandbox.override.yml` swaps the
image and sets the two connector-edge variables, from outside the infra repo.
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
