# relay conformance suite

A black-box suite for a relay **container image**. It starts the image, starts
a stub HTTP server standing in for the connector's `GET /ilp`, and talks only
to the relay's read port (WebSocket + NIP-11) and write port (`POST /write`,
`GET /health`). It imports nothing from the relay's source, so the same suite
can gate any implementation of the relay's wire contract.

```
docker build -f packages/relay/Dockerfile -t relay:ci .
CONFORMANCE_IMAGE=relay:ci pnpm --filter @toon-protocol/relay-conformance conformance
```

| env                 | meaning                                               | default      |
| ------------------- | ----------------------------------------------------- | ------------ |
| `CONFORMANCE_IMAGE` | image reference under test (required)                 | —            |
| `CONFORMANCE_IMPL`  | name of the implementation, e.g. `typescript`, `rust` | `typescript` |

## Expected failures

A test that one implementation is known to fail is declared with
`conformanceTest(name, fn, { expectedFailureFor: ['rust'] })`. Under `rust` it
runs as `it.fails` (it must still fail, and goes red when it starts passing, so
the marker cannot go stale); under every other implementation it is an
ordinary test.

## What it covers

| file                | covers                                                                                                                                                                                        |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tracer.test.ts`    | one paid write, one free read, one document fetch                                                                                                                                             |
| `document.test.ts`  | the Relay Information Document in each edge state (known, no connector, unreachable, address not terminated), carriage precedence, `limitation`, `fees`, `supported_nips`, CORS and `OPTIONS` |
| `endpoints.test.ts` | `GET /health` and `GET /metrics`                                                                                                                                                              |
| `startup.test.ts`   | a connector that is down at start, settings the relay must refuse (exit non-zero with an `Error:` line), every documented env variable                                                        |

The stub connector is varied per case (`ilpDocument()` overrides, or down /
absent). A run that passes command-line flags uses the image's documented
command, `node dist/cli.js`; set `CONFORMANCE_COMMAND` for an implementation
whose command differs.
