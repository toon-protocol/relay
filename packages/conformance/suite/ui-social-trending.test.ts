import { afterAll, beforeAll, describe, expect } from 'vitest';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';
import {
  trendingFixture,
  type TrendingFixture,
} from './harness/trending-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let social: SocialFixture;
let trending: TrendingFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
  const now = Math.floor(Date.now() / 1000);
  social = socialFixture(now, 'relay.example');
  trending = trendingFixture(social, now);
  for (const event of [...social.events, ...trending.events]) {
    await publishOk(relay, event);
  }
});

afterAll(async () => {
  await relay?.stop();
});

/** GET a /social page and check what every page must hold. */
async function page(path: string): Promise<string> {
  const response = await fetch(`${relay.readUrl}${path}`);
  const text = await response.text();
  expect(response.status, path).toBe(200);
  expect(response.headers.get('content-security-policy')).toBe(POLICY);
  expect(text).not.toMatch(/<script/i);
  expect(text).not.toMatch(/\son\w+=/);
  expect(text).not.toMatch(/\sstyle=/);
  return text;
}

/** A ranked row, from its repo's address to the end of the row. */
function row(text: string, repo: string): string | undefined {
  const start = text.indexOf(`data-repo="${repo}"`);
  if (start < 0) return undefined;
  return text.slice(start, text.indexOf('</li>', start));
}

describe('relay image conformance: /social/trending and topics', () => {
  conformanceTest(
    'trending counts distinct people per window and prints its formula',
    async () => {
      const { repos } = social;
      const day = await page('/social/trending?window=day');
      expect(day).toContain('data-formula');
      expect(day).toContain('seg on" href="/social/trending?window=day"');
      expect(row(day, repos.tidepool)).toContain('data-people="4"');
      expect(row(day, trending.repos.relayKit)).toBeUndefined();

      const week = await page('/social/trending');
      // Six people: four from today, Nina's fork and Omar's star.
      expect(row(week, repos.tidepool)).toContain('data-people="6"');
      expect(row(week, repos.tidepool)).toContain('data-forks="1"');
      expect(row(week, repos.tidepool)).toMatch(
        /<span class="spark"[^>]*>(<i class="h\d"><\/i>){7}<\/span>/
      );
      expect(row(week, trending.repos.relayKit)).toContain('data-people="4"');
      expect(row(week, trending.repos.wasmSynth)).toBeUndefined();

      const month = await page('/social/trending?window=month');
      expect(row(month, repos.tidepool)).toContain('data-people="7"');
      expect(row(month, trending.repos.wasmSynth)).toContain('data-people="3"');
    }
  );

  conformanceTest(
    'weighting by a reader counts their follows 3x and hides what they reported',
    async () => {
      const text = await page(
        `/social/trending?window=month&as=${trending.npubs.quinn}`
      );
      expect(text).toContain('data-weighted');
      expect(text).toContain('1 repo is hidden');
      expect(row(text, trending.repos.wasmSynth)).toBeUndefined();
      expect(row(text, trending.repos.relayKit)).toContain('data-score="8"');
      expect(row(text, social.repos.tidepool)).toContain('data-score="11"');
    }
  );

  conformanceTest(
    'a topic page lists owners’ t tags and NIP-32 labels with the labeller',
    async () => {
      const text = await page('/social/t/built-with%3Aclaude');
      expect(row(text, social.repos.tidepool)).toContain('Tagged by its owner');
      const labelled = row(text, social.repos.surfcast);
      expect(labelled).toContain('data-labeller');
      expect(labelled).toContain(trending.npubs.curator);
      const index = await page('/social/t');
      expect(index).toContain(
        'href="/social/t/rust">rust<span class="count">5</span>'
      );
      const filtered = await page('/social/trending?topic=nostr');
      expect(row(filtered, trending.repos.relayKit)).toBeDefined();
      expect(row(filtered, social.repos.tidepool)).toBeUndefined();
      // The feed's sidebar and the repo page link topics here.
      const feed = await page('/social');
      expect(feed).toContain('data-topics');
      expect(feed).toContain('href="/social/trending"');
      const repo = await page(`/social/r/${social.npubs.maya}/tidepool`);
      expect(repo).toContain('class="chip chip-topic" href="/social/t/rust"');
    }
  );

  conformanceTest(
    'NIP-85 providers are listed, and one provider ranking is shown as signed',
    async () => {
      const text = await page('/social/trending');
      expect(text).toContain('data-other-rankings');
      expect(text).toContain(`data-provider="${trending.npubs.vertex}"`);
      const ranked = await page(`/social/trending?by=${trending.npubs.vertex}`);
      expect(row(ranked, social.repos.tidepool)).toContain('data-rank="91"');
      expect(ranked.indexOf(social.repos.tidepool)).toBeLessThan(
        ranked.indexOf(trending.repos.relayKit)
      );
    }
  );
});
