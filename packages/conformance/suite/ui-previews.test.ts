import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import {
  conformanceTest,
  conformanceTestEach,
  imageUnderTest,
} from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';

/**
 * Link previews (#314, ADR 0006): every app UI page carries Open Graph and X
 * card tags, and the card image a page names is a 1200×630 PNG the relay
 * draws from its own events. The public origin is set, so the tags are
 * absolute; the harness fetches the path part from the image under test.
 */
const ORIGIN = 'https://relay.example';

let relay: RunningRelay;
let fixture: SocialFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: {
      TOON_UI_RELAY_URL: 'wss://relay.example',
      TOON_UI_PUBLIC_URL: ORIGIN,
    },
  });
  fixture = socialFixture(Math.floor(Date.now() / 1000), 'relay.example');
  for (const event of fixture.events) {
    await publishOk(relay, event);
  }
});

afterAll(async () => {
  await relay?.stop();
});

/** The `content` of the `<meta>` called `name`, unescaped of `&amp;`. */
function meta(html: string, name: string): string | undefined {
  const match = new RegExp(
    `<meta (?:property|name)="${name.replace(/[\\^$.*+?()[\]{}|]/g, '\\$&')}" content="([^"]*)">`
  ).exec(html);
  return match?.[1]?.replace(/&amp;/g, '&');
}

async function html(path: string): Promise<string> {
  const response = await fetch(`${relay.readUrl}${path}`);
  expect(response.status, path).toBe(200);
  return response.text();
}

/** GET `url` (absolute, on ORIGIN) from the relay under test. */
async function image(
  url: string,
  headers: Record<string, string> = {}
): Promise<Response> {
  expect(url.startsWith(`${ORIGIN}/`), url).toBe(true);
  return fetch(`${relay.readUrl}${url.slice(ORIGIN.length)}`, { headers });
}

/** A PNG's width and height, from its IHDR chunk. */
function pngSize(bytes: Uint8Array): [number, number] | undefined {
  const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (bytes.length < 24 || signature.some((byte, at) => bytes[at] !== byte)) {
    return undefined;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return [view.getUint32(16), view.getUint32(20)];
}

async function expectCard(url: string): Promise<Response> {
  const response = await image(url);
  expect(response.status, url).toBe(200);
  expect(response.headers.get('content-type')).toBe('image/png');
  expect(response.headers.get('cache-control')).toMatch(
    /^public, max-age=\d+$/
  );
  expect(response.headers.get('etag')).toMatch(/^"[0-9a-f]{32}"$/);
  const size = pngSize(new Uint8Array(await response.clone().arrayBuffer()));
  expect(size, url).toEqual([1200, 630]);
  return response;
}

describe('relay image conformance: link previews on the app UIs', () => {
  conformanceTest(
    'a repo page names an absolute large card that is a PNG of 1200×630',
    async () => {
      const path = `/social/r/${fixture.npubs.maya}/tidepool`;
      const text = await html(path);
      expect(meta(text, 'og:site_name')).toBe('TOON');
      expect(meta(text, 'og:type')).toBe('website');
      expect(meta(text, 'og:title')).toBe('Maya Okafor / tidepool');
      expect(meta(text, 'og:description')).toBe(
        'Offline tide charts from NOAA harmonics, rendered in the browser.'
      );
      expect(meta(text, 'description')).toBe(meta(text, 'og:description'));
      expect(meta(text, 'og:url')).toBe(`${ORIGIN}${path}`);
      const card = `${ORIGIN}/social/og/r/${fixture.npubs.maya}/tidepool.png`;
      expect(meta(text, 'og:image')).toBe(card);
      expect(meta(text, 'og:image:width')).toBe('1200');
      expect(meta(text, 'og:image:height')).toBe('630');
      expect(meta(text, 'og:image:alt')).toBe('Maya Okafor / tidepool');
      expect(meta(text, 'twitter:card')).toBe('summary_large_image');
      expect(meta(text, 'twitter:title')).toBe('Maya Okafor / tidepool');
      expect(meta(text, 'twitter:image')).toBe(card);
      await expectCard(card);
    }
  );

  conformanceTest(
    "/repo's page names the same card while /social is on",
    async () => {
      const text = await html(`/repo/${fixture.npubs.maya}/tidepool`);
      expect(meta(text, 'og:image')).toBe(
        `${ORIGIN}/social/og/r/${fixture.npubs.maya}/tidepool.png`
      );
      await expectCard(`${ORIGIN}/repo/og/${fixture.npubs.maya}/tidepool.png`);
    }
  );

  conformanceTest(
    'a profile, a ship post and an article each name a card of their own',
    async () => {
      const profile = await html(`/social/p/${fixture.npubs.maya}`);
      expect(meta(profile, 'og:type')).toBe('profile');
      expect(meta(profile, 'og:description')).toBe(
        'Sails a 19-foot boat; builds tide tools.'
      );
      await expectCard(meta(profile, 'og:image') ?? '');
      expect(meta(profile, 'og:image')).toBe(
        `${ORIGIN}/social/og/p/${fixture.npubs.maya}.png`
      );

      const ship = await html(
        `/social/e/${nip19.noteEncode(fixture.named.ship.id)}`
      );
      expect(meta(ship, 'og:image')).toBe(
        `${ORIGIN}/social/og/e/${fixture.named.ship.id}.png`
      );
      expect(meta(ship, 'og:description')).toMatch(
        /^Shipped tidepool: tide charts/
      );
      await expectCard(meta(ship, 'og:image') ?? '');

      const naddr = nip19.naddrEncode({
        kind: 30023,
        pubkey: fixture.pubkeys.maya,
        identifier: 'tidepool-nsite',
      });
      const article = await html(`/social/a/${naddr}`);
      expect(meta(article, 'og:type')).toBe('article');
      expect(meta(article, 'og:title')).toBe(
        'Moving tidepool to an nsite, and what broke'
      );
      expect(meta(article, 'og:description')).toBe(
        'The service worker cached the wrong harmonics file for a week.'
      );
      expect(meta(article, 'og:image')).toBe(
        `${ORIGIN}/social/og/a/${fixture.npubs.maya}/tidepool-nsite.png`
      );
      await expectCard(meta(article, 'og:image') ?? '');
    }
  );

  conformanceTestEach(
    ['/social', '/social/trending', '/repo'],
    (path) =>
      `a page without a card of its own names its app’s site card: ${path}`,
    async (path) => {
      const text = await html(path);
      const app = path.split('/')[1];
      expect(meta(text, 'og:image')).toBe(`${ORIGIN}/${app}/og/site.png`);
      expect(meta(text, 'twitter:card')).toBe('summary_large_image');
      expect(meta(text, 'og:title')).toBeTruthy();
      await expectCard(meta(text, 'og:image') ?? '');
    }
  );

  conformanceTest('a card asked for again with its ETag is 304', async () => {
    const url = `${ORIGIN}/social/og/r/${fixture.npubs.kofi}/gitsail.png`;
    const first = await expectCard(url);
    const etag = first.headers.get('etag') ?? '';
    const again = await image(url, { 'if-none-match': etag });
    expect(again.status).toBe(304);
    expect(again.headers.get('etag')).toBe(etag);
  });

  conformanceTest(
    'what the relay does not hold has no card, and a card loosens no policy',
    async () => {
      const stranger = nip19.npubEncode('ab'.repeat(32));
      for (const path of [
        `/social/og/r/${fixture.npubs.maya}/no-such-repo.png`,
        `/social/og/p/${stranger}.png`,
        `/social/og/e/${'0'.repeat(64)}.png`,
        `/social/og/a/${fixture.npubs.maya}/no-such-article.png`,
        `/repo/og/${fixture.npubs.maya}/no-such-repo.png`,
        `/social/og/r/${fixture.npubs.maya}/tidepool.jpg`,
      ]) {
        const response = await fetch(`${relay.readUrl}${path}`);
        expect(response.status, path).toBe(404);
      }
      // A post that is not a ship post has no card: its page names the site's.
      const reply = await html(`/social/e/${fixture.named.shipReply.id}`);
      expect(meta(reply, 'og:image')).toBe(`${ORIGIN}/social/og/site.png`);
      const card = await image(`${ORIGIN}/social/og/site.png`);
      expect(card.headers.get('content-security-policy')).toBe(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'"
      );
    }
  );
});
