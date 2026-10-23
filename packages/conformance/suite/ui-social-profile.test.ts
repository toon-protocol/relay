import { afterAll, beforeAll, describe, expect } from 'vitest';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';
import {
  COUNTED_KINDS,
  graphSince,
  profileFixture,
  type ProfileFixture,
} from './harness/profile-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let base: SocialFixture;
let profile: ProfileFixture;
let now: number;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: { TOON_UI_RELAY_URL: 'wss://relay.example' },
  });
  now = Math.floor(Date.now() / 1000);
  base = socialFixture(now, 'relay.example');
  profile = profileFixture(base, now);
  for (const event of [...base.events, ...profile.events]) {
    await publishOk(relay, event);
  }
});

afterAll(async () => {
  await relay?.stop();
});

/** GET a /social page and check what every page must hold. */
async function page(path: string): Promise<{ status: number; text: string }> {
  const response = await fetch(`${relay.readUrl}${path}`);
  const text = await response.text();
  expect(response.headers.get('content-type')).toMatch(/^text\/html/);
  expect(response.headers.get('content-security-policy')).toBe(POLICY);
  expect(response.headers.get('referrer-policy')).toBe('no-referrer');
  expect(text).not.toMatch(/<script/i);
  expect(text).not.toMatch(/\son\w+=/);
  expect(text).not.toMatch(/\sstyle=/);
  return { status: response.status, text };
}

const maya = () => `/social/p/${base.npubs.maya}`;

describe('relay image conformance: the /social builder profile', () => {
  conformanceTest(
    'a profile shows the kind 0, its claims as claims, and its status',
    async () => {
      const { status, text } = await page(maya());
      expect(status).toBe(200);
      expect(text).toContain('<h1 class="profile-name">Maya Okafor</h1>');
      expect(text).toContain(
        '<img class="banner" src="https://picsum.photos/seed/maya-banner/1500/300"'
      );
      expect(text).toContain('href="https://tide.dev"');
      // NIP-05 and NIP-39 are drawn as unverified claims with their proofs.
      expect(text).toContain('maya@tide.dev</span><span class="chip"');
      expect(text).toContain('>unverified</span>');
      expect(text).toContain(
        'href="https://gist.github.com/mayaokafor/9721ce4ee4fceb91c9711ca2a6c9a5ab"'
      );
      expect(text).toContain(
        'href="https://twitter.com/mayabuilds/status/1619358434134196225"'
      );
      // NIP-38: the status, linking the repo it names.
      expect(text).toContain('data-status="general"');
      expect(text).toContain(`href="/social/r/${base.npubs.maya}/tidepool"`);
      // Dan and Iris follow her; she follows two people and one repo.
      expect(text).toContain('data-followers="2"');
      expect(text).toContain('data-following="2"');
      expect(text).toContain('data-repos-followed="1"');
      // NIP-58: Kofi's badge, which he awarded and she accepted.
      expect(text).toContain('>First merged PR</a>');
      // Following is done in a client.
      expect(text).toContain(`href="nostr:${base.npubs.maya}"`);
      expect(text).not.toContain('chip-bot');
    }
  );

  conformanceTest(
    'the pinned list keeps its order and only the key’s own repos',
    async () => {
      const { text } = await page(maya());
      const tidepool = text.indexOf(`data-repo="${base.repos.tidepool}"`);
      const nmea = text.indexOf(`data-repo="${profile.nmea}"`);
      expect(tidepool).toBeGreaterThan(-1);
      expect(nmea).toBeGreaterThan(tidepool);
      expect(text).not.toContain(`data-repo="${base.repos.gitsail}"`);
      expect(text).toContain('Pinned by this key, in its order (kind 10617)');
      // Kofi has no pinned list: his most-starred repos, and the page says so.
      const kofi = await page(`/social/p/${base.npubs.kofi}`);
      expect(kofi.text).toContain('No pinned list (kind 10617)');
      expect(kofi.text).toContain(`data-repo="${base.repos.gitsail}"`);
    }
  );

  conformanceTest(
    'the activity graph counts regular events only, and prints its rule',
    async () => {
      const { text } = await page(maya());
      const since = graphSince(now);
      const counted = [...base.events, ...profile.events].filter(
        (event) =>
          event.pubkey === base.pubkeys.maya &&
          COUNTED_KINDS.includes(event.kind) &&
          event.created_at >= since
      );
      const touched = new Set(
        counted.flatMap((event) =>
          event.tags
            .filter(
              ([name, value]) =>
                (name === 'a' || name === 'A') && value?.startsWith('30617:')
            )
            .map(([, value]) => value)
        )
      );
      expect(text).toContain(
        `data-contributions="${counted.length}" data-repos-touched="${touched.size}"`
      );
      expect(text).toContain(
        `${counted.length} contributions across ${touched.size} repos in the last year`
      );
      expect(text.match(/class="cell (l\d|future)"/g)).toHaveLength(53 * 7 + 5);
      expect(text).toContain('Repo state (30618) is not counted');
      // Recent activity: her newest contributions, the issue among them.
      expect(text).toContain(`data-id="${profile.named.issue.id}"`);
    }
  );

  conformanceTest(
    'the tabs are routes: repositories, starred and posts',
    async () => {
      const repos = await page(`${maya()}/repos`);
      expect(repos.status).toBe(200);
      expect(repos.text).toContain(`data-repo="${base.repos.tidepool}"`);
      expect(repos.text).toContain(`data-repo="${profile.nmea}"`);
      // Kofi maintains tidepool: it is on his Repositories tab, so marked.
      const kofi = await page(`/social/p/${base.npubs.kofi}/repos`);
      expect(kofi.text).toContain(`data-repo="${base.repos.tidepool}"`);
      expect(kofi.text).toContain('<span class="chip">maintainer</span>');
      const starred = await page(`${maya()}/starred`);
      expect(starred.text).toContain(`data-repo="${base.repos.gitsail}"`);
      const posts = await page(`${maya()}/posts`);
      expect(posts.text).toContain(`data-id="${base.named.ship.id}"`);
      expect(posts.text).toContain(`data-id="${base.named.article.id}"`);
      expect(posts.text).not.toContain(`data-id="${base.named.makerReply.id}"`);
      expect((await page(`${maya()}/agents`)).status).toBe(404);
      expect((await page('/social/p/npub1nope')).status).toBe(404);
    }
  );

  conformanceTest('a key whose profile says bot: true is marked', async () => {
    const { text } = await page(`/social/p/${profile.bot.npub}`);
    expect(text).toContain('chip-bot');
    expect(text).toContain(`data-id="${profile.named.botComment.id}"`);
  });

  conformanceTest(
    'authors across /social and /repo link to their profile',
    async () => {
      const link = `class="person-link" href="${maya()}"`;
      expect((await page('/social')).text).toContain(link);
      expect(
        (await page(`/social/r/${base.npubs.maya}/tidepool`)).text
      ).toContain(link);
      expect((await page(`/social/e/${base.named.ship.id}`)).text).toContain(
        link
      );
      const code = await fetch(
        `${relay.readUrl}/repo/${base.npubs.maya}/tidepool`
      );
      expect(await code.text()).toContain(link);
    }
  );
});
