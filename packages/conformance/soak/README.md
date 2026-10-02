# Soaking a relay image

What the conformance suite cannot say about an image: that it works behind a
real connector on the paid path, and how it compares with the image it is
meant to replace. Three things, all run from this machine against published
images (#203). Nothing here runs in CI.

## 1. The infra sandbox, on the image under test

The sandbox (`toon-protocol/infra`, `sandbox/`) pins its own relay image and
does not tell it where its Write Edge is. `sandbox.override.yml` swaps the
image and sets the two connector-edge variables, from outside the infra repo:

```
cd ../infra/sandbox
export COMPOSE_FILE=docker-compose.yml:$RELAY_REPO/packages/conformance/soak/sandbox.override.yml
export RELAY_IMAGE=ghcr.io/toon-protocol/relay:rust-candidate
make up-payments                 # PROVIDER_CONTEXT=… if ../../provider is not on client 4.x
make smoke-payments              # paid write, free read-back, x402 deposit
make smoke-directory             # the provider's paid, replaceable directory events
```

`COMPOSE_FILE` has to stay exported for every later `make` or `docker compose`
in that shell: the smokes drive compose themselves. To compare, repeat with
`RELAY_IMAGE` set to the other image. `docker compose --profile payments up -d
relay` swaps the relay alone, over the same `/data` volume, which is the
cutover and the rollback. `make clean` afterwards leaves the infra checkout as
it was.

## 2. Paid writes through the hub

```
SANDBOX=../infra/sandbox node soak/paid-writes.mjs
```

`WRITES` (default 300) paid writes from the sandbox's funded buyer, one
voucher each, then a read of all of them. It reports writes per second and the
latency a payer sees. Nearly all of that is the connector and the client's
signing, so it says whether the relay holds the paid path up, not how fast the
relay is.

## 3. The benchmark

```
BENCH_IMAGES="typescript=ghcr.io/toon-protocol/relay:release,rust=ghcr.io/toon-protocol/relay:rust-candidate" \
  pnpm --filter @toon-protocol/relay-conformance bench
```

Starts each image alone and measures memory at idle, deliveries to `POST
/write` carrying the connector's payment statement, and fan-out to live
subscriptions; see the head of `bench.mjs` for the settings. Images are
interleaved within a round and the medians are reported, as a Markdown table
on stdout. `BENCH_CPUS=1` gives every image the same single core.

## The suite on a firewalled host

The suite's stub connector listens on the host, and a host firewall that drops
traffic from the Docker bridge fails every case that needs the Write Edge. A
Docker-in-Docker daemon has no such firewall: load both images into a
`docker:dind` container, and run the suite in a `node` container that shares
its network namespace with `DOCKER_HOST=tcp://127.0.0.1:2375`.
