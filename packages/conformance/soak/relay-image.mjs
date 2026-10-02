// Starting a relay image alone, for the runs in this directory that compare
// images: no connector, a fresh /data volume, both ports on loopback.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);
export const docker = async (...args) =>
  (await run('docker', args)).stdout.trim();
export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** The images named by `<variable>="<name>=<image>[,<name>=<image>…]"`. */
export function imagesFrom(variable) {
  const images = (process.env[variable] ?? '')
    .split(',')
    .filter(Boolean)
    .map((pair) => {
      const at = pair.indexOf('=');
      return at < 1
        ? {}
        : { name: pair.slice(0, at), image: pair.slice(at + 1) };
    });
  if (images.length === 0 || images.some((i) => !i.name || !i.image)) {
    console.error(`${variable}="<name>=<image>[,<name>=<image>…]" is required`);
    process.exit(2);
  }
  return images;
}

async function hostPort(container, port) {
  const match = /:(\d+)$/m.exec(await docker('port', container, `${port}/tcp`));
  if (!match) throw new Error(`no host port for ${port}`);
  return Number(match[1]);
}

/** Start `image` and wait until it is healthy. `cpus` is `docker run --cpus`. */
export async function start(image, { cpus } = {}) {
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
    ...(cpus ? ['--cpus', cpus] : []),
    image
  );
  try {
    const writePort = await hostPort(container, 3100);
    const readPort = await hostPort(container, 7100);
    const writeUrl = `http://127.0.0.1:${writePort}`;
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
    return {
      container,
      writeUrl,
      readUrl: `ws://127.0.0.1:${readPort}`,
      readHttpUrl: `http://127.0.0.1:${readPort}`,
    };
  } catch (error) {
    await remove(container);
    throw error;
  }
}

export const remove = (container) =>
  docker('rm', '-f', '-v', container).catch(() => undefined);
