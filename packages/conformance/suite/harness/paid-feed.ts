import { createHash } from 'node:crypto';
import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { Event } from 'nostr-tools';
import { ilpDocument, STUB_ILP_ADDRESS, STUB_PRICE } from './stub-connector.js';
import {
  startRelay,
  type Env,
  type RunningRelay,
  type StartOptions,
} from './relay-container.js';
import { now } from './wire.js';
import { Client, sleep, type Frame } from './client.js';

/** The route a subscribe packet is paid at, and what the stub charges. */
export const SUBSCRIBE_ADDRESS = `${STUB_ILP_ADDRESS}.subscribe`;
export const SUBSCRIBE_PRICE = 1000;
/** What the relay debits for each event it broadcasts. */
export const BROADCAST_PRICE = 10;
/** The URL the relay is told it is reached at (NIP-42 `relay`, NIP-98 `u`). */
export const RELAY_URL = 'wss://relay.conformance.test';
export const RELAY_HTTP_URL = 'https://relay.conformance.test/';

/** The stub's self-description with a subscribe route beside the write route. */
export function sellingDocument(
  subscribe: Record<string, unknown> = {}
): Record<string, unknown> {
  return ilpDocument({
    routes: [
      { prefix: STUB_ILP_ADDRESS, price: STUB_PRICE },
      {
        prefix: SUBSCRIBE_ADDRESS,
        price: String(SUBSCRIBE_PRICE),
        ...subscribe,
      },
    ],
  });
}

/** The environment that makes a relay sell its live feed. */
export const SELLING_ENV: Env = {
  TOON_SUBSCRIBE_ILP_ADDRESS: SUBSCRIBE_ADDRESS,
  TOON_BROADCAST_PRICE: String(BROADCAST_PRICE),
  TOON_RELAY_URL: RELAY_URL,
};

/** Start a relay that sells its feed, with `options` on top. */
export function startSelling(
  image: string,
  options: StartOptions = {}
): Promise<RunningRelay> {
  return startRelay(image, {
    document: sellingDocument(),
    ...options,
    env: { ...SELLING_ENV, ...options.env },
  });
}

const sha256Hex = (body: string): string =>
  createHash('sha256').update(body).digest('hex');

export interface Nip98Options {
  method?: string;
  url?: string;
  /** The body the `payload` tag hashes; `undefined` for no `payload` tag. */
  body?: string;
  createdAt?: number;
  /** Replace the `payload` tag's hash. */
  payload?: string;
}

/** The `Authorization` header value of a NIP-98 event signed by `secretKey`. */
export function nip98(
  secretKey: Uint8Array,
  options: Nip98Options = {}
): string {
  const method = options.method ?? 'POST';
  const tags = [
    ['u', options.url ?? RELAY_HTTP_URL],
    ['method', method],
  ];
  const payload =
    options.payload ??
    (options.body === undefined ? undefined : sha256Hex(options.body));
  if (payload !== undefined) tags.push(['payload', payload]);
  const event = finalizeEvent(
    {
      kind: 27235,
      created_at: options.createdAt ?? now(),
      tags,
      content: '',
    },
    secretKey
  );
  return `Nostr ${Buffer.from(JSON.stringify(event)).toString('base64')}`;
}

export interface Payment {
  /** The JSON body, or a string sent as it is. */
  body?: unknown;
  /** `Authorization` header; default a fresh, valid NIP-98 for the body. */
  authorization?: string | null;
  /** Headers the connector states on a delivery. */
  amount?: string | null;
  /** Statement of a payer, which the relay must not use. */
  payer?: string;
}

export interface Answer {
  status: number;
  body: Record<string, unknown>;
}

/**
 * Deliver a subscribe packet the way the relay's connector does: a `POST` to
 * the route's handler on the write port, with the request the subscriber put
 * in the packet and the statement of what the route charged.
 */
export async function pay(
  relay: RunningRelay,
  secretKey: Uint8Array,
  payment: Payment = {}
): Promise<Answer> {
  const body =
    typeof payment.body === 'string'
      ? payment.body
      : JSON.stringify(payment.body ?? {});
  const authorization =
    payment.authorization === undefined
      ? nip98(secretKey, { body })
      : payment.authorization;
  const amount =
    payment.amount === undefined ? String(SUBSCRIBE_PRICE) : payment.amount;
  const headers: Record<string, string> = {
    'content-type': 'application/json',
  };
  if (authorization !== null) headers['authorization'] = authorization;
  if (amount !== null) {
    headers['x-toon-amount'] = amount;
    headers['x-toon-payer'] = payment.payer ?? `evm:0x${'ab'.repeat(32)}`;
    headers['x-toon-chain'] = 'evm';
  }
  const response = await fetch(`${relay.writeUrl}/subscribe`, {
    method: 'POST',
    headers,
    body,
  });
  return {
    status: response.status,
    body: (await response.json()) as Record<string, unknown>,
  };
}

/** Read a subscription's balance over HTTP on the read port. */
export async function readBalance(
  relay: RunningRelay,
  authorization: string | null
): Promise<Answer & { contentType: string | null }> {
  const headers: Record<string, string> = {
    accept: 'application/toon-subscription+json',
  };
  if (authorization !== null) headers['authorization'] = authorization;
  const response = await fetch(relay.readUrl, { headers });
  return {
    status: response.status,
    contentType: response.headers.get('content-type'),
    body: (await response.json()) as Record<string, unknown>,
  };
}

/** The balance a key holds now, as the subscriber reads it. */
export async function balanceOf(
  relay: RunningRelay,
  secretKey: Uint8Array
): Promise<number> {
  const answer = await readBalance(relay, nip98(secretKey, { method: 'GET' }));
  if (answer.status !== 200) {
    throw new Error(`balance read answered ${answer.status}`);
  }
  return answer.body['balance'] as number;
}

/** The NIP-42 `AUTH` message answering `challenge`, signed by `secretKey`. */
export function authMessage(
  secretKey: Uint8Array,
  challenge: string,
  relayUrl = RELAY_URL
): ['AUTH', Event] {
  return [
    'AUTH',
    finalizeEvent(
      {
        kind: 22242,
        created_at: now(),
        tags: [
          ['relay', relayUrl],
          ['challenge', challenge],
        ],
        content: '',
      },
      secretKey
    ),
  ];
}

/** Connect and wait for the relay's `AUTH` challenge. */
export async function connectAndChallenge(
  relay: RunningRelay
): Promise<{ client: Client; challenge: string }> {
  const client = await Client.connect(relay.readWsUrl);
  const frame = await client.next((f) => f[0] === 'AUTH');
  return { client, challenge: String(frame[1]) };
}

/** Connect, authenticate as `secretKey`, and require the relay accepts it. */
export async function connectAs(
  relay: RunningRelay,
  secretKey: Uint8Array
): Promise<Client> {
  const { client, challenge } = await connectAndChallenge(relay);
  const message = authMessage(secretKey, challenge);
  client.send(message);
  const ok = await client.next((f) => f[0] === 'OK' && f[1] === message[1].id);
  if (ok[2] !== true) throw new Error(`AUTH refused: ${JSON.stringify(ok)}`);
  return client;
}

/** The frames of `kind` addressed to `subscription`, still unread. */
export async function framesFor(
  client: Client,
  subscription: string,
  ms = 750
): Promise<Frame[]> {
  await sleep(ms);
  return (await client.quiet(0)).filter((f) => f[1] === subscription);
}

export const publicKey = (secretKey: Uint8Array): string =>
  getPublicKey(secretKey);
