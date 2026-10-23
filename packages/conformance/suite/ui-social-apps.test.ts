import { afterAll, beforeAll, describe, expect } from 'vitest';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';
import {
  socialAppsFixture,
  type SocialAppsFixture,
} from './harness/social-apps-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let base: SocialFixture;
let fixture: SocialAppsFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: { TOON_UI_RELAY_URL: 'wss://relay.example' },
  });
  const now = Math.floor(Date.now() / 1000);
  base = socialFixture(now, 'relay.example');
  fixture = socialAppsFixture(base, now);
  for (const event of [...base.events, ...fixture.events]) {
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
  expect(text).not.toMatch(/<script/i);
  expect(text).not.toMatch(/\son\w+=/);
  expect(text).not.toMatch(/\sstyle=/);
  return { status: response.status, text };
}

const has = (text: string, id: string) => text.includes(`data-id="${id}"`);

describe('relay image conformance: /social apps, bounties and trust', () => {
  conformanceTest(
    '/social/apps lists NIP-82 apps, filters by topic and counts who vouches',
    async () => {
      const apps = await page('/social/apps');
      expect(apps.status).toBe(200);
      expect(apps.text).toContain(`data-app="${fixture.app}"`);
      // Dan's star and Iris's app set: two people.
      expect(apps.text).toContain('data-vouches="2"');
      expect(apps.text).toContain('Iris&#39;s sailing kit');
      expect(apps.text).toContain('href="/social/apps">Apps</a>');
      expect(apps.text).toContain('href="/social/bounties">Bounties</a>');
      expect(apps.text).toContain('open draft (nostr-protocol/nips#1336)');
      expect((await page('/social/apps?t=weather')).text).toContain(
        `data-app="${fixture.app}"`
      );
      expect((await page('/social/apps?t=rust')).text).not.toContain(
        `data-app="${fixture.app}"`
      );
    }
  );

  conformanceTest(
    'an app page shows the newest release by version, its files and their commit',
    async () => {
      const { named } = fixture;
      const { status, text } = await page(fixture.appPath);
      expect(status).toBe(200);
      // 1.1.9 was published after v1.2.0, but v1.2.0 is the newer version.
      expect(text).toContain('data-release="v1.2.0"');
      expect(text).toContain('Southern Hemisphere preview');
      expect(text).toContain('href="/social/app/');
      expect(text).toContain('?channel=beta');
      expect(text).toContain(`data-asset="${named.apk.id}"`);
      expect(text).toContain(`data-asset="${named.wasm.id}"`);
      const apkUrl = named.apk.tags.find((tag) => tag[0] === 'url')?.[1] ?? '';
      expect(text).toContain(
        `href="${apkUrl}" rel="nofollow noopener noreferrer">Download`
      );
      expect(text).toContain('Fetch by hash from Blossom');
      const commit =
        named.apk.tags.find((tag) => tag[0] === 'commit')?.[1] ?? '';
      expect(text).toContain(
        `/repo/${base.npubs.maya}/tidepool/commit/${commit}`
      );
      expect(text).toContain('8.1 MB');
      expect(text).toContain('Supports NIP-01, NIP-34');
      expect(text).toContain('Tip split (set by the publisher)');
      expect(text).toContain('80%');
      expect(text).toContain('who also keeps the source repo');
      expect(text).toContain('<strong>offline</strong>');
      expect(text).toContain(`data-repo="${base.repos.tidepool}"`);
      // The NIP-82 draft's status is on the page.
      expect(text).toContain('still an open draft');

      const beta = await page(`${fixture.appPath}?channel=beta`);
      expect(beta.text).toContain('data-release="1.3.0-beta.1"');
      const byNaddr = await page(`/social/app/${fixture.appNaddr}`);
      expect(byNaddr.status).toBe(200);
      expect(byNaddr.text).toContain('data-release="v1.2.0"');
    }
  );

  conformanceTest(
    'read as a key, an app counts only its follows and shows their reports',
    async () => {
      const lens = await page(`${fixture.appPath}?as=${base.npubs.ruth}`);
      // Ruth follows Iris, who listed it and reported its wasm file.
      expect(lens.text).toContain('data-vouchers="1"');
      expect(lens.text).toContain('1 of them reported it.');
      expect(lens.text).toContain('data-reports-banner="1"');
      expect(lens.text).toContain('malware (1)');
      const plain = await page(fixture.appPath);
      expect(plain.text).not.toContain('data-reports-banner');
      expect(plain.text).toContain('data-reports="1"');
      const set = await page(`/social/apps/sets/${fixture.setNaddr}`);
      expect(set.status).toBe(200);
      expect(set.text).toContain(`data-app="${fixture.app}"`);
    }
  );

  conformanceTest(
    'bounties are open, claimed or resolved by the honoured status of their issue',
    async () => {
      const { named } = fixture;
      const { status, text } = await page('/social/bounties');
      expect(status).toBe(200);
      const state = (id: string) =>
        new RegExp(`data-id="${id}" data-bounty-status="([a-z]+)"`).exec(
          text
        )?.[1];
      expect(state(named.resolvedBounty.id)).toBe('resolved');
      expect(state(named.openBounty.id)).toBe('open');
      expect(state(named.claimedBounty.id)).toBe('claimed');
      expect(text).toContain('50 USDC');
      // The goal shows its target and says why there is no progress.
      expect(text).toContain(`data-goal="${named.goal.id}"`);
      expect(text).toContain('500,000 sats');
      expect(text).toContain('data-no-progress');
      expect(text).not.toMatch(/<progress|<meter/);
      const resolved = await page('/social/bounties?status=resolved');
      expect(has(resolved.text, named.resolvedBounty.id)).toBe(true);
      expect(has(resolved.text, named.openBounty.id)).toBe(false);
    }
  );

  conformanceTest(
    'the repo page names its latest release, open bounties and reports',
    async () => {
      const repoPage = `/social/r/${base.npubs.maya}/tidepool`;
      const plain = await page(repoPage);
      expect(plain.text).toContain('data-latest-release');
      expect(plain.text).toContain('v1.2.0');
      expect(plain.text).toContain('data-open-bounties="1"');
      // A stranger's spam report and Iris's malware report: two people.
      expect(plain.text).toContain('data-reports="2"');
      expect(plain.text).not.toContain('data-reports-banner');
      const lens = await page(`${repoPage}?as=${base.npubs.ruth}`);
      expect(lens.text).toContain('data-reports-banner="1"');
      const reports = await page(`${repoPage}/reports?as=${base.npubs.ruth}`);
      expect(reports.status).toBe(200);
      expect(has(reports.text, fixture.named.friendReport.id)).toBe(true);
      expect(has(reports.text, fixture.named.strangerReport.id)).toBe(true);
      expect(reports.text).toContain('data-reports-from-follows="1"');
    }
  );

  conformanceTest(
    "a mute list hides people and hashtags only in its owner's view",
    async () => {
      const { named } = fixture;
      const everyone = await page('/social');
      expect(has(everyone.text, named.spam.id)).toBe(true);
      expect(has(everyone.text, named.airdrop.id)).toBe(true);
      const dan = await page(`/social?as=${base.npubs.dan}`);
      expect(has(dan.text, named.spam.id)).toBe(false);
      expect(has(dan.text, named.airdrop.id)).toBe(false);
      expect(has(dan.text, base.named.ship.id)).toBe(true);
      expect(dan.text).toContain('data-muted');
    }
  );
});
