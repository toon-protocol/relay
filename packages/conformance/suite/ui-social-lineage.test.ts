import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';
import {
  lineageFixture,
  type LineageFixture,
} from './harness/lineage-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let social: SocialFixture;
let lineage: LineageFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: {
      TOON_UI_RELAY_URL: 'wss://relay.example',
      TOON_UI_NSITE_GATEWAY: 'https://nsite.example',
    },
  });
  const now = Math.floor(Date.now() / 1000);
  social = socialFixture(now, 'relay.example');
  lineage = lineageFixture(social, now, 'relay.example');
  for (const event of [...social.events, ...lineage.events]) {
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
  expect(response.headers.get('content-security-policy')).toBe(POLICY);
  expect(text).not.toMatch(/<script/i);
  expect(text).not.toMatch(/\son\w+=/);
  expect(text).not.toMatch(/\sstyle=/);
  return { status: response.status, text };
}

/** The chip on the tree node that names `marker` (a data attribute). */
const chipOf = (text: string, marker: string): string | undefined => {
  const at = text.indexOf(marker);
  if (at < 0) return undefined;
  const node = text.slice(text.lastIndexOf('<div class="node', at), at);
  return />([^<]+)<\/span>/.exec(node)?.[1];
};

describe('relay image conformance: /social forks, remixes, live demos and Open in', () => {
  conformanceTest(
    'the Forks tab draws the fork tree, site remixes and same-first-commit repos',
    async () => {
      const base = `/social/r/${social.npubs.maya}/tidepool`;
      const { status, text } = await page(`${base}/forks`);
      expect(status).toBe(200);
      const repo = (a: string) => `data-repo-node="${a}"`;
      const site = (a: string) => `data-site="${a}"`;
      expect(chipOf(text, repo(lineage.repos.harmonics))).toBe('origin');
      expect(chipOf(text, repo(social.repos.tidepool))).toBe('this repo');
      expect(chipOf(text, repo(lineage.repos.harmonicsRs))).toBe('fork');
      expect(chipOf(text, repo(lineage.repos.ruthTidepool))).toBe('fork');
      expect(chipOf(text, repo(lineage.repos.tidepoolNz))).toBe(
        'same first commit'
      );
      expect(chipOf(text, site(lineage.sites.surfcast))).toBe('remix');
      expect(chipOf(text, site(lineage.sites.kitepool))).toBe('remix');
      expect(chipOf(text, site(lineage.sites.mirror))).toBe('identical copy');
      expect(chipOf(text, site(lineage.sites.lookalike))).toBe('same site');
      // Nested lists: origin, then this repo, then its fork, in that order.
      const order = [
        lineage.repos.harmonics,
        social.repos.tidepool,
        lineage.repos.ruthTidepool,
      ].map((a) => text.indexOf(repo(a)));
      expect(order).toEqual([...order].sort((x, y) => x - y));
      expect(text).toContain('<ul class="lineage lineage-root">');
      expect(text).toContain('aria-current="true"');
      // A remix links to the code it names and to its site on the gateway.
      expect(text).toContain(`href="/social/r/${social.npubs.leo}/surfcast"`);
      expect(text).toMatch(
        /href="https:\/\/[0-9a-z]{50}surfcast\.nsite\.example\/" rel="noopener noreferrer">Try it/
      );
      // People, not events: Ruth, Dan, Leo, Iris and the anonymous key.
      expect(text).toContain('Forks<span class="count">5</span>');
      expect(text).toContain('public and permanent');
    }
  );

  conformanceTest(
    'a fork says what it was forked from on /social, /repo and its card',
    async () => {
      const base = `/social/r/${social.npubs.maya}/tidepool`;
      const repoPage = await page(base);
      expect(repoPage.text).toContain(
        `Forked from <a href="${base}/forks">noaa-tools / harmonics</a>`
      );
      const home = await (
        await fetch(`${relay.readUrl}/repo/${social.npubs.maya}/tidepool`)
      ).text();
      expect(home).toContain(
        `Forked from <a href="/repo/${lineage.npubs.noaa}/harmonics">`
      );
      const feed = await page('/social');
      const card = feed.text.slice(
        feed.text.indexOf(`data-repo="${social.repos.tidepool}"`)
      );
      expect(card.split('class="meta"')[0]).toContain(
        'Forked from <a href="/social/r/'
      );
    }
  );

  conformanceTest(
    'a live demo shows on the card, the repo page and the launch alike',
    async () => {
      const demo =
        /href="https:\/\/[0-9a-z]{50}tidepool\.nsite\.example\/" rel="noopener noreferrer" data-demo/;
      const repoPage = await page(`/social/r/${social.npubs.maya}/tidepool`);
      expect(repoPage.text).toMatch(demo);
      expect(repoPage.text).toContain('class="demo-link"');
      const launch = await page(`/social/e/${social.named.ship.id}`);
      expect(launch.text).toMatch(demo);
      // A web app listing's (kind 32267) url is gitsail's website, not a
      // demo: NIP-82 defines it as the app's website.
      const feed = await page('/social');
      const card = feed.text
        .slice(feed.text.indexOf(`data-repo="${social.repos.gitsail}"`))
        .split('</article>')[0];
      expect(card).toContain(
        'href="https://gitsail.example" rel="nofollow noopener noreferrer" data-website'
      );
      expect(card).toContain('Website</a>');
      expect(card).not.toContain('data-demo');
    }
  );

  conformanceTest(
    'Open in lists recommended handlers with the naddr filled in, follows first',
    async () => {
      const base = `/social/r/${social.npubs.maya}/tidepool`;
      const anyone = (await page(base)).text;
      // Each handler's template, its placeholder filled with tidepool's naddr.
      const filled = (prefix: string): string => {
        const escaped = prefix.replace(/[.?/]/g, '\\$&');
        const match = new RegExp(`href="${escaped}(naddr1[0-9a-z]+)"`).exec(
          anyone
        );
        expect(match, prefix).not.toBeNull();
        const naddr = match?.[1] ?? '';
        const decoded = nip19.decode(naddr);
        expect(decoded.type).toBe('naddr');
        if (decoded.type === 'naddr') {
          expect(decoded.data.kind).toBe(30617);
          expect(decoded.data.pubkey).toBe(social.pubkeys.maya);
          expect(decoded.data.identifier).toBe('tidepool');
        }
        return `href="${prefix}${naddr}"`;
      };
      const gitworkshop = filled('https://gitworkshop.example/');
      const shakespeare = filled('https://shakespeare.diy/clone?url=nostr://');
      expect(anyone.indexOf(gitworkshop)).toBeLessThan(
        anyone.indexOf(shakespeare)
      );
      // Iris follows Maya, who recommends Shakespeare.
      const asIris = (await page(`${base}?as=${social.npubs.iris}`)).text;
      expect(asIris.indexOf(shakespeare)).toBeLessThan(
        asIris.indexOf(gitworkshop)
      );
      expect(asIris).toContain(`${base}/talk?as=${social.npubs.iris}`);
    }
  );

  conformanceTest(
    'Remix is offered only through a builder known to clone nostr:// repos',
    async () => {
      const { text } = await page(
        `/social/r/${social.npubs.maya}/tidepool/forks`
      );
      const clone = encodeURIComponent(
        `nostr://${social.npubs.maya}/relay.example/tidepool`
      );
      expect(text).toContain(
        `href="https://shakespeare.diy/clone?url=${clone}"`
      );
      expect(text).toContain('Remix in Shakespeare');
      expect(text).not.toContain('Remix in gitworkshop');
    }
  );
});
