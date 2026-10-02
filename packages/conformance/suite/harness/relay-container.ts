import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import {
  startStubConnector,
  STUB_ILP_ADDRESS,
  type IlpDocument,
  type StubConnector,
} from './stub-connector.js';

const run = promisify(execFile);

/** The ports the image documents. */
const WRITE_PORT = 3100;
const READ_PORT = 7100;
const ALIAS = 'host.docker.internal';

/** The relay's identity secret key (hex) unless a case overrides it. */
export const DEFAULT_SECRET_KEY = '1'.repeat(64);

/**
 * The image's documented command, for a run that passes command-line flags
 * (the image's `CMD` is replaced by anything after the image name).
 */
function relayCommand(): string[] {
  return (process.env['CONFORMANCE_COMMAND'] || 'relay').split(' ');
}

/** Environment overrides: a string sets a variable, `undefined` removes it. */
export type Env = Record<string, string | undefined>;

export interface StartOptions {
  /**
   * What the relay is told about its connector. `up` (default): a stub that
   * answers. `down`: a stub address nothing listens on yet. `none`: no
   * connector is configured at all.
   */
  connector?: 'up' | 'down' | 'none';
  /** The self-description the stub serves (default the stub's own). */
  document?: IlpDocument;
  /** Overrides on top of the default environment. */
  env?: Env;
}

export interface RunningRelay {
  /** `http://127.0.0.1:<port>` of POST /write and GET /health. */
  writeUrl: string;
  /** `http://127.0.0.1:<port>` of the NIP-01 WebSocket and NIP-11 document. */
  readUrl: string;
  /** `ws://…` form of `readUrl`. */
  readWsUrl: string;
  /** The stub connector (present even when it is `down` or `none`). */
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

function envArgs(env: Env): string[] {
  return Object.entries(env).flatMap(([name, value]) =>
    value === undefined ? [] : ['-e', `${name}=${value}`]
  );
}

/**
 * Start `image` plus a stub connector, and wait for the relay's /health.
 * Only the image's two documented ports are touched, from outside. If the
 * environment moves a port (`TOON_BLS_PORT`, `TOON_RELAY_PORT`) the harness
 * follows it.
 */
export async function startRelay(
  image: string,
  options: StartOptions = {}
): Promise<RunningRelay> {
  const mode = options.connector ?? 'up';
  const connector = await startStubConnector(ALIAS, {
    ...(options.document !== undefined && { document: options.document }),
    listening: mode !== 'down',
  });
  const env: Env = {
    TOON_SECRET_KEY: DEFAULT_SECRET_KEY,
    ...(mode !== 'none' && {
      TOON_CONNECTOR_URL: connector.ilpUrl,
      TOON_WRITE_ILP_ADDRESS: STUB_ILP_ADDRESS,
    }),
    ...options.env,
  };
  const writePort = Number(env['TOON_BLS_PORT'] ?? WRITE_PORT);
  const readPort = Number(env['TOON_RELAY_PORT'] ?? READ_PORT);
  let container: string | undefined;
  try {
    container = await docker(
      'run',
      '-d',
      '--rm',
      '--add-host',
      `${ALIAS}:host-gateway`,
      '-p',
      `127.0.0.1::${writePort}`,
      '-p',
      `127.0.0.1::${readPort}`,
      ...envArgs(env),
      image
    );
    const writeUrl = `http://127.0.0.1:${await hostPort(container, writePort)}`;
    const readUrl = `http://127.0.0.1:${await hostPort(container, readPort)}`;
    await waitHealthy(writeUrl, container);
    const id = container;
    return {
      writeUrl,
      readUrl,
      readWsUrl: readUrl.replace(/^http/, 'ws'),
      connector,
      secretKey: env['TOON_SECRET_KEY'] ?? '',
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

export interface ExitedRelay {
  /** The process exit code. */
  code: number;
  /** Everything it wrote to stdout and stderr. */
  output: string;
}

/**
 * Run `image` with `env` and `args` and wait for it to exit, for settings the
 * relay must refuse to start with. Only the `TOON_*` variables given are set:
 * there is no default identity.
 */
export async function runRelayToExit(
  image: string,
  options: { env?: Env; args?: string[] } = {}
): Promise<ExitedRelay> {
  const args = options.args ?? [];
  const command = args.length > 0 ? [...relayCommand(), ...args] : [];
  // Named, so a relay that wrongly keeps running is removed, not leaked.
  const name = `conformance-exit-${process.pid}-${Date.now()}`;
  try {
    const { stdout, stderr } = await run(
      'docker',
      [
        'run',
        '--rm',
        '--name',
        name,
        ...envArgs(options.env ?? {}),
        image,
        ...command,
      ],
      { timeout: 60_000 }
    );
    return { code: 0, output: `${stdout}\n${stderr}` };
  } catch (error) {
    const failed = error as {
      code?: unknown;
      stdout?: string;
      stderr?: string;
      killed?: boolean;
    };
    if (typeof failed.code !== 'number' || failed.killed) {
      await docker('rm', '-f', name).catch(() => undefined);
      throw error;
    }
    return {
      code: failed.code,
      output: `${failed.stdout ?? ''}\n${failed.stderr ?? ''}`,
    };
  }
}
