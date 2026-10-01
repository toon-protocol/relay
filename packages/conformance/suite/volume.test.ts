import { generateSecretKey } from 'nostr-tools/pure';
import type { Event } from 'nostr-tools';
import { afterAll, describe, expect } from 'vitest';
import {
  removeVolume,
  startRelay,
  type RunningRelay,
} from './harness/relay-container.js';
import {
  now,
  pubkeyOf,
  publishOk,
  settle,
  sign,
  storedIds,
} from './harness/wire.js';
import { conformanceTestEach } from './implementation.js';

// Cutover and rollback are both an image swap over the same /data volume.
// This file needs BOTH images, so it is driven by its own variables and runs
// in its own CI job (`image-swap`); without them it is skipped, because the
// per-implementation jobs have only one image.
const typescriptImage = process.env['CONFORMANCE_TYPESCRIPT_IMAGE'];
const rustImage = process.env['CONFORMANCE_RUST_IMAGE'];
const both = !!typescriptImage && !!rustImage;

const ids = (...events: Event[]) => events.map((e) => e.id).sort();

const volumes: string[] = [];
afterAll(async () => {
  await Promise.all(volumes.map(removeVolume));
});

async function stopAll(relay: RunningRelay | undefined) {
  await relay?.stop();
}

describe.skipIf(!both)('relay image conformance: image swap over /data', () => {
  conformanceTestEach(
    [
      { from: 'typescript', to: 'rust' },
      { from: 'rust', to: 'typescript' },
    ],
    ({ from, to }) => `${from} writes, ${to} reads the same volume`,
    async ({ from, to }) => {
      const image = (name: string) =>
        (name === 'rust' ? rustImage : typescriptImage) as string;
      const volume = `conformance-swap-${process.pid}-${Date.now()}-${from}`;
      volumes.push(volume);

      const secretKey = generateSecretKey();
      const pubkey = pubkeyOf(secretKey);
      const t = now();

      const regular = sign(secretKey, { kind: 1, created_at: t - 50 });
      const oldReplaceable = sign(secretKey, {
        kind: 10002,
        created_at: t - 40,
        content: 'old',
      });
      const newReplaceable = sign(secretKey, {
        kind: 10002,
        created_at: t - 30,
        content: 'new',
      });
      const oldAddressable = sign(secretKey, {
        kind: 30023,
        created_at: t - 40,
        tags: [['d', 'a']],
        content: 'old',
      });
      const newAddressable = sign(secretKey, {
        kind: 30023,
        created_at: t - 30,
        tags: [['d', 'a']],
        content: 'new',
      });
      const doomed = sign(secretKey, { kind: 1, created_at: t - 45 });
      const deletion = sign(secretKey, {
        kind: 5,
        created_at: t - 20,
        tags: [['e', doomed.id]],
      });
      const lasting = sign(secretKey, {
        kind: 1,
        created_at: t - 10,
        tags: [['expiration', String(t + 3600)]],
      });
      const expiring = sign(secretKey, {
        kind: 1,
        created_at: t - 10,
        tags: [['expiration', String(t + 8)]],
      });

      const written = sign(secretKey, { kind: 1, content: 'second' });

      const first = await startRelay(image(from), { volume });
      try {
        for (const event of [
          regular,
          oldReplaceable,
          newReplaceable,
          oldAddressable,
          newAddressable,
          doomed,
          deletion,
          lasting,
          expiring,
        ]) {
          await publishOk(first, event);
        }
      } finally {
        await stopAll(first);
      }

      const second = await startRelay(image(to), { volume });
      try {
        // Its own events and the other image's tombstone and replacement
        // rules are all honoured by the image that did not write them.
        const readNow = async () =>
          storedIds(second, { authors: [pubkey], limit: 100 });
        expect(await readNow()).toEqual(
          ids(regular, newReplaceable, newAddressable, lasting, expiring)
        );

        // A replaced and a deleted event stay gone when re-submitted.
        await publishOk(second, oldReplaceable).catch(() => undefined);
        await publishOk(second, doomed).catch(() => undefined);
        expect(await readNow()).not.toContain(doomed.id);
        expect(await readNow()).not.toContain(oldReplaceable.id);

        // The expiring event is read as expired once its time has passed.
        while (now() <= t + 8) await settle(500);
        expect(await readNow()).toEqual(
          ids(regular, newReplaceable, newAddressable, lasting)
        );

        // The second image can write to the volume too, and the first image
        // sees it on the way back.
        await publishOk(second, written);
        expect(await readNow()).toContain(written.id);
      } finally {
        await stopAll(second);
      }

      const third = await startRelay(image(from), { volume });
      try {
        expect(
          await storedIds(third, { authors: [pubkey], limit: 100 })
        ).toEqual(
          ids(regular, newReplaceable, newAddressable, lasting, written)
        );
      } finally {
        await stopAll(third);
      }
    }
  );
});
