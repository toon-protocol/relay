import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import WebSocket from 'ws';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument } from './harness/nip11.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let repoOnly: RunningRelay;
let fixture: SocialFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: {
      TOON_UI_RELAY_URL: 'wss://relay.example',
      TOON_UI_NSITE_GATEWAY: 'https://nsite.example',
    },
  });
  repoOnly = await startRelay(imageUnderTest(), {
    env: { TOON_UI_APPS: 'repo' },
  });
  fixture = socialFixture(Math.floor(Date.now() / 1000), 'relay.example');
  for (const event of fixture.events) await publishOk(relay, event);
});

afterAll(async () => {
  await relay?.stop();
  await repoOnly?.stop();
});

interface Page {
  status: number;
  text: string;
  headers: Headers;
}

/** GET a /social page and check what every page must hold. */
async function page(path: string): Promise<Page> {
  const response = await fetch(`${relay.readUrl}${path}`);
  const text = await response.text();
  expect(response.headers.get('content-type')).toMatch(/^text\/html/);
  expect(response.headers.get('content-security-policy')).toBe(POLICY);
  expect(response.headers.get('referrer-policy')).toBe('no-referrer');
  // No script of any kind, and no inline handlers or styles.
  expect(text).not.toMatch(/<script/i);
  expect(text).not.toMatch(/\son\w+=/);
  expect(text).not.toMatch(/\sstyle=/);
  return { status: response.status, text, headers: response.headers };
}

const has = (text: string, id: string) => text.includes(`data-id="${id}"`);

describe('relay image conformance: the /social app UI', () => {
  conformanceTest(
    'GET /social is a page with its own policy and the shared stylesheet',
    async () => {
      const feed = await page('/social');
      expect(feed.status).toBe(200);
      const sheet = /<link rel="stylesheet" href="([^"]+)">/.exec(
        feed.text
      )?.[1];
      expect(sheet).toMatch(/^\/social\/assets\/style\.[0-9a-f]{16}\.css$/);
      const css = await fetch(`${relay.readUrl}${sheet}`);
      expect(css.status).toBe(200);
      expect(css.headers.get('cache-control')).toBe(
        'public, max-age=31536000, immutable'
      );
      // One switcher across the apps.
      expect(feed.text).toContain('class="app-link" href="/repo"');
      expect(feed.text).toContain('class="app-link current" href="/social"');
    }
  );

  conformanceTest(
    'the feed shows launches, unfurls repo mentions and keeps replies on their post',
    async () => {
      const { named, repos, npubs } = fixture;
      const { text } = await page('/social');
      expect(text).toContain(`data-id="${named.ship.id}" data-ship`);
      expect(text).toContain(`data-id="${named.app.id}" data-ship`);
      expect(text).toContain(
        '<img class="hero" src="https://picsum.photos/seed/tidepool-chart/1200/675"'
      );
      // The naddr in Dan's note unfurls into gitsail's card.
      expect(has(text, named.naddrNote.id)).toBe(true);
      expect(text).toContain(`data-repo="${repos.gitsail}"`);
      expect(has(text, named.boost.id)).toBe(true);
      expect(has(text, named.article.id)).toBe(true);
      expect(has(text, named.merge.id)).toBe(true);
      expect(has(text, named.shipReply.id)).toBe(false);
      // The clone line names this relay's host, and the owner's claimed name.
      expect(text).toContain(
        `git clone nostr://${npubs.maya}/relay.example/tidepool`
      );
      expect(text).toContain(
        'git clone nostr://maya@tide.dev/relay.example/tidepool'
      );
      // The live demo goes through the operator's gateway.
      expect(text).toMatch(
        /href="https:\/\/[0-9a-z]{50}tidepool\.nsite\.example\/" rel="noopener noreferrer" data-demo/
      );
      expect(text).toContain('Trending today');
    }
  );

  conformanceTest(
    'a repo card counts people, and a deleted star is gone',
    async () => {
      const { text } = await page('/social');
      const card = (repo: string) =>
        text
          .slice(text.indexOf(`data-repo="${repo}"`))
          .split('</div></div>')[0];
      expect(card(fixture.repos.tidepool)).toContain('data-stars="4"');
      expect(card(fixture.repos.tidepool)).toContain('data-followers="2"');
      expect(card(fixture.repos.tidepool)).toContain('1 open issue');
      expect(card(fixture.repos.gitsail)).toContain('data-stars="2"');
    }
  );

  conformanceTest(
    'the repo page splits the maker from the crowd, tab by tab',
    async () => {
      const { named, npubs } = fixture;
      const base = `/social/r/${npubs.maya}/tidepool`;
      const maker = await page(base);
      expect(maker.status).toBe(200);
      expect(has(maker.text, named.ship.id)).toBe(true);
      expect(has(maker.text, named.article.id)).toBe(true);
      expect(has(maker.text, named.comment.id)).toBe(false);
      // Code goes back to /repo.
      expect(maker.text).toContain(`href="/repo/${npubs.maya}/tidepool"`);

      const talk = await page(`${base}/talk`);
      for (const event of [named.comment, named.thread, named.highlight]) {
        expect(has(talk.text, event.id)).toBe(true);
      }
      expect(has(talk.text, named.ship.id)).toBe(false);

      const stars = await page(`${base}/stars`);
      for (const who of ['dan', 'iris', 'leo', 'ruth'] as const) {
        expect(stars.text).toContain(npubs[who]);
      }
      const followers = await page(`${base}/followers`);
      expect(followers.text).toContain(npubs.dan);
      expect(followers.text).toContain(npubs.iris);
      expect(followers.text).not.toContain(`following/${npubs.leo}"`);

      const media = await page(`${base}/media`);
      expect(media.text).toContain('https://picsum.photos/seed/tidepool-dark/');
      const snippets = await page(`${base}/snippets`);
      expect(has(snippets.text, named.snippet.id)).toBe(true);
      expect(snippets.text).toContain('harmonic.rs');

      // The Forks tab is ui-social-lineage.test.ts's; an unknown tab is not.
      expect((await page(`${base}/forks`)).status).toBe(200);
      expect((await page(`${base}/lineage`)).status).toBe(404);
      const naddr = nip19.naddrEncode({
        kind: 30617,
        pubkey: fixture.pubkeys.maya,
        identifier: 'tidepool',
      });
      const moved = await fetch(`${relay.readUrl}/social/r/${naddr}`, {
        redirect: 'manual',
      });
      expect(moved.status).toBe(301);
      expect(moved.headers.get('location')).toBe(base);
    }
  );

  conformanceTest(
    'a ship post is a launch with its replies, the maker marked',
    async () => {
      const { named } = fixture;
      const launch = await page(`/social/e/${named.ship.id}`);
      expect(launch.status).toBe(200);
      expect(launch.text).toContain('class="launch-title"');
      expect(launch.text).toContain('owns this repo');
      expect(has(launch.text, named.shipReply.id)).toBe(true);
      const makerReply = launch.text.slice(
        launch.text.indexOf(`data-id="${named.makerReply.id}"`)
      );
      expect(makerReply.split('</article>')[0]).toContain('chip-maker');
      expect(launch.text).toContain(':shipit: 1');
      // note1 and nevent spellings reach the same page.
      const note = await page(`/social/e/${nip19.noteEncode(named.ship.id)}`);
      expect(note.text).toContain('class="launch-title"');

      const app = await page(`/social/e/${named.app.id}`);
      expect(app.text).toContain('made with Shakespeare');
      expect(app.text).toContain('surfcast');
    }
  );

  conformanceTest(
    "the following feed reads the key's follows and followed repos",
    async () => {
      const { named, npubs } = fixture;
      const feed = await page(`/social/following/${npubs.dan}`);
      expect(feed.status).toBe(200);
      expect(has(feed.text, named.ship.id)).toBe(true);
      expect(has(feed.text, named.issue.id)).toBe(true);
      // Dan follows neither Leo nor surfcast, so Leo's launch is not his.
      expect(has(feed.text, named.app.id)).toBe(false);
      const form = await fetch(
        `${relay.readUrl}/social/following?npub=${npubs.dan}`,
        { redirect: 'manual' }
      );
      expect(form.status).toBe(303);
      expect(form.headers.get('location')).toBe(
        `/social/following/${npubs.dan}`
      );
    }
  );

  conformanceTest(
    '/repo links to /social and the switcher offers both',
    async () => {
      const home = await fetch(
        `${relay.readUrl}/repo/${fixture.npubs.maya}/tidepool`
      );
      const html = await home.text();
      expect(html).toContain(`href="/social/r/${fixture.npubs.maya}/tidepool"`);
      expect(html).toContain('class="app-link" href="/social"');
    }
  );

  conformanceTest(
    'with TOON_UI_APPS=repo, /social returns 426 and /repo draws no link to it',
    async () => {
      expect((await fetch(`${repoOnly.readUrl}/social`)).status).toBe(426);
      const list = await (await fetch(`${repoOnly.readUrl}/repo`)).text();
      expect(list).not.toContain('/social');
    }
  );

  conformanceTest(
    'an unknown post, repo or path is a 404 page, not 426',
    async () => {
      for (const path of [
        `/social/e/${'0'.repeat(64)}`,
        '/social/e/not-an-id',
        `/social/r/${fixture.npubs.maya}/none`,
        '/social/following/npub1nope',
        '/social/nope',
      ]) {
        expect((await page(path)).status, path).toBe(404);
      }
    }
  );

  conformanceTest(
    'a WebSocket upgrade and the NIP-11 document on /social still work',
    async () => {
      const upgraded = await new Promise<boolean>((resolve) => {
        const socket = new WebSocket(`${relay.readWsUrl}/social`);
        socket.once('open', () => {
          socket.close();
          resolve(true);
        });
        socket.once('error', () => resolve(false));
      });
      expect(upgraded).toBe(true);
      const document = await getDocument(`${relay.readUrl}/social`);
      expect(document.status).toBe(200);
      expect(document.body.pubkey).toMatch(/^[0-9a-f]{64}$/);
    }
  );
});
