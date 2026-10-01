import { generateSecretKey } from 'nostr-tools/pure';
import type { Event } from 'nostr-tools';
import { afterAll, describe, expect, vi } from 'vitest';
import { removeVolume, startRelay } from './harness/relay-container.js';
import {
  now,
  pubkeyOf,
  publish,
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
const images = {
  typescript: process.env['CONFORMANCE_TYPESCRIPT_IMAGE'] ?? '',
  rust: process.env['CONFORMANCE_RUST_IMAGE'] ?? '',
};
type Implementation = keyof typeof images;
const both = images.typescript !== '' && images.rust !== '';

// How long the expiring event outlives the first image's start: it must still
// be live when the second image first reads it, after the first image's writes,
// its stop and the second image's boot.
const EXPIRY_MARGIN_S = 30;

// Three boots (each up to 60s for /health) and the wait for the expiry.
vi.setConfig({ testTimeout: 300_000 });

const ids = (...events: Event[]) => events.map((e) => e.id).sort();

const volumes: string[] = [];
afterAll(async () => {
  await Promise.all(volumes.map(removeVolume));
});

describe.skipIf(!both)('relay image conformance: image swap over /data', () => {
  conformanceTestEach(
    [
      { from: 'typescript', to: 'rust' },
      { from: 'rust', to: 'typescript' },
    ] satisfies { from: Implementation; to: Implementation }[],
    ({ from, to }) => `${from} writes, ${to} reads the same volume`,
    async ({ from, to }) => {
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
      const doomedAddressable = sign(secretKey, {
        kind: 30023,
        created_at: t - 45,
        tags: [['d', 'doomed']],
      });
      const deletion = sign(secretKey, {
        kind: 5,
        created_at: t - 20,
        tags: [
          ['e', doomed.id],
          ['a', `30023:${pubkey}:doomed`],
        ],
      });
      const lasting = sign(secretKey, {
        kind: 1,
        created_at: t - 10,
        tags: [['expiration', String(t + 3600)]],
      });

      const written = sign(secretKey, { kind: 1, content: 'second' });

      const first = await startRelay(images[from], { volume });
      // Signed once the first image is up, so its boot does not eat the margin.
      const expiresAt = now() + EXPIRY_MARGIN_S;
      const expiring = sign(secretKey, {
        kind: 1,
        created_at: t - 10,
        tags: [['expiration', String(expiresAt)]],
      });
      try {
        for (const event of [
          regular,
          oldReplaceable,
          newReplaceable,
          oldAddressable,
          newAddressable,
          doomed,
          doomedAddressable,
          deletion,
          lasting,
          expiring,
        ]) {
          await publishOk(first, event);
        }
      } finally {
        await first.stop();
      }

      const second = await startRelay(images[to], { volume });
      try {
        // Its own events and the other image's tombstone and replacement
        // rules are all honoured by the image that did not write them.
        // The deletion request is itself a stored event, and is served.
        const readNow = () =>
          storedIds(second, { authors: [pubkey], limit: 100 });
        expect(await readNow()).toEqual(
          ids(
            regular,
            newReplaceable,
            newAddressable,
            deletion,
            lasting,
            expiring
          )
        );

        // Replaced and deleted events stay gone when re-submitted.
        for (const event of [
          oldReplaceable,
          oldAddressable,
          doomed,
          doomedAddressable,
        ]) {
          await publish(second, event);
        }
        expect(await readNow()).toEqual(
          ids(
            regular,
            newReplaceable,
            newAddressable,
            deletion,
            lasting,
            expiring
          )
        );

        // The expiring event is read as expired once its time has passed.
        while (now() <= expiresAt) await settle(500);
        expect(await readNow()).toEqual(
          ids(regular, newReplaceable, newAddressable, deletion, lasting)
        );

        // The second image can write to the volume too.
        await publishOk(second, written);
        expect(await readNow()).toContain(written.id);
      } finally {
        await second.stop();
      }

      // And the first image reads that write on the way back.
      const third = await startRelay(images[from], { volume });
      try {
        expect(
          await storedIds(third, { authors: [pubkey], limit: 100 })
        ).toEqual(
          ids(
            regular,
            newReplaceable,
            newAddressable,
            deletion,
            lasting,
            written
          )
        );
      } finally {
        await third.stop();
      }
    }
  );
});
