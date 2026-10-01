import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';

export const STUB_ILP_ADDRESS = 'g.toon.relay';
export const STUB_SEAL_KEY = 'ab'.repeat(32);
export const STUB_PRICE = '1000';
/** The carriage the stub pins on the relay's route. */
export const STUB_CARRIAGE = 'btp';
/**
 * The Write Edge the stub advertises as its `httpEndpoint`. Deliberately not
 * the URL the relay reads `GET /ilp` from, so a relay that echoes its own
 * configuration instead of the connector's self-description is caught.
 */
export const STUB_WRITE_EDGE = 'http://write-edge.stub.invalid:8080';

export interface StubConnector {
  /** The URL of its `GET /ilp`, as the relay container reaches it. */
  ilpUrl: string;
  stop(): Promise<void>;
}

/**
 * A stand-in for the connector's free `GET /ilp` self-description. It serves
 * only that document, which is all the relay ever asks of its connector.
 *
 * @param advertisedHost - The host the relay container reaches this stub at.
 */
export async function startStubConnector(
  advertisedHost: string
): Promise<StubConnector> {
  const server: Server = createServer((req, res) => {
    if (req.method === 'GET' && req.url === '/ilp') {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          httpEndpoint: STUB_WRITE_EDGE,
          edgeIdentity: { keyId: 'stub', publicKey: STUB_SEAL_KEY },
          settlements: [
            {
              chain: 'evm:31337',
              tokenAddress: '0x' + '11'.repeat(20),
              decimals: 6,
            },
          ],
          routes: [
            {
              prefix: STUB_ILP_ADDRESS,
              price: STUB_PRICE,
              requiredTransport: STUB_CARRIAGE,
            },
            // A longer prefix at another price, which is not the relay's edge.
            { prefix: `${STUB_ILP_ADDRESS}.store`, price: '2000' },
          ],
        })
      );
      return;
    }
    res.writeHead(404).end();
  });
  await new Promise<void>((resolve) => server.listen(0, '0.0.0.0', resolve));
  const { port } = server.address() as AddressInfo;
  return {
    ilpUrl: `http://${advertisedHost}:${port}/ilp`,
    stop: () =>
      new Promise<void>((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}
