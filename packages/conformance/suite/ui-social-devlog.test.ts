import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';
import { devlogFixture, type DevlogFixture } from './harness/devlog-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let social: SocialFixture;
let devlog: DevlogFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: { TOON_UI_RELAY_URL: 'wss://relay.example' },
  });
  const now = Math.floor(Date.now() / 1000);
  social = socialFixture(now, 'relay.example');
  devlog = devlogFixture(social, now, 'relay.example');
  for (const event of [...social.events, ...devlog.events]) {
    await publishOk(relay, event);
  }
});

afterAll(async () => {
  await relay?.stop();
});

interface Page {
  status: number;
  text: string;
}

/** GET a /social page and check what every page must hold. */
async function page(path: string): Promise<Page> {
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

const has = (text: string, id: string) => text.includes(`data-id="${id}"`);

/**
 * The articles a page links to the reader, as `<pubkey>:<d>`. An naddr's
 * TLVs may come in any order, so links are compared decoded.
 */
function readerLinks(text: string): string[] {
  return [...text.matchAll(/href="\/social\/a\/(naddr1[0-9a-z]+)"/g)].flatMap(
    ([, naddr]) => {
      const decoded = nip19.decode(naddr ?? '');
      return decoded.type === 'naddr' && decoded.data.kind === 30023
        ? [`${decoded.data.pubkey}:${decoded.data.identifier}`]
        : [];
    }
  );
}

describe('relay image conformance: /social articles, series, highlights and snippets', () => {
  conformanceTest(
    'an article is read with its cover, series, author, repo and Markdown',
    async () => {
      const { named, articles } = devlog;
      const reader = await page(`/social/a/${articles.cache.naddr}`);
      expect(reader.status).toBe(200);
      const { text } = reader;
      expect(has(text, named.cache.id)).toBe(true);
      expect(text).toContain('Caching harmonics without a host');
      expect(text).toContain(
        '<img class="cover" src="https://picsum.photos/seed/tidepool-cache/1200/600"'
      );
      expect(text).toContain('Part 3 of 4');
      expect(text).toContain('1 min read');
      expect(text).toContain('<h2>The fix</h2>');
      // Raw HTML in Markdown is text, never markup.
      expect(text).not.toContain('<b>HTML</b>');
      expect(text).toContain('devlog of');
      expect(text).toContain(`data-repo="${social.repos.tidepool}"`);
      // A mention is drawn as a link to the person.
      expect(text).toContain(`href="/social/p/${social.npubs.kofi}"`);
      // The snippet mentioned on its own line is embedded.
      expect(has(text, named.snippet.id)).toBe(true);
      expect(text).toContain('&lt;never the path&gt;');
      // The series, in the author's order, with the current part marked.
      expect(text).toContain('tidepool devlog');
      expect(text).toContain('Why I built a tide app');
      expect(text).toContain('Moving tidepool to an nsite, and what broke');
      expect(text).toMatch(
        /class="part current" href="[^"]+" aria-current="page"><span class="mono">3<\/span>Caching harmonics/
      );
      expect(text).toContain('tidepool-next (not on this relay)');
    }
  );

  conformanceTest(
    'highlights are grouped by passage, counted by person, and the most-highlighted are marked',
    async () => {
      const { passages, articles } = devlog;
      const { text } = await page(`/social/a/${articles.cache.naddr}`);
      expect(text).toContain('3 passages highlighted by 5 people');
      expect(text).toContain(
        `<mark class="hl" title="3 people highlighted this">${passages.most}</mark>`
      );
      expect(text).toContain(
        `<mark class="hl" title="2 people highlighted this">${passages.second}</mark>`
      );
      // One person is not "most": in the rail, not marked.
      expect(text).toContain(`&ldquo;${passages.once}&rdquo;`);
      expect(text).not.toContain(`highlighted this">${passages.once}`);
      expect(text.match(/<mark /g)?.length).toBe(2);
    }
  );

  conformanceTest(
    "an article's reactions, boosts and comments count people",
    async () => {
      const { named, articles } = devlog;
      const { text } = await page(`/social/a/${articles.cache.naddr}`);
      expect(text).toContain('+ 2</span>');
      expect(text).toContain('🌊 1</span>');
      expect(text).toContain('1 person boosted');
      expect(text).toContain('2 comments from 2 people');
      expect(has(text, named.comment.id)).toBe(true);
      const maker = text.slice(
        text.indexOf(`data-id="${named.makerComment.id}"`)
      );
      expect(maker.split('</article>')[0]).toContain('chip-maker');
    }
  );

  conformanceTest(
    'every spelling of an article reaches the reader, and articles link to it',
    async () => {
      const { named } = devlog;
      const cache = `${social.pubkeys.maya}:tidepool-cache`;
      const byKey = await page(`/social/a/${social.npubs.maya}/tidepool-cache`);
      expect(byKey.status).toBe(200);
      expect(has(byKey.text, named.cache.id)).toBe(true);
      const moved = await fetch(`${relay.readUrl}/social/e/${named.cache.id}`, {
        redirect: 'manual',
      });
      expect(moved.status).toBe(303);
      const location = moved.headers.get('location') ?? '';
      expect(readerLinks(`href="${location}"`)).toEqual([cache]);

      const feed = await page('/social');
      expect(readerLinks(feed.text)).toContain(cache);
      const base = `/social/r/${social.npubs.maya}/tidepool`;
      const maker = await page(base);
      expect(readerLinks(maker.text)).toContain(cache);
      // Writing holds every article on the repo, the fan's too.
      const writing = await page(`${base}/writing`);
      expect(writing.status).toBe(200);
      expect(has(writing.text, named.cache.id)).toBe(true);
      expect(has(writing.text, named.review.id)).toBe(true);
      expect(readerLinks(writing.text)).toContain(
        `${social.pubkeys.dan}:a-week-with-tidepool`
      );
      expect(has(maker.text, named.review.id)).toBe(false);
    }
  );

  conformanceTest(
    'a snippet has its own page: language, licence, escaped code and its repo',
    async () => {
      const { named } = devlog;
      const path = `/social/s/${named.snippet.id}`;
      const snippet = await page(path);
      expect(snippet.status).toBe(200);
      expect(snippet.text).toContain('sw.ts');
      expect(snippet.text).toContain('>typescript<');
      expect(snippet.text).toContain('>MIT<');
      expect(snippet.text).toContain('workbox-core@7');
      expect(snippet.text).toContain('&lt;never the path&gt;');
      expect(snippet.text).not.toContain('<never the path>');
      // Its repo, named by a nostr:// URL, unfurls into the card.
      expect(snippet.text).toContain(`data-repo="${social.repos.tidepool}"`);
      const note = await page(
        `/social/s/${nip19.noteEncode(named.snippet.id)}`
      );
      expect(note.status).toBe(200);

      const tab = await page(
        `/social/r/${social.npubs.maya}/tidepool/snippets`
      );
      expect(tab.text).toContain(`href="${path}"`);
      expect(tab.text).toContain(`href="/social/s/${social.named.snippet.id}"`);
    }
  );

  conformanceTest('an unknown article or snippet is a 404 page', async () => {
    for (const path of [
      `/social/a/${nip19.naddrEncode({ kind: 30023, pubkey: social.pubkeys.maya, identifier: 'none' })}`,
      `/social/a/${nip19.naddrEncode({ kind: 30617, pubkey: social.pubkeys.maya, identifier: 'tidepool' })}`,
      '/social/a/naddr1nope',
      `/social/s/${devlog.named.cache.id}`,
      '/social/s/not-an-id',
    ]) {
      expect((await page(path)).status, path).toBe(404);
    }
  });
});
