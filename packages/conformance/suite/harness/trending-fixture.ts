import { createHash } from 'node:crypto';
import { nip19 } from 'nostr-tools';
import type { Event } from 'nostr-tools';
import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { SocialFixture } from './social-fixture.js';

/**
 * A month of activity on top of the social fixture, for /social/trending
 * and the topic pages: two more repos, stars, comments and boosts spread
 * over the last thirty days (so each window and each sparkline differs), a
 * fork, NIP-32 topic labels by a curator, a report, a reader whose follows
 * weight the ranking, and a NIP-85 provider's signed repo ranks.
 *
 * Load it after `socialFixture`'s events. Keys come from fixed labels, so the
 * npubs are the same on every run; the data is made up.
 */

export type TrendingPerson =
  | 'nina'
  | 'omar'
  | 'pia'
  | 'quinn'
  | 'curator'
  | 'vertex';

export interface TrendingFixture {
  pubkeys: Record<TrendingPerson, string>;
  npubs: Record<TrendingPerson, string>;
  /** The added repos, and Nina's fork of tidepool. */
  repos: { relayKit: string; wasmSynth: string; fork: string };
  /** Every event, in the order to write them. */
  events: Event[];
}

const keyOf = (label: string): Uint8Array =>
  new Uint8Array(
    createHash('sha256').update(`toon-trending-fixture:${label}`).digest()
  );

const DAY = 86_400;

export function trendingFixture(
  social: SocialFixture,
  now: number
): TrendingFixture {
  const people: TrendingPerson[] = [
    'nina',
    'omar',
    'pia',
    'quinn',
    'curator',
    'vertex',
  ];
  const keys = Object.fromEntries(people.map((p) => [p, keyOf(p)])) as Record<
    TrendingPerson,
    Uint8Array
  >;
  const pubkeys = Object.fromEntries(
    people.map((p) => [p, getPublicKey(keys[p])])
  ) as Record<TrendingPerson, string>;
  const npubs = Object.fromEntries(
    people.map((p) => [p, nip19.npubEncode(pubkeys[p])])
  ) as Record<TrendingPerson, string>;
  const events: Event[] = [];
  const sign = (
    key: Uint8Array,
    kind: number,
    created_at: number,
    content: string,
    tags: string[][] = []
  ): Event => {
    const event = finalizeEvent({ kind, created_at, tags, content }, key);
    events.push(event);
    return event;
  };
  const as = (who: TrendingPerson) => keys[who];
  const ago = (days: number) => now - Math.round(days * DAY);

  const names: Record<TrendingPerson, string> = {
    nina: 'nina',
    omar: 'omar',
    pia: 'pia',
    quinn: 'quinn',
    curator: 'stack-curator',
    vertex: 'vertex: builders web-of-trust',
  };
  for (const person of people) {
    sign(as(person), 0, ago(40), JSON.stringify({ name: names[person] }));
  }

  // Two more repos, by people from the social fixture.
  const relayKit = `30617:${social.pubkeys.iris}:relay-kit`;
  const wasmSynth = `30617:${social.pubkeys.ruth}:wasm-synth`;
  const fork = `30617:${pubkeys.nina}:tidepool`;
  sign(social.keys.iris, 30617, ago(40), '', [
    ['d', 'relay-kit'],
    ['name', 'relay-kit'],
    ['description', 'Building blocks for Nostr relays.'],
    ['t', 'rust'],
    ['t', 'nostr'],
  ]);
  sign(social.keys.ruth, 30617, ago(40), '', [
    ['d', 'wasm-synth'],
    ['name', 'wasm-synth'],
    ['description', 'A modular synth that runs in the browser.'],
    ['t', 'wasm'],
    ['t', 'rust'],
  ]);
  // Nina forks tidepool two days ago.
  sign(as('nina'), 30617, ago(2), '', [
    ['d', 'tidepool'],
    ['name', 'tidepool'],
    ['description', 'tidepool, for the Baltic.'],
    ['u', social.repos.tidepool],
    ['t', 'rust'],
  ]);

  const star = (who: TrendingPerson, at: number, repo: string) =>
    sign(as(who), 7, at, '+', [
      ['a', repo],
      ['k', '30617'],
    ]);
  // relay-kit: one person a day for a week.
  star('nina', ago(1), relayKit);
  star('omar', ago(3), relayKit);
  star('pia', ago(5), relayKit);
  sign(as('quinn'), 1111, ago(6), 'The subscription matcher is tidy.', [
    ['A', relayKit],
    ['K', '30617'],
    ['a', relayKit],
    ['k', '30617'],
  ]);
  // wasm-synth: only this month, then reported by Omar.
  star('nina', ago(12), wasmSynth);
  sign(as('pia'), 16, ago(20), '', [
    ['a', wasmSynth],
    ['k', '30617'],
  ]);
  sign(as('omar'), 1, ago(15), 'wasm-synth is fun, but the demo autoplays.', [
    ['a', wasmSynth],
  ]);
  sign(as('omar'), 1984, ago(14), 'Autoplays loud audio.', [
    ['a', wasmSynth, 'other'],
    ['p', social.pubkeys.ruth, 'other'],
  ]);
  // tidepool: a few older acts too, so its sparkline has a history.
  star('omar', ago(4), social.repos.tidepool);
  star('pia', ago(9), social.repos.tidepool);

  // Quinn follows Nina and Omar: ?as=<quinn> weights them and hides what
  // Omar reported.
  sign(as('quinn'), 3, ago(30), '', [
    ['p', pubkeys.nina],
    ['p', pubkeys.omar],
  ]);

  // A curator's NIP-32 topic labels, in the #t namespace.
  sign(as('curator'), 1985, ago(8), 'Shakespeare builds on Claude.', [
    ['L', '#t'],
    ['l', 'built-with:claude', '#t'],
    ['l', 'wasm', '#t'],
    ['a', social.repos.surfcast],
  ]);
  sign(as('curator'), 1985, ago(8), '', [
    ['L', '#t'],
    ['l', 'agents', '#t'],
    ['a', relayKit],
  ]);

  // A NIP-85 provider's signed ranks of three repos.
  for (const [repo, rank] of [
    [social.repos.tidepool, '91'],
    [relayKit, '70'],
    [social.repos.gitsail, '64'],
  ] as const) {
    sign(as('vertex'), 30384, ago(1), '', [
      ['d', repo],
      ['rank', rank],
    ]);
  }

  return {
    pubkeys,
    npubs,
    repos: { relayKit, wasmSynth, fork },
    events,
  };
}
