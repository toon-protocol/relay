# relay conformance suite

A black-box suite for a relay **container image**. It starts the image, starts
a stub HTTP server standing in for the connector's `GET /ilp`, and talks only
to the relay's read port (WebSocket + NIP-11) and write port (`POST /write`,
`POST /write-ephemeral`, `GET /health`, and the retired paths that must `404`). It imports nothing from the relay's source, so the same suite
can gate any implementation of the relay's wire contract.

```
docker build -f packages/relay/Dockerfile -t relay:ci .
CONFORMANCE_IMAGE=relay:ci pnpm --filter @toon-protocol/relay-conformance conformance
```

Against the Rust relay (`crates/relay`, #185):

```
docker build -f crates/relay/Dockerfile -t relay-rust:ci .
CONFORMANCE_IMAGE=relay-rust:ci CONFORMANCE_IMPL=rust CONFORMANCE_COMMAND=relay \
  pnpm --filter @toon-protocol/relay-conformance conformance
```

CI runs both on every pull request. The relay must be able to reach the stub
connector on the host (`host.docker.internal`); a host firewall that drops
traffic from the Docker bridge makes every case that needs the write edge fail.

| env                   | meaning                                               | default            |
| --------------------- | ----------------------------------------------------- | ------------------ |
| `CONFORMANCE_IMAGE`   | image reference under test (required)                 | —                  |
| `CONFORMANCE_IMPL`    | name of the implementation, e.g. `typescript`, `rust` | `typescript`       |
| `CONFORMANCE_COMMAND` | the image's command, for a run that passes flags      | `node dist/cli.js` |

## Expected failures

A test that one implementation is known to fail is declared with
`conformanceTest(name, fn, { expectedFailureFor: ['rust'] })`. Under `rust` it
runs as `it.fails` (it must still fail, and goes red when it starts passing, so
the marker cannot go stale); under every other implementation it is an
ordinary test.

The Rust relay is being built one surface at a time, so many cases carry
`'rust'` today. It answers `GET /health`; takes a regular event on
`POST /write`, stores it and serves it over WebSocket, stored or live; refuses
`EVENT` over WebSocket; answers `426` to a plain `GET` on the read port; and
refuses a bad identity or port. Replaceable, addressable, deletion and
ephemeral kinds are answered `501` until the store slices land. A change that
builds a surface removes that surface's `'rust'` markers in the same change,
because a marked case that passes is red. Cases that wait for a relay to
refuse a setting it does not read yet run to the harness's 60s deadline, which
is what makes the Rust run take minutes.

## Coverage

- `tracer.test.ts`: health, one paid write and read, the NIP-11 document.
- `write.test.ts`: `POST /write` statuses, `X-TOON-*` payment attribution,
  live delivery of stored writes, `POST /write-ephemeral`, retired paths.
- `ephemeral-rate-limit.test.ts`: the ephemeral `429`, in its own relay so
  exhausting the limiter cannot starve the other cases.
- `store.test.ts`: what the store keeps, replaces, deletes and expires
  (replaceable and addressable kinds, tag filters, kind 5, NIP-40 with
  enforcement on and off, the operator blocklist, duplicates), observed only
  through writes and reads on the wire. Known differences between the
  TypeScript relay and the spec are marked `expectedFailureFor: ['typescript']`.
- `document.test.ts`: the Relay Information Document in each edge state
  (known, no connector, unreachable, address not terminated), carriage
  precedence, `limitation`, `fees`, `supported_nips`, CORS and `OPTIONS`.
- `endpoints.test.ts`: `GET /health` and `GET /metrics`.
- `startup.test.ts`: a connector that is down at start, settings the relay
  must refuse (exit non-zero with an `Error:` line), every documented env
  variable.

The stub connector is varied per case (`ilpDocument()` overrides, or down /
absent). A run that passes command-line flags uses the image's documented
command, `node dist/cli.js`; set `CONFORMANCE_COMMAND` for an implementation
whose command differs.
