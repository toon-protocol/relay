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

## Coverage

`suite/tracer.test.ts` proves the harness; `suite/store.test.ts` covers what
the store keeps, replaces, deletes and expires (replaceable and addressable
kinds, tag filters, kind 5, NIP-40 with enforcement on and off, the operator
blocklist, duplicates), observed only through writes and reads on the wire.
Known differences between the TypeScript relay and the spec are marked
`expectedFailureFor: ['typescript']`.
