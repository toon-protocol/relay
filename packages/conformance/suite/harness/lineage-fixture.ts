import { createHash } from 'node:crypto';
import { nip19 } from 'nostr-tools';
import type { Event } from 'nostr-tools';
import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { SocialFixture } from './social-fixture.js';

/**
 * Forks, remixes and app handlers on top of the /social fixture
 * (social-fixture.ts), for the Forks tab, the live demo and "Open in…":
 *
 * - repos (NIP-34): noaa's `harmonics` is the origin; Maya's `tidepool`
 *   forks it (`u` tag, with a git URL) and Kofi's `harmonics-rs` too; Ruth
 *   forks `tidepool`; Dan's `tidepool-nz` has tidepool's first commit
 *   (`r … euc`) and no `u` tag;
 * - sites (NIP-5A): Leo's `surfcast` site copies Maya's `tidepool` site
 *   (`a` and `A`), Dan's `kitepool` copies Leo's (`a` Leo, `A` Maya), Iris
 *   mirrors Maya's byte for byte with credit, and an anonymous key deploys
 *   the same files with no credit tag (found by its `x` aggregate);
 * - apps: a kind 32267 listing for Kofi's `gitsail` with an `https` url (a
 *   live demo with no site), and two NIP-89 handlers for repos, gitworkshop
 *   (recommended by Dan and Ruth) and Shakespeare (recommended by Maya,
 *   whom Iris follows).
 *
 * It replaces Maya's tidepool announcement and Leo's surfcast site with
 * newer ones that carry the lineage tags, keeping every other tag. Load it
 * after the social fixture's events. Keys are derived from fixed labels.
 */

export interface LineageFixture {
  /** The extra people's hex pubkeys. */
  pubkeys: {
    noaa: string;
    anon: string;
    gitworkshop: string;
    shakespeare: string;
  };
  npubs: { noaa: string; anon: string };
  /** `30617:…` addresses. */
  repos: {
    harmonics: string;
    harmonicsRs: string;
    ruthTidepool: string;
    tidepoolNz: string;
  };
  /** `35128:…` site addresses. */
  sites: {
    tidepool: string;
    surfcast: string;
    kitepool: string;
    mirror: string;
    lookalike: string;
  };
  /** The handlers' `31990:…` addresses. */
  handlers: { gitworkshop: string; shakespeare: string };
  /** Every event, in the order to write them. */
  events: Event[];
}

const keyOf = (label: string): Uint8Array =>
  new Uint8Array(
    createHash('sha256').update(`toon-lineage-fixture:${label}`).digest()
  );

const commit = (label: string): string =>
  createHash('sha1').update(label).digest('hex');

/** NIP-5A's aggregate hash of a manifest's `path` tags. */
export const aggregate = (paths: string[][]): string =>
  createHash('sha256')
    .update(
      paths
        .map(([, path, hash]) => `${hash} ${path}\n`)
        .sort()
        .join('')
    )
    .digest('hex');

/**
 * The fixture, its events created after `now - 1800` (Unix seconds), so
 * they replace the social fixture's older ones.
 */
export function lineageFixture(
  social: SocialFixture,
  now: number,
  relayHost = 'relay.example'
): LineageFixture {
  const own = {
    noaa: keyOf('noaa'),
    anon: keyOf('anon'),
    gitworkshop: keyOf('gitworkshop'),
    shakespeare: keyOf('shakespeare'),
  };
  const pubkeys = {
    noaa: getPublicKey(own.noaa),
    anon: getPublicKey(own.anon),
    gitworkshop: getPublicKey(own.gitworkshop),
    shakespeare: getPublicKey(own.shakespeare),
  };
  const p = social.pubkeys;
  let at = now - 1_800;
  const events: Event[] = [];
  const sign = (
    key: Uint8Array,
    kind: number,
    content: string,
    tags: string[][] = []
  ): Event => {
    at += 10;
    const event = finalizeEvent({ kind, created_at: at, tags, content }, key);
    events.push(event);
    return event;
  };

  const repos = {
    harmonics: `30617:${pubkeys.noaa}:harmonics`,
    harmonicsRs: `30617:${p.kofi}:harmonics-rs`,
    ruthTidepool: `30617:${p.ruth}:tidepool`,
    tidepoolNz: `30617:${p.dan}:tidepool-nz`,
  };
  const sites = {
    tidepool: `35128:${p.maya}:tidepool`,
    surfcast: `35128:${p.leo}:surfcast`,
    kitepool: `35128:${p.dan}:kitepool`,
    mirror: `35128:${p.iris}:tidemirror`,
    lookalike: `35128:${pubkeys.anon}:tidepool`,
  };

  sign(
    own.noaa,
    0,
    JSON.stringify({ name: 'noaa-tools', about: 'Tide harmonics as data.' })
  );
  sign(own.noaa, 30617, '', [
    ['d', 'harmonics'],
    ['name', 'harmonics'],
    ['description', 'Harmonic constants for tide stations, as JSON.'],
    ['r', commit('harmonics-root'), 'euc'],
  ]);
  // Maya's tidepool, announced again with what it was forked from.
  const tidepoolRoot = commit('tidepool-root');
  sign(social.keys.maya, 30617, '', [
    ['d', 'tidepool'],
    ['name', 'tidepool'],
    [
      'description',
      'Offline tide charts from NOAA harmonics, rendered in the browser.',
    ],
    ['web', 'https://tidepool.example'],
    ['t', 'rust'],
    ['t', 'wasm'],
    ['t', 'sailing'],
    ['t', 'built-with:claude'],
    ['maintainers', p.kofi],
    ['r', tidepoolRoot, 'euc'],
    ['u', `${repos.harmonics}|https://git.example/noaa/harmonics.git`],
  ]);
  sign(social.keys.kofi, 30617, '', [
    ['d', 'harmonics-rs'],
    ['name', 'harmonics-rs'],
    ['description', 'The same constants as a Rust crate.'],
    ['u', repos.harmonics],
  ]);
  sign(social.keys.ruth, 30617, '', [
    ['d', 'tidepool'],
    ['name', 'tidepool'],
    ['description', 'Fixes for New Zealand stations.'],
    ['r', tidepoolRoot, 'euc'],
    ['u', `${social.repos.tidepool}|nostr://${social.npubs.maya}/tidepool`],
  ]);
  sign(social.keys.dan, 30617, '', [
    ['d', 'tidepool-nz'],
    ['name', 'tidepool-nz'],
    ['description', 'tidepool with the NZ station list, pushed as is.'],
    ['r', tidepoolRoot, 'euc'],
  ]);

  // Sites: Maya's (the social fixture's, its files), and the copies.
  const tidepoolFiles = [['path', '/index.html', 'f'.repeat(64)]];
  const tidepoolHash = aggregate(tidepoolFiles);
  sign(social.keys.leo, 35128, '', [
    ['d', 'surfcast'],
    ['a', sites.tidepool],
    ['A', sites.tidepool],
    ['path', '/index.html', 'e'.repeat(64)],
    ['title', 'surfcast'],
    ['description', 'Surf forecasts built on tidepool, made with Shakespeare.'],
    ['source', `nostr://${social.npubs.leo}/${relayHost}/surfcast`],
  ]);
  sign(social.keys.dan, 35128, '', [
    ['d', 'kitepool'],
    ['a', sites.surfcast],
    ['A', sites.tidepool],
    ['path', '/index.html', 'd'.repeat(64)],
    ['title', 'kitepool'],
    ['description', 'Wind and tide for kiters.'],
  ]);
  sign(social.keys.iris, 35128, '', [
    ['d', 'tidemirror'],
    ['a', sites.tidepool],
    ['A', sites.tidepool],
    ...tidepoolFiles,
    ['x', tidepoolHash, 'aggregate'],
    ['title', 'tidepool mirror'],
  ]);
  sign(own.anon, 35128, '', [
    ['d', 'tidepool'],
    ...tidepoolFiles,
    ['x', tidepoolHash, 'aggregate'],
  ]);

  // Kofi's gitsail as a web app listing (nips#1336): a demo with no site.
  sign(social.keys.kofi, 32267, '', [
    ['d', 'dev.gitsail'],
    ['name', 'gitsail'],
    ['a', social.repos.gitsail],
    ['url', 'https://gitsail.example'],
  ]);

  // NIP-89: two handlers for repos, and who recommends them.
  const handlers = {
    gitworkshop: `31990:${pubkeys.gitworkshop}:repos`,
    shakespeare: `31990:${pubkeys.shakespeare}:repos`,
  };
  sign(own.gitworkshop, 31990, JSON.stringify({ name: 'gitworkshop' }), [
    ['d', 'repos'],
    ['k', '30617'],
    ['web', 'https://gitworkshop.example/<bech32>', 'naddr'],
  ]);
  sign(own.shakespeare, 31990, JSON.stringify({ name: 'Shakespeare' }), [
    ['d', 'repos'],
    ['k', '30617'],
    ['web', 'https://shakespeare.diy/clone?url=nostr://<bech32>'],
  ]);
  const recommend = (key: Uint8Array, handler: string) =>
    sign(key, 31989, '', [
      ['d', '30617'],
      ['a', handler, `wss://${relayHost}`, 'web'],
    ]);
  recommend(social.keys.dan, handlers.gitworkshop);
  recommend(social.keys.ruth, handlers.gitworkshop);
  recommend(social.keys.maya, handlers.shakespeare);
  // Iris follows Maya, so viewing as Iris puts Maya's pick first.
  sign(social.keys.iris, 3, '', [['p', p.maya]]);

  return {
    pubkeys,
    npubs: {
      noaa: nip19.npubEncode(pubkeys.noaa),
      anon: nip19.npubEncode(pubkeys.anon),
    },
    repos,
    sites,
    handlers,
    events,
  };
}
