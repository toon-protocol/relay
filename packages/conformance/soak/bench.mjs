// One benchmark for a relay container image, black-box like the suite beside
// it: it starts the image and talks only to its two ports, and reads the
// container's memory from its cgroup (#203).
//
//   BENCH_IMAGES="typescript=<image>,rust=<image>" node soak/bench.mjs
//
// Each round starts every image in turn on a fresh /data volume and measures,
// in this order:
//
//   idle       memory once the relay has been healthy and untouched for
//              IDLE_SECONDS
//   subscribed memory with SUBSCRIBERS live subscriptions open and nothing
//              being written, on the still-empty database
//   fan-out    FANOUT_EVENTS stored writes, one at a time, each timed from
//              its POST until the last subscriber has it
//   writes     with the subscribers gone, WRITES signed events delivered to
//              POST /write the way the connector delivers a paid one (the
//              X-TOON-* attribution triple on each, echoed back by the
//              relay), WRITE_CONCURRENCY at a time: events/s, and the latency
//              of one delivery. Nothing is paid here: paid-writes.mjs is the
//              run through a connector.
//
// The order the images run in alternates from round to round, so that drift
// on the host lands on both, and the figure reported for each is the median
// of the rounds. The numbers compare images on one host; they are not a
// capacity figure for any of them.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { finalizeEvent, generateSecretKey } from 'nostr-tools/pure';
import WebSocket from 'ws';

const run = promisify(execFile);
const docker = async (...args) => (await run('docker', args)).stdout.trim();
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const number = (name, fallback) => Number(process.env[name] ?? fallback);
const ROUNDS = number('ROUNDS', 3);
const IDLE_SECONDS = number('IDLE_SECONDS', 15);
const WRITES = number('WRITES', 5000);
const WRITE_CONCURRENCY = number('WRITE_CONCURRENCY', 32);
const SUBSCRIBERS = number('SUBSCRIBERS', 500);
const FANOUT_EVENTS = number('FANOUT_EVENTS', 200);
// Optional `docker run --cpus` for every image, to compare on equal cores.
const CPUS = process.env.BENCH_CPUS;

const IMAGES = (process.env.BENCH_IMAGES ?? '')
  .split(',')
  .filter(Boolean)
  .map((pair) => {
    const at = pair.indexOf('=');
    return at < 1 ? {} : { name: pair.slice(0, at), image: pair.slice(at + 1) };
  });
if (IMAGES.length === 0 || IMAGES.some((i) => !i.name || !i.image)) {
  console.error('BENCH_IMAGES="<name>=<image>[,<name>=<image>…]" is required');
  process.exit(2);
}

// What the connector states on a paid delivery (connector ADR 0040).
const ATTRIBUTION = {
  'X-TOON-Payer': `evm:0x${'ab'.repeat(32)}`,
  'X-TOON-Amount': '1',
  'X-TOON-Chain': 'evm',
};

const KIND = 1;
function signed(count, label, created_at) {
  const secretKey = generateSecretKey();
  return Array.from({ length: count }, (_, i) =>
    finalizeEvent(
      { kind: KIND, created_at, tags: [], content: `${label} ${i}` },
      secretKey
    )
  );
}

async function hostPort(container, port) {
  const match = /:(\d+)$/m.exec(await docker('port', container, `${port}/tcp`));
  if (!match) throw new Error(`no host port for ${port}`);
  return Number(match[1]);
}

async function start(image) {
  const container = await docker(
    'run',
    '-d',
    '--rm',
    '-p',
    '127.0.0.1::3100',
    '-p',
    '127.0.0.1::7100',
    '-e',
    `TOON_SECRET_KEY=${'1'.repeat(64)}`,
    '-e',
    'TOON_MAX_CONNECTIONS=4096',
    ...(CPUS ? ['--cpus', CPUS] : []),
    image
  );
  try {
    const writeUrl = `http://127.0.0.1:${await hostPort(container, 3100)}`;
    const readUrl = `ws://127.0.0.1:${await hostPort(container, 7100)}`;
    const deadline = Date.now() + 60_000;
    for (;;) {
      const healthy = await fetch(`${writeUrl}/health`).then(
        (r) => r.ok,
        () => false
      );
      if (healthy) break;
      if (Date.now() > deadline)
        throw new Error(`${image} never became healthy`);
      await sleep(100);
    }
    return { container, writeUrl, readUrl };
  } catch (error) {
    await remove(container);
    throw error;
  }
}

const remove = (container) =>
  docker('rm', '-f', '-v', container).catch(() => undefined);

/**
 * The container's memory in MiB as `docker stats` counts it: its cgroup's
 * usage less its inactive file cache.
 */
async function memoryMiB(container) {
  const read = (file) =>
    docker('exec', container, 'cat', `/sys/fs/cgroup/${file}`);
  const current = Number(await read('memory.current'));
  const inactive = /^inactive_file (\d+)$/m.exec(await read('memory.stat'));
  return (current - Number(inactive?.[1] ?? 0)) / 2 ** 20;
}

async function post(writeUrl, event) {
  const startedAt = performance.now();
  const response = await fetch(`${writeUrl}/write`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...ATTRIBUTION },
    body: JSON.stringify({ event }),
  });
  const body = await response.text();
  const elapsed = performance.now() - startedAt;
  if (response.status !== 200 || !body.includes('"payment"')) {
    throw new Error(`POST /write answered ${response.status} ${body}`);
  }
  return elapsed;
}

/** `task(item)` over `items`, `concurrency` at a time; resolves with results. */
async function pooled(items, concurrency, task) {
  const results = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: concurrency }, async () => {
      while (next < items.length) {
        const i = next++;
        results[i] = await task(items[i]);
      }
    })
  );
  return results;
}

const percentile = (sorted, p) =>
  sorted[Math.min(sorted.length - 1, Math.floor((sorted.length * p) / 100))];
const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = sorted.length >> 1;
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};

async function writes({ writeUrl }, events) {
  const startedAt = performance.now();
  const latencies = await pooled(events, WRITE_CONCURRENCY, (event) =>
    post(writeUrl, event)
  );
  const seconds = (performance.now() - startedAt) / 1000;
  latencies.sort((a, b) => a - b);
  return {
    writesPerSecond: events.length / seconds,
    writeP50Ms: percentile(latencies, 50),
    writeP99Ms: percentile(latencies, 99),
  };
}

/** Open one live subscription into `sockets`; `onEvent` runs per EVENT. */
function subscribe(readUrl, sockets, onEvent) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(readUrl);
    sockets.push(ws);
    ws.on('error', reject);
    ws.on('open', () => ws.send(JSON.stringify(['REQ', 'bench', {}])));
    ws.on('message', (data) => {
      // Frames are told apart by their head, not parsed: with every
      // subscriber in this one process, parsing would be what is measured.
      const frame = String(data);
      if (frame.startsWith('["EVENT"')) onEvent();
      else if (frame.startsWith('["EOSE"')) resolve();
      else reject(new Error(`unexpected frame: ${frame.slice(0, 120)}`));
    });
  });
}

/**
 * One event at a time: POST it, and note when the last subscriber has it.
 * That moment, counted from the POST, is the event's fan-out, whether it
 * falls before or after the POST is answered.
 */
async function fanOut({ container, writeUrl, readUrl }, events, sockets) {
  let pending = 0;
  let reached = () => undefined;
  await pooled(Array.from({ length: SUBSCRIBERS }), 50, () =>
    subscribe(readUrl, sockets, () => {
      if (--pending === 0) reached(performance.now());
    })
  );
  await sleep(IDLE_SECONDS * 1000);
  const subscribedMiB = await memoryMiB(container);

  const times = [];
  for (const event of events) {
    const lastDelivery = new Promise((resolve) => (reached = resolve));
    pending = SUBSCRIBERS;
    const postedAt = performance.now();
    let timer;
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(
        () => reject(new Error(`fan-out: ${pending} deliveries missing`)),
        60_000
      );
    });
    const [reachedAt] = await Promise.race([
      Promise.all([lastDelivery, post(writeUrl, event)]),
      timeout,
    ]);
    clearTimeout(timer);
    times.push(reachedAt - postedAt);
  }
  times.sort((a, b) => a - b);
  return {
    subscribedMiB,
    fanOutP50Ms: percentile(times, 50),
    fanOutP99Ms: percentile(times, 99),
  };
}

async function measure(image, writeEvents, fanOutEvents) {
  const relay = await start(image);
  const sockets = [];
  try {
    await sleep(IDLE_SECONDS * 1000);
    const idleMiB = await memoryMiB(relay.container);
    const fanned = await fanOut(relay, fanOutEvents, sockets);
    for (const ws of sockets) ws.terminate();
    await sleep(1000);
    const written = await writes(relay, writeEvents);
    return { idleMiB, ...fanned, ...written };
  } finally {
    for (const ws of sockets) ws.terminate();
    await remove(relay.container);
  }
}

const METRICS = [
  ['idleMiB', 'memory at idle, no connections (MiB)'],
  ['subscribedMiB', `memory with ${SUBSCRIBERS} idle subscribers (MiB)`],
  ['fanOutP50Ms', 'one event to every subscriber p50 (ms)'],
  ['fanOutP99Ms', 'one event to every subscriber p99 (ms)'],
  ['writesPerSecond', 'attributed writes per second'],
  ['writeP50Ms', 'write latency p50 (ms)'],
  ['writeP99Ms', 'write latency p99 (ms)'],
];

const rounds = Object.fromEntries(IMAGES.map(({ name }) => [name, []]));
for (let round = 1; round <= ROUNDS; round++) {
  const order = round % 2 ? IMAGES : [...IMAGES].reverse();
  for (const { name, image } of order) {
    // Fresh events per run: a second delivery of a stored event is a
    // duplicate, which is a different code path from a write.
    const now = Math.floor(Date.now() / 1000);
    const fanOutEvents = signed(FANOUT_EVENTS, `fan-out ${round} ${name}`, now);
    const writeEvents = signed(WRITES, `write ${round} ${name}`, now);
    const result = await measure(image, writeEvents, fanOutEvents);
    rounds[name].push(result);
    console.error(`round ${round} ${name}: ${JSON.stringify(result)}`);
  }
}

const settings = {
  ROUNDS,
  IDLE_SECONDS,
  WRITES,
  WRITE_CONCURRENCY,
  SUBSCRIBERS,
  FANOUT_EVENTS,
  CPUS: CPUS ?? 'unlimited',
};
console.log(`settings: ${JSON.stringify(settings)}\n`);
console.log(
  `| median of ${ROUNDS} rounds | ${IMAGES.map((i) => i.name).join(' | ')} |`
);
console.log(`| --- | ${IMAGES.map(() => '---:').join(' | ')} |`);
for (const [key, label] of METRICS) {
  const cells = IMAGES.map(({ name }) =>
    median(rounds[name].map((r) => r[key])).toFixed(
      key.endsWith('PerSecond') ? 0 : 2
    )
  );
  console.log(`| ${label} | ${cells.join(' | ')} |`);
}
