import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import { publishOk } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { socialFixture, type SocialFixture } from './harness/social-fixture.js';
import {
  socialEventsFixture,
  type SocialEventsFixture,
} from './harness/social-events-fixture.js';

/** The policy every /social page carries: media over https, no script. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self' https:; " +
  "media-src 'self' https:; form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let plain: RunningRelay;
let base: SocialFixture;
let fixture: SocialEventsFixture;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: {
      TOON_NIP29_GROUPS: 'true',
      TOON_UI_RELAY_URL: 'wss://relay.example',
    },
  });
  plain = await startRelay(imageUnderTest());
  const now = Math.floor(Date.now() / 1000);
  base = socialFixture(now, 'relay.example');
  fixture = socialEventsFixture(now, base);
  for (const event of [...base.events, ...fixture.events]) {
    await publishOk(relay, event);
  }
});

afterAll(async () => {
  await relay?.stop();
  await plain?.stop();
});

/** GET a /social page and check what every page must hold. */
async function page(
  path: string,
  on: RunningRelay = relay
): Promise<{ status: number; text: string }> {
  const response = await fetch(`${on.readUrl}${path}`, { redirect: 'manual' });
  const text = await response.text();
  if (response.status !== 301) {
    expect(response.headers.get('content-type')).toMatch(/^text\/html/);
    expect(response.headers.get('content-security-policy')).toBe(POLICY);
    expect(text).not.toMatch(/<script/i);
    expect(text).not.toMatch(/\son\w+=/);
    expect(text).not.toMatch(/\sstyle=/);
  }
  return { status: response.status, text };
}

/** Where `needle` first appears in `text`, failing if it does not. */
function at(text: string, needle: string): number {
  const index = text.indexOf(needle);
  expect(index, needle).toBeGreaterThanOrEqual(0);
  return index;
}

describe('relay image conformance: /social events and rooms', () => {
  conformanceTest(
    'the events index splits calendar events into live, upcoming and past',
    async () => {
      const { status, text } = await page('/social/events');
      expect(status).toBe(200);
      const live = at(text, 'data-section="live-now"');
      const upcoming = at(text, 'data-section="upcoming"');
      const past = at(text, 'data-section="past"');
      const { calendar } = fixture;
      const build = at(text, `data-event="${calendar.buildNight}"`);
      const launch = at(text, `data-event="${calendar.launchDay}"`);
      const meetup = at(text, `data-event="${calendar.meetup}"`);
      const hack = at(text, `data-event="${calendar.hackathon}"`);
      expect(live).toBeLessThan(build);
      expect(build).toBeLessThan(upcoming);
      expect(upcoming).toBeLessThan(launch);
      expect(launch).toBeLessThan(meetup);
      expect(meetup).toBeLessThan(past);
      expect(past).toBeLessThan(hack);
      expect(text).toContain('(all day)');
    }
  );

  conformanceTest(
    'a hackathon page counts RSVPs by person and pins the winners of its badges',
    async () => {
      const { status, text } = await page(fixture.paths.hackathon);
      expect(status).toBe(200);
      expect(text).toContain('data-phase="ended"');
      expect(text).toContain('winners announced');
      expect(text).toContain('3 going &middot; 1 interested');
      expect(text).toMatch(/href="nostr:naddr1[0-9a-z]+">RSVP in your client/);
      expect(text).toContain('Hosted by');
      expect(text).toContain('3 projects shipped');
      expect(text).toContain('Winners pinned first');
      // Maya (Winner) and Leo (Crowd pick) are pinned before Dan.
      const tidepool = at(text, `data-id="${base.repos.tidepool}" data-winner`);
      const forge = at(text, `data-id="${fixture.forge}" data-winner`);
      const dan = at(text, `data-id="${fixture.named.danSubmission.id}"`);
      expect(Math.max(tidepool, forge)).toBeLessThan(dan);
      expect(text).not.toContain(
        `data-id="${fixture.named.danSubmission.id}" data-winner`
      );
      expect(text).toContain('<span class="chip chip-win">Winner</span>');
      expect(text).toContain('<span class="chip chip-win">Crowd pick</span>');
      // A note that names the event is talk, not a submission.
      expect(text).not.toContain(fixture.named.talk.id);
      // Badges: each awarded once; the stranger's award does not count.
      expect(text).toContain('data-badges');
      expect((text.match(/awarded to 1 person/g) ?? []).length).toBe(2);
      // Its room, read-only.
      expect(text).toContain('data-room="shipathon"');
      expect(text).toContain('Doors open. Post your repo early');
      expect(text).toContain('Read-only here; chat from your client');
    }
  );

  conformanceTest(
    'an naddr of a calendar event redirects to its page',
    async () => {
      const { pubkeys } = base;
      const naddr = nip19.naddrEncode({
        kind: 31923,
        pubkey: pubkeys.iris,
        identifier: 'weekend-shipathon',
      });
      const response = await fetch(`${relay.readUrl}/social/ev/${naddr}`, {
        redirect: 'manual',
      });
      expect(response.status).toBe(301);
      expect(response.headers.get('location')).toBe(fixture.paths.hackathon);
    }
  );

  conformanceTest(
    'a live event shows its stream as a link and an upcoming one waits for submissions',
    async () => {
      const live = await page(fixture.paths.buildNight);
      expect(live.status).toBe(200);
      expect(live.text).toContain('data-phase="live"');
      expect(live.text).toContain('Build night stream &middot; 12 watching');
      expect(live.text).toContain(
        'href="https://stream.example/build-night.m3u8"'
      );
      expect(live.text).not.toMatch(/<video[^>]+build-night/);
      expect(live.text).toContain('Submitted so far');

      const soon = await page(fixture.paths.launchDay);
      expect(soon.text).toContain('data-phase="before"');
      expect(soon.text).toContain('Submissions open when the event starts');
      expect(soon.text).toContain('1 going &middot; 1 interested');
      expect(soon.text).toContain('the host&#39;s time zone is Europe/Lisbon');
    }
  );

  conformanceTest(
    'a room shows its chat oldest first, its threads, and no input',
    async () => {
      const { status, text } = await page('/social/g/shipathon');
      expect(status).toBe(200);
      expect(text).toContain('#shipathon');
      const { named } = fixture;
      const first = at(text, `data-id="${named.doorsOpen.id}"`);
      const later = at(text, `data-id="${named.remixAnswer.id}"`);
      expect(first).toBeLessThan(later);
      // The answer quotes the question it replies to (NIP-C7 q tag).
      expect(text).toContain('remixing tidepool, is that allowed?');
      expect(text).toContain(`/social/e/${named.judging.id}`);
      expect(text).toContain('Judging criteria');
      expect(text).toContain('Read-only here; chat from your client');
      expect(text).not.toMatch(/<textarea|<input[^>]+name="(content|message)"/);
    }
  );

  conformanceTest(
    'a private group shows nothing on any page, as the relay serves it',
    async () => {
      const { named } = fixture;
      const room = await page('/social/g/judges');
      // No private group's metadata is served to a visitor: its room is not
      // found, as a room that does not exist is.
      expect(room.status).toBe(404);
      expect(room.text).toContain('needs sign-in in a client');
      expect(room.text).not.toContain('Judges only');
      expect(room.text).not.toContain('shortlist');
      expect((await page(`/social/e/${named.secretMessage.id}`)).status).toBe(
        404
      );
      expect((await page(`/social/e/${named.secretNote.id}`)).status).toBe(404);
      const feed = await page('/social');
      expect(feed.text).not.toContain('A note for the judges alone');
    }
  );

  conformanceTest(
    "a repo's page links the room named its d that its owner keeps",
    async () => {
      const { text } = await page(`/social/r/${base.npubs.maya}/tidepool`);
      expect(text).toContain('data-room="tidepool"');
      expect(text).toContain('href="/social/g/tidepool"');
      expect(text).toMatch(/2 messages today/);
    }
  );

  conformanceTest(
    "the feed's sidebar shows what is coming up, and the header links Events",
    async () => {
      const { text } = await page('/social');
      const box = at(text, 'data-coming-up');
      expect(at(text, 'gitsail build night')).toBeGreaterThan(box);
      expect(text).toContain('live now');
      expect(text).toContain('tidepool 2.0 launch day');
      expect(text).toContain('class="navlink" href="/social/events"');
    }
  );

  conformanceTest('a relay without NIP-29 has no rooms', async () => {
    const { status, text } = await page('/social/g/shipathon', plain);
    expect(status).toBe(404);
    expect(text).toContain('This relay has no rooms');
  });
});
