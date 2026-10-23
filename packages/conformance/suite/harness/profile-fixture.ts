import { createHash } from 'node:crypto';
import { nip19 } from 'nostr-tools';
import type { Event } from 'nostr-tools';
import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { Person, SocialFixture } from './social-fixture.js';

/**
 * What the builder profile (`/social/p/<npub>`) reads, on top of the
 * /social fixture: Maya's fuller kind 0 (banner, website), a second repo,
 * a NIP-38 status, NIP-39 identities, a GitWorkshop pinned list (kind
 * 10617), a NIP-58 badge Kofi defined and awarded her, her follows and
 * star, a year of scattered activity, and a bot key that says it is one.
 * The profile conformance cases seed it after the base fixture, and
 * `seed/social.ts` loads it too. The data is made up.
 */
export interface ProfileFixture {
  /** The bot's key, npub and hex pubkey. */
  bot: { key: Uint8Array; pubkey: string; npub: string };
  /** Maya's second repo's address. */
  nmea: string;
  named: {
    status: Event;
    pins: Event;
    award: Event;
    issue: Event;
    oldNote: Event;
    botComment: Event;
  };
  /** The events to write after the base fixture's. */
  events: Event[];
}

/** The kinds the activity graph counts (crates/ui-social/src/profile/activity.rs). */
export const COUNTED_KINDS = [
  1617, 1618, 1619, 1621, 1630, 1631, 1632, 1633, 1111, 1, 9841, 9842,
];

const DAY = 86_400;

/** The first second of the activity graph that ends at `now`: 53 weeks from a Sunday. */
export function graphSince(now: number): number {
  const today = Math.floor(now / DAY);
  const weekday = (((today + 4) % 7) + 7) % 7;
  return (today - weekday - 52 * 7) * DAY;
}

/**
 * The profile events for `base`, the newest made in the half hour before
 * `now` (after every base event) and a few spread over the past year.
 */
export function profileFixture(
  base: SocialFixture,
  now: number
): ProfileFixture {
  const botKey = new Uint8Array(
    createHash('sha256').update('toon-social-fixture:shipbot').digest()
  );
  const botPubkey = getPublicKey(botKey);
  const keyOf = (who: Person | 'shipbot') =>
    who === 'shipbot' ? botKey : base.keys[who];
  const { pubkeys, repos } = base;
  let at = now - 1_800;
  const events: Event[] = [];
  const sign = (
    who: Person | 'shipbot',
    kind: number,
    content: string,
    tags: string[][] = [],
    createdAt?: number
  ): Event => {
    at += 10;
    const event = finalizeEvent(
      { kind, created_at: createdAt ?? at, tags, content },
      keyOf(who)
    );
    events.push(event);
    return event;
  };
  const nmea = `30617:${pubkeys.maya}:nmea-lite`;
  const badge = `30009:${pubkeys.kofi}:first-merge`;

  sign(
    'maya',
    0,
    JSON.stringify({
      name: 'Maya Okafor',
      about:
        'Sails a 19-foot boat; builds tide tools. Ex-firmware engineer, posts the repos here first.',
      nip05: 'maya@tide.dev',
      website: 'https://tide.dev',
      banner: 'https://picsum.photos/seed/maya-banner/1500/300',
    })
  );
  sign(
    'shipbot',
    0,
    JSON.stringify({
      name: 'shipbot',
      about: 'Writes release notes for tidepool. An agent.',
      bot: true,
    })
  );
  sign('maya', 30617, '', [
    ['d', 'nmea-lite'],
    ['name', 'nmea-lite'],
    ['description', 'Parse boat instrument sentences without allocations.'],
    ['t', 'rust'],
    ['t', 'embedded'],
  ]);
  const status = sign('maya', 30315, 'Shipping wind overlays for', [
    ['d', 'general'],
    ['a', repos.tidepool],
    ['expiration', String(now + 7 * DAY)],
  ]);
  sign('maya', 10011, '', [
    ['i', 'github:mayaokafor', '9721ce4ee4fceb91c9711ca2a6c9a5ab'],
    ['i', 'twitter:mayabuilds', '1619358434134196225'],
  ]);
  // Pinned: her two repos in her order; gitsail is not hers, so it is dropped.
  const pins = sign('maya', 10617, '', [
    ['a', repos.tidepool],
    ['a', nmea],
    ['a', repos.gitsail],
  ]);
  sign('kofi', 30009, '', [
    ['d', 'first-merge'],
    ['name', 'First merged PR'],
    ['description', 'Merged a pull request to gitsail'],
  ]);
  const award = sign('kofi', 8, '', [
    ['a', badge],
    ['p', pubkeys.maya],
  ]);
  sign('maya', 10008, '', [
    ['a', badge],
    ['e', award.id],
  ]);
  sign('maya', 3, '', [
    ['p', pubkeys.kofi],
    ['p', pubkeys.iris],
  ]);
  sign('iris', 3, '', [['p', pubkeys.maya]]);
  sign('maya', 10018, '', [['a', repos.gitsail]]);
  sign('maya', 7, '+', [
    ['a', repos.gitsail],
    ['k', '30617'],
  ]);
  const issue = sign('maya', 1621, 'Overlay forecast wind on the chart.', [
    ['a', repos.tidepool],
    ['subject', 'Wind overlays'],
  ]);
  sign('maya', 1111, 'The remote helper should cache refs.', [
    ['A', repos.gitsail],
    ['K', '30617'],
    ['a', repos.gitsail],
    ['k', '30617'],
  ]);
  const botComment = sign('shipbot', 1111, 'Release notes drafted for v0.3.', [
    ['A', repos.tidepool],
    ['K', '30617'],
  ]);
  // A year of scattered work, so the graph has shades.
  let oldNote: Event | undefined;
  for (const [daysAgo, perDay] of [
    [3, 2],
    [9, 1],
    [16, 4],
    [40, 1],
    [75, 3],
    [140, 2],
    [220, 1],
    [300, 5],
  ] as const) {
    for (let i = 0; i < perDay; i += 1) {
      const note = sign(
        'maya',
        i % 2 === 0 ? 1111 : 1,
        `Notes from the dock, day -${daysAgo}.`,
        i % 2 === 0
          ? [
              ['A', repos.tidepool],
              ['K', '30617'],
            ]
          : [],
        now - daysAgo * DAY - i * 60
      );
      oldNote ??= note;
    }
  }

  return {
    bot: { key: botKey, pubkey: botPubkey, npub: nip19.npubEncode(botPubkey) },
    nmea,
    named: {
      status,
      pins,
      award,
      issue,
      oldNote: oldNote as Event,
      botComment,
    },
    events,
  };
}
