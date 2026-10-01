import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import {
  startStubConnector,
  STUB_ILP_ADDRESS,
  type StubConnector,
} from './stub-connector.js';

const run = promisify(execFile);

/** The ports the image documents. */
const WRITE_PORT = 3100;
const READ_PORT = 7100;
const ALIAS = 'host.docker.internal';

export interface RelayOptions {
  /** Extra environment for the container, e.g. `TOON_ENFORCE_EXPIRATION`. */
  env?: Record<string, string>;
}

export interface RunningRelay {
  /** `http://127.0.0.1:<port>` of POST /write and GET /health. */
  writeUrl: string;
  /** `http://127.0.0.1:<port>` of the NIP-01 WebSocket and NIP-11 document. */
  readUrl: string;
  /** `ws://…` form of `readUrl`. */
  readWsUrl: string;
  connector: StubConnector;
  /** The relay's identity secret key (hex), as given to the container. */
  secretKey: string;
  stop(): Promise<void>;
}

async function docker(...args: string[]): Promise<string> {
  const { stdout } = await run('docker', args);
  return stdout.trim();
}

async function hostPort(container: string, port: number): Promise<number> {
  const mapping = await docker('port', container, `${port}/tcp`);
  const match = /:(\d+)$/m.exec(mapping);
  if (!match) throw new Error(`no host port for ${port}: ${mapping}`);
  return Number(match[1]);
}

async function waitHealthy(url: string, container: string): Promise<void> {
  const deadline = Date.now() + 60_000;
  let last = 'no answer';
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${url}/health`);
      if (response.ok) return;
      last = `HTTP ${response.status}`;
    } catch (error) {
      last = error instanceof Error ? error.message : String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  const logs = await docker('logs', container).catch(() => '');
  throw new Error(`relay never became healthy (${last})\n${logs}`);
}

/**
 * Start `image` plus a stub connector, and wait for the relay's /health.
 * Only the image's two documented ports are touched, from outside.
 */
export async function startRelay(
  image: string,
  options: RelayOptions = {}
): Promise<RunningRelay> {
  const connector = await startStubConnector(ALIAS);
  const secretKey = '1'.repeat(64);
  let container: string | undefined;
  try {
    container = await docker(
      'run',
      '-d',
      '--rm',
      '--add-host',
      `${ALIAS}:host-gateway`,
      '-p',
      `127.0.0.1::${WRITE_PORT}`,
      '-p',
      `127.0.0.1::${READ_PORT}`,
      '-e',
      `TOON_SECRET_KEY=${secretKey}`,
      '-e',
      `TOON_CONNECTOR_URL=${connector.ilpUrl}`,
      '-e',
      `TOON_WRITE_ILP_ADDRESS=${STUB_ILP_ADDRESS}`,
      ...Object.entries(options.env ?? {}).flatMap(([key, value]) => [
        '-e',
        `${key}=${value}`,
      ]),
      image
    );
    const writeUrl = `http://127.0.0.1:${await hostPort(container, WRITE_PORT)}`;
    const readUrl = `http://127.0.0.1:${await hostPort(container, READ_PORT)}`;
    await waitHealthy(writeUrl, container);
    const id = container;
    return {
      writeUrl,
      readUrl,
      readWsUrl: readUrl.replace(/^http/, 'ws'),
      connector,
      secretKey,
      stop: async () => {
        await docker('rm', '-f', id).catch(() => undefined);
        await connector.stop();
      },
    };
  } catch (error) {
    if (container) await docker('rm', '-f', container).catch(() => undefined);
    await connector.stop();
    throw error;
  }
}
