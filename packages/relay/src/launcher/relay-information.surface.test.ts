/**
 * The NIP-11 surface on a REAL relay, over a real socket.
 *
 * What is worth booting a process for, rather than unit-testing:
 *
 *   - a `GET` carrying `Accept: application/nostr+json` on the read port
 *     answers the document, which is the acceptance criterion of
 *     TOON_Network#121 and the thing the live devnet relay cannot do today;
 *   - a normal client still upgrades to a WebSocket and reads, because the
 *     read port stopped belonging to the `ws` library to make this possible
 *     and that swap is exactly the sort of change that silently breaks the
 *     handshake;
 *   - every other plain HTTP request still answers `426 Upgrade Required`,
 *     byte for byte, so nothing that works against a relay today changes;
 *   - the document's edge is the CONNECTOR's own words, fetched from a
 *     connector standing in for the real one.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { createServer } from 'node:http';
import type { Server } from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket } from 'ws';
import { generateSecretKey } from 'nostr-tools/pure';
import { startRelay } from './relay.js';
import type { RelayInstance } from './relay.js';
import { InMemoryEventStore } from '../storage/index.js';
import type { RelayInformationDocument } from '../nips/relay-information.js';

let portCursor = 18450;
let relayPort = portCursor;
let blsPort = portCursor + 1;

let instance: RelayInstance | undefined;
let connector: Server | undefined;
let dataDir: string | undefined;

afterEach(async () => {
  if (instance) {
    await instance.stop();
    instance = undefined;
  }
  if (connector) {
    const server = connector;
    connector = undefined;
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
  if (dataDir) {
    rmSync(dataDir, { recursive: true, force: true });
    dataDir = undefined;
  }
});

/** A stand-in for the connector in front of the relay, serving `GET /ilp`. */
async function startConnector(body: unknown): Promise<string> {
  const server = createServer((_request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify(body));
  });
  connector = server;
  const port = await new Promise<number>((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      resolve(typeof address === 'object' && address ? address.port : 0);
    });
  });
  return `http://127.0.0.1:${port}/ilp`;
}

const SELF_DESCRIPTION = {
  ilpAddresses: ['g.toon.relay', 'g.toon.relay.ephemeral'],
  httpEndpoint: 'https://proxy.relay.example/ilp',
  btpEndpoint: 'wss://proxy.relay.example/ilp/btp',
  edgeIdentity: { keyId: 'connector-signer', publicKey: '0x04915d2990' },
  settlements: [{ chain: 'solana', tokenAddress: '34eSxY7', decimals: 6 }],
  routes: [
    { prefix: 'g.toon.relay', price: '1', requiredTransport: 'btp' },
    { prefix: 'g.toon.relay.ephemeral', price: '0' },
  ],
};

async function boot(
  extra: Record<string, unknown> = {}
): Promise<RelayInstance> {
  portCursor += 2;
  relayPort = portCursor;
  blsPort = portCursor + 1;
  dataDir = mkdtempSync(join(tmpdir(), 'relay-nip11-'));
  return startRelay({
    secretKey: generateSecretKey(),
    relayPort,
    blsPort,
    dataDir,
    eventStore: new InMemoryEventStore(),
    ...extra,
  });
}

/** Wait until the background reader has the edge (its first read is immediate). */
async function waitForEdge(node: RelayInstance): Promise<void> {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (node.relayInformation().toon !== undefined) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error('the relay never read its write edge');
}

function readDocument(): Promise<Response> {
  return fetch(`http://127.0.0.1:${relayPort}/`, {
    headers: { accept: 'application/nostr+json' },
  });
}

describe('the relay information document, served', () => {
  it('answers a NIP-11 request with its paid write edge, in its connector’s words', async () => {
    const connectorUrl = await startConnector(SELF_DESCRIPTION);
    instance = await boot({ connectorUrl, writeIlpAddress: 'g.toon.relay' });
    await waitForEdge(instance);

    const response = await readDocument();
    expect(response.status).toBe(200);
    expect(response.headers.get('content-type')).toBe('application/nostr+json');
    // NIP-11 is read from browsers; the document is free and identical for
    // everyone, so there is nothing for an origin check to protect.
    expect(response.headers.get('access-control-allow-origin')).toBe('*');

    const document = (await response.json()) as RelayInformationDocument;

    expect(document.toon).toEqual({
      ilp_address: 'g.toon.relay',
      // The URL the connector advertises, NOT the loopback address the relay
      // dialled to ask it.
      connector_url: 'https://proxy.relay.example/ilp',
      connector_seal_key: '0x04915d2990',
      carriage: 'btp',
      price: 1,
      settlement: [{ chain: 'solana', token: '34eSxY7', decimals: 6 }],
    });
    expect(document.limitation.payment_required).toBe(true);
    expect(document.fees).toEqual({
      publication: [{ amount: 1, unit: 'uusdc' }],
    });
    expect(document.pubkey).toBe(instance.pubkey);
    expect(document.supported_nips).toContain(11);
  });

  it('still upgrades a normal client, and still serves its reads', async () => {
    const connectorUrl = await startConnector(SELF_DESCRIPTION);
    instance = await boot({ connectorUrl, writeIlpAddress: 'g.toon.relay' });

    const ws = new WebSocket(`ws://127.0.0.1:${relayPort}`);
    await new Promise<void>((resolve, reject) => {
      ws.once('open', () => resolve());
      ws.once('error', reject);
    });

    const eose = new Promise<unknown[]>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('no EOSE')), 5000);
      ws.on('message', (data: Buffer) => {
        const parsed: unknown = JSON.parse(data.toString());
        if (Array.isArray(parsed) && parsed[0] === 'EOSE') {
          clearTimeout(timer);
          resolve(parsed);
        }
      });
    });
    ws.send(JSON.stringify(['REQ', 'sub1', { kinds: [1] }]));
    expect(await eose).toEqual(['EOSE', 'sub1']);

    ws.close();
  });

  it('refuses a WebSocket write by naming the same edge the document does', async () => {
    const connectorUrl = await startConnector(SELF_DESCRIPTION);
    instance = await boot({ connectorUrl, writeIlpAddress: 'g.toon.relay' });
    await waitForEdge(instance);

    const ws = new WebSocket(`ws://127.0.0.1:${relayPort}`);
    await new Promise<void>((resolve, reject) => {
      ws.once('open', () => resolve());
      ws.once('error', reject);
    });

    const ok = new Promise<unknown[]>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('no OK')), 5000);
      ws.on('message', (data: Buffer) => {
        const parsed: unknown = JSON.parse(data.toString());
        if (Array.isArray(parsed) && parsed[0] === 'OK') {
          clearTimeout(timer);
          resolve(parsed);
        }
      });
    });
    ws.send(
      JSON.stringify([
        'EVENT',
        {
          id: 'f'.repeat(64),
          kind: 1,
          content: 'hi',
          tags: [],
          sig: '',
          pubkey: '',
          created_at: 0,
        },
      ])
    );

    const frame = await ok;
    const message = frame[3] as string;
    const document = (await (
      await readDocument()
    ).json()) as RelayInformationDocument;
    expect(message).toContain(document.toon?.ilp_address ?? 'unreachable');
    expect(message).toContain(document.toon?.connector_url ?? 'unreachable');

    ws.close();
  });

  it('answers every other plain request exactly as it did before', async () => {
    instance = await boot();

    for (const [path, headers] of [
      ['/', {}],
      ['/', { accept: '*/*' }],
      ['/', { accept: 'text/html' }],
      ['/health', {}],
    ] as [string, Record<string, string>][]) {
      const response = await fetch(`http://127.0.0.1:${relayPort}${path}`, {
        headers,
      });
      expect(response.status, `${path} ${JSON.stringify(headers)}`).toBe(426);
      expect(await response.text()).toBe('Upgrade Required');
    }
  });

  it('serves a document with no edge when it was told about no connector', async () => {
    instance = await boot();

    const document = (await (
      await readDocument()
    ).json()) as RelayInformationDocument;
    expect(document.toon).toBeUndefined();
    expect(document.limitation.payment_required).toBe(false);
    // A relay with no payment gate in front of it still never takes a write
    // on the WebSocket.
    expect(document.limitation.restricted_writes).toBe(true);
  });

  it('refuses to start half-told, rather than advertising an unchecked address', async () => {
    await expect(boot({ writeIlpAddress: 'g.toon.relay' })).rejects.toThrow(
      /connectorUrl and writeIlpAddress go together/
    );
    // Nothing to tear down: the refusal happens before anything binds.
    rmSync(dataDir as string, { recursive: true, force: true });
    dataDir = undefined;
  });

  it('carries the operator’s own words when it has been given them', async () => {
    instance = await boot({
      description: { name: 'devnet relay', contact: 'mailto:ops@example' },
    });

    const document = (await (
      await readDocument()
    ).json()) as RelayInformationDocument;
    expect(document.name).toBe('devnet relay');
    expect(document.contact).toBe('mailto:ops@example');
    expect(document.description).toBeUndefined();
  });

  it('releases the read port on stop, so the next boot can take it', async () => {
    const node = await boot();
    const port = relayPort;
    await node.stop();

    // The read port is our own HTTP server now; `ws` only ever closed one it
    // had created itself, so forgetting to close it would leave the listener
    // bound and every later boot on this port would fail EADDRINUSE.
    const second = await startRelay({
      secretKey: generateSecretKey(),
      relayPort: port,
      blsPort: blsPort + 100,
      dataDir: mkdtempSync(join(tmpdir(), 'relay-nip11-')),
      eventStore: new InMemoryEventStore(),
    });
    expect(second.isRunning()).toBe(true);
    await second.stop();
  });
});
