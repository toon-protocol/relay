// Paid-write throughput through the whole paid path of a running infra
// sandbox: client -> hub connector -> relay POST /write, each write a voucher
// on the buyer's own x402 channel, then one free read of all of them (#203). Run it once per relay image, on the same sandbox profile.
//
//   SANDBOX=<path to infra/sandbox> node soak/paid-writes.mjs
//
// The client library, the endpoint rewrite and the buyer's funded identity
// are the sandbox's own (its node_modules and scripts/), read from SANDBOX,
// which is why this file imports them by path. What it measures is dominated
// by the connector and the client's signing; it shows what a payer sees, and
// bench.mjs beside it is the one that isolates the relay.
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const SANDBOX = resolve(process.env.SANDBOX ?? '');
if (!process.env.SANDBOX) {
  console.error('SANDBOX=<path to infra/sandbox> is required');
  process.exit(2);
}
const from = (path) => import(pathToFileURL(join(SANDBOX, path)).href);
const { ToonClient } = await from(
  'node_modules/@toon-protocol/client/dist/index.js'
);
const { finalizeEvent, generateSecretKey } = await from(
  'node_modules/nostr-tools/lib/esm/pure.js'
);
const { hostFetch } = await from('scripts/lib/sandbox-endpoints.mjs');

const WRITES = Number(process.env.WRITES ?? 300);
// The read-back is one answer, and the Rust relay serves at most 500 events
// per filter: refused here, before anything is paid for.
if (!(WRITES >= 1 && WRITES <= 500)) {
  console.error('WRITES must be between 1 and 500');
  process.exit(2);
}
const HUB = process.env.HUB_URL ?? 'http://localhost:3200';
const RELAY_WS = process.env.RELAY_WS ?? 'ws://localhost:7100';

// The sandbox smoke's buyer (scripts/smoke-toon.mjs): anvil's published test
// mnemonic, funded in mock USDC by the sandbox's own seed job, and the same
// channel store, so a run reuses the channel the smoke opened.
mkdirSync(join(SANDBOX, '.toon-client'), { recursive: true });
const client = await ToonClient.create({
  connector: HUB,
  mnemonic: 'test test test test test test test test test test test junk',
  chain: 'solana',
  rpcUrl: process.env.RPC_URL ?? 'http://127.0.0.1:8899',
  channelStore: join(SANDBOX, '.toon-client', 'channels.json'),
  deposit: 10_000_000n,
  timeoutMs: 60_000,
  fetch: hostFetch(),
});
await client.channel.open();

const secretKey = generateSecretKey();
const created_at = Math.floor(Date.now() / 1000);
const events = Array.from({ length: WRITES }, (_, i) =>
  finalizeEvent(
    { kind: 1, created_at, tags: [], content: `paid write ${i}` },
    secretKey
  )
);

async function write(event) {
  const startedAt = performance.now();
  const answer = await client.send('g.toon.relay', {
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ event }),
  });
  if (answer.fulfilled !== true || answer.status !== 200) {
    throw new Error(
      `paid write refused: fulfilled=${answer.fulfilled} status=${answer.status}`
    );
  }
  return performance.now() - startedAt;
}

// One at a time: a channel's vouchers are cumulative, so one buyer's writes
// are a sequence, not a pool.
const latencies = [];
const startedAt = performance.now();
for (const event of events) latencies.push(await write(event));
const seconds = (performance.now() - startedAt) / 1000;
latencies.sort((a, b) => a - b);
const at = (p) =>
  latencies[
    Math.min(latencies.length - 1, Math.floor((latencies.length * p) / 100))
  ];

// Read-back: everything this run paid for is stored and served.
const stored = await new Promise((done, fail) => {
  const socket = new WebSocket(RELAY_WS);
  const ids = new Set();
  const timer = setTimeout(
    () => fail(new Error(`no EOSE from ${RELAY_WS}`)),
    30_000
  );
  socket.onerror = () => fail(new Error(`cannot reach ${RELAY_WS}`));
  socket.onopen = () =>
    socket.send(
      JSON.stringify([
        'REQ',
        'soak',
        { authors: [events[0].pubkey], kinds: [1], limit: WRITES },
      ])
    );
  socket.onmessage = (message) => {
    const frame = JSON.parse(message.data);
    if (frame[0] === 'EVENT') ids.add(frame[2].id);
    if (frame[0] === 'EOSE') {
      clearTimeout(timer);
      socket.close();
      done(ids.size);
    }
  };
});
if (stored !== WRITES)
  throw new Error(`read back ${stored} of ${WRITES} paid writes`);

console.log(
  JSON.stringify({
    writes: WRITES,
    paidWritesPerSecond: Number((WRITES / seconds).toFixed(1)),
    p50Ms: Number(at(50).toFixed(1)),
    p99Ms: Number(at(99).toFixed(1)),
    readBack: stored,
  })
);
// The client keeps its connections open; nothing is left to wait for.
process.exit(0);
