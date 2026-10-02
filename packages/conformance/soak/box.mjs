// A running relay node from outside, before and after its relay image is
// swapped over the same data (#204). It reads the node's two public
// hostnames and nothing else: no ssh, no payment, nothing written.
//
//   node soak/box.mjs baseline > baseline.json     before the swap
//   EXPECT_VERSION=<version> node soak/box.mjs check baseline.json
//
// `baseline` records the Relay Information Document and every stored event
// (the newest SAMPLE of them, if that is set). `check` exits 1 unless
//
//   - a plain GET on the read host answers 426, and the connector's
//     /ilp/identity 200: the two answers the fleet's health check asks the
//     relay node for
//   - the document is the baseline's, apart from what is documented to differ
//     between the images: `version`, and `limitation.max_limit` and
//     `default_limit` (#233)
//   - an EVENT over WebSocket is refused with the document's ILP address
//   - every baseline event is still served, as it was. One that is not is
//     accounted for only if it has expired (NIP-40), a newer event has taken
//     its address, or its author deleted it (NIP-09)
import { readFileSync } from 'node:fs';
import { isDeepStrictEqual } from 'node:util';
import { finalizeEvent, generateSecretKey } from 'nostr-tools/pure';
import WebSocket from 'ws';

const READ_URL =
  process.env.READ_URL ?? 'https://relay-ws.devnet.toonprotocol.dev';
const EDGE_URL =
  process.env.EDGE_URL ?? 'https://proxy.relay.devnet.toonprotocol.dev';
const SAMPLE = Number(process.env.SAMPLE ?? Infinity);
if (!(SAMPLE >= 1)) {
  console.error('SAMPLE must be a number of events, 1 or more');
  process.exit(2);
}
// The Rust relay answers one filter with at most 500 events (#233).
const PAGE = 500;
const TIMEOUT_MS = 30_000;

/** One WebSocket, one message, and the frames up to the one `last` accepts. */
function exchange(message, last) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(READ_URL.replace(/^http/, 'ws'));
    const frames = [];
    const timer = setTimeout(() => {
      ws.terminate();
      reject(new Error(`no answer to ${message[0]} in ${TIMEOUT_MS} ms`));
    }, TIMEOUT_MS);
    ws.on('open', () => ws.send(JSON.stringify(message)));
    ws.on('message', (data) => {
      const frame = JSON.parse(String(data));
      frames.push(frame);
      if (!last(frame)) return;
      clearTimeout(timer);
      ws.close();
      resolve(frames);
    });
    ws.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    // After the answer this rejects a promise already settled, which is nothing.
    ws.on('close', () => {
      clearTimeout(timer);
      reject(new Error(`closed before an answer to ${message[0]}`));
    });
  });
}

/** The stored events one filter is answered with. */
async function stored(filter) {
  const frames = await exchange(['REQ', 'box', filter], ([type]) =>
    ['EOSE', 'CLOSED', 'NOTICE'].includes(type)
  );
  const end = frames.at(-1);
  if (end[0] !== 'EOSE') throw new Error(`REQ answered ${JSON.stringify(end)}`);
  return frames.filter(([type]) => type === 'EVENT').map((frame) => frame[2]);
}

async function document() {
  const response = await fetch(READ_URL, {
    headers: { accept: 'application/nostr+json' },
  });
  return { status: response.status, body: await response.json() };
}

/** The stored events by id, newest first, paged back by `until` to SAMPLE or so. */
async function newest() {
  const events = new Map();
  let until;
  while (events.size < SAMPLE) {
    const page = await stored({ limit: PAGE, ...(until && { until }) });
    const before = events.size;
    for (const event of page) events.set(event.id, event);
    // `until` is inclusive, so a page starts with events already held, and
    // one that adds nothing is the end of the store. Unless it is full: then
    // one second holds more events than a page, and the rest of that second
    // cannot be reached.
    const oldest = Math.min(...page.map((event) => event.created_at));
    if (events.size > before) until = oldest;
    else if (page.length < PAGE) break;
    else {
      console.error(
        `more than ${PAGE} events at ${oldest}: the rest of that second is skipped`
      );
      until = oldest - 1;
    }
  }
  return events;
}

const tag = (event, name) => event.tags.find(([key]) => key === name)?.[1];

// The relay's own replacement rule (`rule` in crates/relay/src/store.rs):
// addressable by `d`, the TOON range included, and one per author and kind.
const addressable = ({ kind }) =>
  (kind >= 30000 && kind < 40000) || (kind >= 10032 && kind <= 10099);
const replaceable = (event) =>
  addressable(event) ||
  event.kind === 0 ||
  event.kind === 3 ||
  (event.kind >= 10000 && event.kind < 20000);

/**
 * Why an event the baseline holds is no longer served, if there is a reason.
 * `held` is the baseline's ids: an event the baseline already held replaced
 * nothing since.
 */
async function gone(event, held) {
  const { kind, pubkey } = event;
  const expiration = tag(event, 'expiration');
  if (/^\d+$/.test(expiration ?? '') && Number(expiration) <= Date.now() / 1000)
    return 'expired';
  const d = addressable(event) ? (tag(event, 'd') ?? '') : undefined;
  if (replaceable(event)) {
    const others = await stored({ kinds: [kind], authors: [pubkey] });
    const newer = others.some(
      (other) =>
        !held.has(other.id) &&
        other.created_at >= event.created_at &&
        (d === undefined || (tag(other, 'd') ?? '') === d)
    );
    if (newer) return 'replaced';
  }
  const deletion = { kinds: [5], authors: [pubkey] };
  const deletions = [
    ...(await stored({ ...deletion, '#e': [event.id] })),
    ...(d === undefined
      ? []
      : await stored({ ...deletion, '#a': [`${kind}:${pubkey}:${d}`] })),
  ];
  return deletions.length > 0 ? 'deleted' : undefined;
}

async function baseline() {
  const { status, body } = await document();
  if (status !== 200) throw new Error(`the document answered ${status}`);
  const events = [...(await newest()).values()];
  console.error(`${events.length} events, version ${body.version}`);
  console.log(
    JSON.stringify({ at: new Date().toISOString(), document: body, events })
  );
}

async function check(path) {
  const before = JSON.parse(readFileSync(path, 'utf8'));
  let failed = false;
  const report = (ok, line) => {
    failed ||= !ok;
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${line}`);
  };

  const plain = await fetch(READ_URL);
  report(
    plain.status === 426,
    `GET ${READ_URL}/ answers ${plain.status}, wanted 426`
  );
  const identity = await fetch(`${EDGE_URL}/ilp/identity`);
  report(
    identity.status === 200,
    `GET ${EDGE_URL}/ilp/identity answers ${identity.status}, wanted 200`
  );

  const { status, body } = await document();
  report(status === 200, `the document answers ${status}`);
  const expected = process.env.EXPECT_VERSION;
  report(
    !expected || body.version === expected,
    `version ${body.version}${expected ? `, wanted ${expected}` : ''}`
  );
  const comparable = ({ version, limitation, ...rest }) => {
    const { max_limit, default_limit, ...limits } = limitation ?? {};
    return { ...rest, limitation: limits };
  };
  const same = isDeepStrictEqual(comparable(body), comparable(before.document));
  report(
    same,
    `the document is the baseline's, apart from version and the limits`
  );
  if (!same) {
    console.log(`     baseline: ${JSON.stringify(before.document)}`);
    console.log(`     now:      ${JSON.stringify(body)}`);
  }

  const unpaid = finalizeEvent(
    {
      kind: 1,
      created_at: Math.floor(Date.now() / 1000),
      tags: [],
      content: '',
    },
    generateSecretKey()
  );
  const refusal = (
    await exchange(['EVENT', unpaid], ([type]) =>
      ['OK', 'NOTICE'].includes(type)
    )
  ).at(-1);
  report(
    refusal[0] === 'OK' &&
      refusal[2] === false &&
      String(refusal[3]).includes(body.toon?.ilp_address),
    `EVENT over WebSocket is answered ${JSON.stringify(refusal.slice(2))}`
  );

  // The walk the baseline took, then by id whatever it did not reach. By id
  // alone, the TypeScript relay takes seconds over every page.
  const served = await newest();
  const unreached = before.events.filter((event) => !served.has(event.id));
  for (let i = 0; i < unreached.length; i += PAGE) {
    const ids = unreached.slice(i, i + PAGE).map((event) => event.id);
    for (const event of await stored({ ids })) served.set(event.id, event);
  }
  const held = new Set(before.events.map((event) => event.id));
  const reasons = {};
  const lost = [];
  let unchanged = 0;
  for (const event of before.events) {
    if (isDeepStrictEqual(served.get(event.id), event)) {
      unchanged += 1;
    } else if (served.has(event.id)) {
      lost.push(`${event.id} is served differently`);
    } else {
      const reason = await gone(event, held);
      if (reason) reasons[reason] = (reasons[reason] ?? 0) + 1;
      else lost.push(`${event.id} (kind ${event.kind}) is not served`);
    }
  }
  const accounted = Object.entries(reasons)
    .map(([reason, count]) => `, ${count} ${reason}`)
    .join('');
  report(
    lost.length === 0,
    `${unchanged} of the ${before.events.length} events of ${before.at} are served as they were${accounted}`
  );
  for (const line of lost) console.log(`     ${line}`);

  process.exitCode = failed ? 1 : 0;
}

const [command, path] = process.argv.slice(2);
if (command === 'baseline') await baseline();
else if (command === 'check' && path) await check(path);
else {
  console.error('usage: box.mjs baseline > <file>  |  box.mjs check <file>');
  process.exit(2);
}
