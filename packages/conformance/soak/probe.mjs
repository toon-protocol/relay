// The differential probe: the same malformed and edge inputs to every image,
// and the inputs they answer differently (#203). The suite beside this
// directory says what a relay must do; this says where two relays differ,
// including on input the suite has no case for.
//
//   PROBE_IMAGES="typescript=<image>,rust=<image>" node soak/probe.mjs
//
// Each image is started alone and given the same stored events (one key, one
// timestamp, so ids are the same everywhere), then every input in turn: HTTP
// to the write port, HTTP to the read port, and one WebSocket per message
// list. The reads go to a second relay that holds only the stored events, so
// what the write inputs left behind is not compared.
//
// An answer is the status, every header not named in UNCOMPARED and the
// body, or the frames received until the socket has been quiet for QUIET_MS.
// What is expected to differ is taken out first:
//
//   - the version an image reports becomes `<version>`, and the time of a
//     write or of a /health answer `<time>`
//   - /metrics is reduced to its keys: its values are counters
//   - a JSON body's or frame's keys are sorted: their order is not part of
//     the contract
//   - `Connection` and `Keep-Alive` are not compared: the TypeScript relay
//     sends them on every response and the Rust relay on none
//
// An input after which the relay no longer answers /health is marked `the
// relay stopped`, and the inputs after it go to a new relay. Every differing
// input is printed in a Markdown table on stdout.
import { finalizeEvent } from 'nostr-tools/pure';
import WebSocket from 'ws';
import { imagesFrom, remove, start } from './relay-image.mjs';

const IMAGES = imagesFrom('PROBE_IMAGES');
const QUIET_MS = Number(process.env.QUIET_MS ?? 400);
// Headers that say nothing about the answer, or differ on every response.
const UNCOMPARED = new Set([
  'date',
  'content-length',
  'transfer-encoding',
  'connection',
  'keep-alive',
]);

const SECRET_KEY = new Uint8Array(32).fill(7);
const NOW = Math.floor(Date.now() / 1000);
const event = (kind, content, tags = [], created_at = NOW) =>
  finalizeEvent({ kind, created_at, tags, content }, SECRET_KEY);

// No two share a created_at: which of two such events comes first is a
// difference the suite has a case for, and here it would move every answer.
const STORED = [
  event(1, 'first', [['t', 'soak']], NOW - 30),
  event(1, 'second', [['t', 'Soak']], NOW - 20),
  event(1, 'third', [['long', 'value']], NOW - 10),
  event(0, '{"name":"probe"}', [], NOW - 5),
  event(30023, 'article', [['d', 'a_b']]),
];
const [first] = STORED;
const fresh = event(1, 'fresh');
const ATTRIBUTION = {
  'X-TOON-Payer': `evm:0x${'ab'.repeat(32)}`,
  'X-TOON-Amount': '1',
  'X-TOON-Chain': 'evm',
};
const json = { 'content-type': 'application/json' };
const write = (body, headers = {}, path = '/write') => ({
  method: 'POST',
  path,
  headers: { ...json, ...headers },
  body: typeof body === 'string' ? body : JSON.stringify(body),
});

/** [label, request] for the write port. */
const WRITE_PORT = [
  ['GET /health', { path: '/health' }],
  ['GET /metrics', { path: '/metrics', shape: true }],
  ['GET /write', { path: '/write' }],
  ['PUT /write', { method: 'PUT', path: '/write', body: '{}', headers: json }],
  ['DELETE /write', { method: 'DELETE', path: '/write' }],
  ['OPTIONS /write', { method: 'OPTIONS', path: '/write' }],
  ['GET /', { path: '/' }],
  ['GET /unknown', { path: '/unknown' }],
  ['POST /handle-packet', write({}, {}, '/handle-packet')],
  ['write: attributed', write({ event: fresh }, ATTRIBUTION)],
  ['write: the same event again', write({ event: fresh }, ATTRIBUTION)],
  ['write: no attribution', write({ event: event(1, 'bare') })],
  [
    'write: payer only',
    write(
      { event: event(1, 'payer only') },
      { 'X-TOON-Payer': ATTRIBUTION['X-TOON-Payer'] }
    ),
  ],
  [
    'write: amount not a number',
    write(
      { event: event(1, 'bad amount') },
      { ...ATTRIBUTION, 'X-TOON-Amount': 'lots' }
    ),
  ],
  ['write: wrong id', write({ event: { ...fresh, id: '0'.repeat(64) } })],
  ['write: wrong sig', write({ event: { ...fresh, sig: '0'.repeat(128) } })],
  ['write: short id', write({ event: { ...fresh, id: 'abcd' } })],
  ['write: no sig', write({ event: { ...fresh, sig: undefined } })],
  ['write: kind as a string', write({ event: { ...fresh, kind: '1' } })],
  ['write: tags not a list', write({ event: { ...fresh, tags: 'x' } })],
  ['write: event is a string', write({ event: 'x' })],
  ['write: empty object', write({})],
  ['write: a list', write([])],
  ['write: not JSON', write('{not json')],
  ['write: empty body', write('')],
  [
    'write: text/plain',
    write({ event: event(1, 'as text') }, { 'content-type': 'text/plain' }),
  ],
  ['write: ephemeral kind', write({ event: event(20100, 'ephemeral') })],
  [
    'write: kind 5 of another author',
    write({ event: event(5, '', [['e', '1'.repeat(64)]]) }),
  ],
  [
    'write: created_at far ahead',
    write({ event: event(1, 'ahead', [], NOW + 10 ** 7) }),
  ],
  [
    'write: expired on arrival',
    write({ event: event(1, 'gone', [['expiration', String(NOW - 5)]]) }),
  ],
  [
    'write: 300 KiB content',
    write({ event: event(1, 'x'.repeat(300 * 1024)) }),
  ],
  [
    'ephemeral: ephemeral kind',
    write({ event: event(20100, 'e2') }, {}, '/write-ephemeral'),
  ],
  [
    'ephemeral: stored kind',
    write({ event: event(1, 'e3') }, {}, '/write-ephemeral'),
  ],
  ['ephemeral: empty object', write({}, {}, '/write-ephemeral')],
  ['GET /write-ephemeral', { path: '/write-ephemeral' }],
];

const nostrJson = { accept: 'application/nostr+json' };
/** [label, request] for the read port. */
const READ_PORT = [
  ['document', { path: '/', headers: nostrJson }],
  ['GET / with no Accept', { path: '/' }],
  ['OPTIONS /', { method: 'OPTIONS', path: '/' }],
  [
    'HEAD / for the document',
    { method: 'HEAD', path: '/', headers: nostrJson },
  ],
  ['POST /', { method: 'POST', path: '/', headers: json, body: '{}' }],
  ['GET /unknown, document', { path: '/unknown', headers: nostrJson }],
  ['GET /unknown', { path: '/unknown' }],
  ['GET /health on the read port', { path: '/health' }],
];

const req = (...filters) => JSON.stringify(['REQ', 'a', ...filters]);
const author = first.pubkey;
/** [label, messages sent in order on one WebSocket]. */
const WEBSOCKET = [
  ['not JSON', ['{not json']],
  ['an object', ['{"a":1}']],
  ['an empty list', ['[]']],
  ['a number', ['7']],
  ['an empty message', ['']],
  ['a binary frame', [Buffer.from([1, 2, 3])]],
  ['unknown verb', ['["BOGUS","x"]']],
  ['a lower-case verb', ['["req","a",{}]']],
  ['REQ alone', ['["REQ"]']],
  ['REQ with no filter', ['["REQ","a"]']],
  ['REQ with a numeric filter', [req(1)]],
  ['REQ with a list as filter', [req([])]],
  ['REQ with a numeric id', ['["REQ",7,{}]']],
  ['REQ with an empty id', ['["REQ","",{}]']],
  ['REQ with a 65-character id', [JSON.stringify(['REQ', 'x'.repeat(65), {}])]],
  ['REQ {}', [req({})]],
  ['REQ limit 0', [req({ limit: 0 })]],
  ['REQ limit 1', [req({ limit: 1 })]],
  ['REQ limit -1', [req({ limit: -1 })]],
  ['REQ limit 1.5', [req({ limit: 1.5 })]],
  ['REQ limit as a string', [req({ limit: '1' })]],
  ['REQ limit 100000', [req({ kinds: [0], limit: 100000 })]],
  ['REQ kinds ["x"]', [req({ kinds: ['x'] })]],
  ['REQ kinds [-1]', [req({ kinds: [-1] })]],
  ['REQ kinds [70000]', [req({ kinds: [70000] })]],
  ['REQ kinds []', [req({ kinds: [] })]],
  ['REQ ids []', [req({ ids: [] })]],
  ['REQ ids prefix', [req({ ids: [first.id.slice(0, 8)] })]],
  ['REQ ids not hex', [req({ ids: ['z'.repeat(64)] })]],
  ['REQ ids upper case', [req({ ids: [first.id.toUpperCase()] })]],
  ['REQ authors prefix', [req({ authors: [author.slice(0, 8)] })]],
  ['REQ authors upper case', [req({ authors: [author.toUpperCase()] })]],
  ['REQ since after until', [req({ since: NOW, until: NOW - 100 })]],
  ['REQ since as a string', [req({ since: 'yesterday' })]],
  ['REQ until -1', [req({ until: -1 })]],
  ['REQ #t', [req({ '#t': ['soak'] })]],
  ['REQ #t with a wildcard', [req({ '#t': ['so%'] })]],
  ['REQ #t not a list', [req({ '#t': 'soak' })]],
  ['REQ #t []', [req({ '#t': [] })]],
  ['REQ #long', [req({ '#long': ['value'] })]],
  ['REQ #e not hex', [req({ '#e': ['nope'] })]],
  ['REQ #d with a wildcard', [req({ kinds: [30023], '#d': ['a%b'] })]],
  ['REQ unknown filter key', [req({ kinds: [0], bogus: true })]],
  ['REQ search', [req({ kinds: [0], search: 'probe' })]],
  ['REQ two filters', [req({ kinds: [0] }, { kinds: [30023] })]],
  ['REQ two filters, one invalid', [req({ kinds: [0] }, { limit: -1 })]],
  ['REQ twice on one id', [req({ kinds: [0] }), req({ kinds: [30023] })]],
  ['REQ then CLOSE', [req({ kinds: [0] }), '["CLOSE","a"]']],
  ['CLOSE an unknown id', ['["CLOSE","nope"]']],
  ['CLOSE alone', ['["CLOSE"]']],
  ['CLOSE with a numeric id', ['["CLOSE",7]']],
  [
    '21 subscriptions',
    Array.from({ length: 21 }, (_, i) =>
      JSON.stringify(['REQ', `s${i}`, { kinds: [0] }])
    ),
  ],
  ['EVENT', [JSON.stringify(['EVENT', event(1, 'over the socket')])]],
  ['EVENT alone', ['["EVENT"]']],
  ['EVENT with an object that is no event', ['["EVENT",{}]']],
  ['AUTH with an event', [JSON.stringify(['AUTH', event(22242, '')])]],
  ['AUTH with a string', ['["AUTH","challenge"]']],
  ['COUNT', ['["COUNT","c",{"kinds":[1]}]']],
  ['COUNT with no filter', ['["COUNT","c"]']],
  ['NEG-OPEN', ['["NEG-OPEN","n",{"kinds":[1]},"6100000200"]']],
  ['NEG-MSG with nothing open', ['["NEG-MSG","n","6100000200"]']],
  ['NEG-CLOSE with nothing open', ['["NEG-CLOSE","n"]']],
  ['a 200 KiB message', [req({ ids: Array(3000).fill('0'.repeat(64)) })]],
  ['a 2 MiB message', [req({ search: 'x'.repeat(2 * 2 ** 20) })]],
];

/** A JSON value reduced to its sorted key paths. */
function shapeOf(value, path = '') {
  if (Array.isArray(value)) return [`${path}[]`];
  if (value === null || typeof value !== 'object') return [path];
  return Object.keys(value)
    .sort()
    .flatMap((key) => shapeOf(value[key], path ? `${path}.${key}` : key));
}

async function http(base, { method = 'GET', path, headers, body, shape }) {
  const response = await fetch(base + path, { method, headers, body });
  const text = await response.text();
  const kept = [...response.headers]
    .filter(([name]) => !UNCOMPARED.has(name))
    .map(([name, value]) => `${name}: ${value}`)
    .sort();
  let shown = keysSorted(text);
  if (shape) {
    try {
      shown = `keys: ${shapeOf(JSON.parse(text)).join(' ')}`;
    } catch {
      // Not JSON: the body as it is.
    }
  }
  return [String(response.status), ...kept, shown].filter(Boolean).join('\n');
}

/** JSON text with its objects' keys in order; any other text as it is. */
function keysSorted(text) {
  const sorted = (value) =>
    Array.isArray(value)
      ? value.map(sorted)
      : value !== null && typeof value === 'object'
        ? Object.fromEntries(
            Object.keys(value)
              .sort()
              .map((key) => [key, sorted(value[key])])
          )
        : value;
  try {
    return JSON.stringify(sorted(JSON.parse(text)));
  } catch {
    return text;
  }
}

/** Everything one socket hears after `messages`, until it falls quiet. */
function websocket(readUrl, messages) {
  return new Promise((resolve) => {
    const heard = [];
    const ws = new WebSocket(readUrl);
    let quiet;
    const done = () => {
      clearTimeout(quiet);
      ws.terminate();
      resolve(heard.join('\n') || '(nothing)');
    };
    const wait = () => {
      clearTimeout(quiet);
      quiet = setTimeout(done, QUIET_MS);
    };
    ws.on('open', () => {
      for (const message of messages) ws.send(message);
      wait();
    });
    ws.on('message', (data) => {
      heard.push(keysSorted(String(data)));
      wait();
    });
    ws.on('close', (code, reason) => {
      heard.push(`closed ${code} ${reason}`.trim());
      done();
    });
    ws.on('error', (error) => {
      heard.push(`error: ${error.message}`);
      done();
    });
  });
}

/** A relay on `image` that holds STORED, with the version it reports. */
async function seeded(image) {
  const relay = await start(image);
  try {
    const health = await (await fetch(`${relay.writeUrl}/health`)).json();
    for (const stored of STORED) {
      const response = await fetch(`${relay.writeUrl}/write`, {
        method: 'POST',
        headers: json,
        body: JSON.stringify({ event: stored }),
      });
      if (response.status !== 200)
        throw new Error(`${image} refused a stored event: ${response.status}`);
    }
    if (typeof health.version !== 'string')
      throw new Error(`${image} reports no version on /health`);
    return { ...relay, version: health.version };
  } catch (error) {
    await remove(relay.container);
    throw error;
  }
}

async function probe(image) {
  let relay = await seeded(image);
  const answers = new Map();
  const answer = async (label, ask) => {
    let text = await ask().catch((error) => `error: ${error.message}`);
    const alive = await fetch(`${relay.writeUrl}/health`).then(
      (r) => r.ok,
      () => false
    );
    // An input that stops the relay is a finding; the rest go to a new one.
    if (!alive) {
      text += '\nthe relay stopped';
      await remove(relay.container);
      relay = await seeded(image);
    }
    answers.set(
      label,
      text
        .replaceAll(relay.version, '<version>')
        .replace(/"(storedAt|broadcastAt|timestamp)":\d+/g, '"$1":<time>')
    );
  };
  try {
    for (const [label, request] of WRITE_PORT)
      await answer(`write port: ${label}`, () => http(relay.writeUrl, request));
    await remove(relay.container);
    relay = await seeded(image);
    for (const [label, request] of READ_PORT)
      await answer(`read port: ${label}`, () =>
        http(relay.readHttpUrl, request)
      );
    for (const [label, messages] of WEBSOCKET)
      await answer(`WebSocket: ${label}`, () =>
        websocket(relay.readUrl, messages)
      );
    return answers;
  } finally {
    await remove(relay.container);
  }
}

const answers = [];
for (const { name, image } of IMAGES) {
  answers.push(await probe(image));
  console.error(`probed ${name}`);
}

const labels = [...answers[0].keys()];
const differing = labels.filter((label) =>
  answers.some((image) => image.get(label) !== answers[0].get(label))
);

/** How many leading characters every one of `texts` shares. */
function sharedPrefix(texts) {
  let length = 0;
  while (
    texts.every(
      (text) => length < text.length && text[length] === texts[0][length]
    )
  )
    length++;
  return length;
}

const CELL = 400;
/** One answer as a table cell, cut to CELL characters from `from`. */
function cell(text, from) {
  const cut =
    (from > 0 ? '…' : '') +
    text.slice(from, from + CELL) +
    (text.length > from + CELL ? `… (${text.length})` : '');
  return cut
    .split('\n')
    .filter(Boolean)
    .map((line) => `\`${line.replaceAll('|', '\\|').replaceAll('`', "'")}\``)
    .join('<br>');
}

console.log(
  `${labels.length} inputs, ${labels.length - differing.length} answered the same by every image, ${differing.length} not:\n`
);
console.log(`| input | ${IMAGES.map((i) => i.name).join(' | ')} |`);
console.log(`| --- | ${IMAGES.map(() => '---').join(' | ')} |`);
for (const label of differing) {
  const texts = answers.map((image) => image.get(label));
  // A long answer is shown from just before where the images part.
  const shared = sharedPrefix(texts);
  const from = shared > CELL / 2 ? shared - CELL / 4 : 0;
  console.log(
    `| ${label} | ${texts.map((text) => cell(text, from)).join(' | ')} |`
  );
}
