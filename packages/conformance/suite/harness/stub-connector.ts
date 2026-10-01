import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';

export const STUB_ILP_ADDRESS = 'g.toon.relay';
export const STUB_SEAL_KEY = 'ab'.repeat(32);
export const STUB_PRICE = '1000';

export interface StubConnector {
  /** The port it listens on, on every interface. */
  port: number;
  /** The `httpEndpoint` it advertises: what the relay should name as the edge. */
  httpEndpoint: string;
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
  let httpEndpoint = '';
  const server: Server = createServer((req, res) => {
    if (req.method === 'GET' && req.url === '/ilp') {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          httpEndpoint,
          edgeIdentity: { keyId: 'stub', publicKey: STUB_SEAL_KEY },
          settlements: [
            {
              chain: 'evm:31337',
              tokenAddress: '0x' + '11'.repeat(20),
              decimals: 6,
            },
          ],
          routes: [
            { prefix: STUB_ILP_ADDRESS, price: STUB_PRICE },
            { prefix: `${STUB_ILP_ADDRESS}.store`, price: STUB_PRICE },
          ],
        })
      );
      return;
    }
    res.writeHead(404).end();
  });
  await new Promise<void>((resolve) => server.listen(0, '0.0.0.0', resolve));
  const { port } = server.address() as AddressInfo;
  httpEndpoint = `http://${advertisedHost}:${port}/ilp`;
  return {
    port,
    httpEndpoint,
    stop: () =>
      new Promise<void>((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}
