import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';

export const STUB_ILP_ADDRESS = 'g.toon.relay';
export const STUB_SEAL_KEY = 'ab'.repeat(32);
export const STUB_PRICE = '1000';
/**
 * The Write Edge the stub advertises as its `httpEndpoint`. Deliberately not
 * the URL the relay reads `GET /ilp` from, so a relay that echoes its own
 * configuration instead of the connector's self-description is caught.
 */
export const STUB_WRITE_EDGE = 'http://write-edge.stub.invalid:8080';
/** The settlement terms the stub accepts, as `batchSettlements` entries. */
export const STUB_SETTLEMENTS = [
  { network: 'eip155:31337', asset: '0x' + '11'.repeat(20) },
  { network: 'solana:devnet', asset: 'So' + '1'.repeat(40) },
];

/** A connector `GET /ilp` self-description, as JSON. */
export type IlpDocument = Record<string, unknown>;

/**
 * The stub's default self-description: one route that terminates at the relay
 * and a decoy at a longer prefix and another price. `overrides` replace whole
 * top-level keys, so a case varies exactly what it is about.
 */
export function ilpDocument(overrides: IlpDocument = {}): IlpDocument {
  return {
    httpEndpoint: STUB_WRITE_EDGE,
    edgeIdentity: { keyId: 'stub', publicKey: STUB_SEAL_KEY },
    batchSettlements: STUB_SETTLEMENTS,
    routes: [
      { prefix: STUB_ILP_ADDRESS, price: STUB_PRICE },
      { prefix: `${STUB_ILP_ADDRESS}.store`, price: '2000' },
    ],
    ...overrides,
  };
}

export interface StubConnector {
  /** The URL of its `GET /ilp`, as the relay container reaches it. */
  ilpUrl: string;
  /** How many `GET /ilp` requests it has answered (or would have). */
  requests(): number;
  /** Start listening on its port; a no-op while already listening. */
  start(): Promise<void>;
  /** Stop listening; the port stays reserved so `start()` brings it back. */
  stop(): Promise<void>;
}

export interface StubConnectorOptions {
  /** The document to serve (default {@link ilpDocument}). */
  document?: IlpDocument;
  /** Start listening at once (default true). False: a connector that is down. */
  listening?: boolean;
}

/**
 * A stand-in for the connector's free `GET /ilp` self-description. It serves
 * only that document, which is all the relay ever asks of its connector. Its
 * port is reserved for its lifetime, so a connector that is down can come up
 * at the address the relay was already told.
 *
 * @param advertisedHost - The host the relay container reaches this stub at.
 */
export async function startStubConnector(
  advertisedHost: string,
  options: StubConnectorOptions = {}
): Promise<StubConnector> {
  const document = options.document ?? ilpDocument();
  let requests = 0;
  let server: Server | undefined;

  const listen = (port: number): Promise<number> => {
    const created = createServer((req, res) => {
      if (req.method === 'GET' && req.url === '/ilp') {
        requests += 1;
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify(document));
        return;
      }
      res.writeHead(404).end();
    });
    server = created;
    return new Promise<number>((resolve, reject) => {
      created.once('error', reject);
      created.listen(port, '0.0.0.0', () =>
        resolve((created.address() as AddressInfo).port)
      );
    });
  };
  const close = (): Promise<void> => {
    const closing = server;
    server = undefined;
    if (closing === undefined) return Promise.resolve();
    return new Promise<void>((resolve) => {
      closing.closeAllConnections();
      closing.close(() => resolve());
    });
  };

  const port = await listen(0);
  if (options.listening === false) await close();

  return {
    ilpUrl: `http://${advertisedHost}:${port}/ilp`,
    requests: () => requests,
    start: async () => {
      if (server === undefined) await listen(port);
    },
    stop: close,
  };
}
