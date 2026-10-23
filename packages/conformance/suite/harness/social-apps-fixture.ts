import { createHash } from 'node:crypto';
import { nip19 } from 'nostr-tools';
import type { Event } from 'nostr-tools';
import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { SocialFixture } from './social-fixture.js';

/**
 * What the /social apps, bounties and trust-and-safety pages read, on top
 * of the social fixture's community: Maya's tidepool as a NIP-82 app (the
 * draft at nostr-protocol/nips#1336) with three releases on two channels
 * and two files, a star and an app set that vouch for it, bounties on its
 * issues (one resolved by Maya, one claimed, one open), a funding goal,
 * reports on the repo and on one of its files, and a mute list.
 *
 * `seed/social.ts` loads these after the social fixture's events, and
 * `ui-social-apps.test.ts` seeds the same. Like the social fixture, the
 * keys are fixed and the data is made up.
 */
export interface SocialAppsFixture {
  /** The app's `32267:<maya>:dev.tide.tidepool` address. */
  app: string;
  /** Its `naddr`, and its `/social/app/<npub>/<d>` page. */
  appNaddr: string;
  appPath: string;
  /** The app set's `naddr`. */
  setNaddr: string;
  /** A key whose notes Dan's mute list hides. */
  spammer: { pubkey: string; npub: string };
  named: {
    app: Event;
    main: Event;
    olderMain: Event;
    beta: Event;
    apk: Event;
    wasm: Event;
    resolvedIssue: Event;
    resolvedBounty: Event;
    openBounty: Event;
    claimedBounty: Event;
    goal: Event;
    friendReport: Event;
    strangerReport: Event;
    spam: Event;
    airdrop: Event;
    muteList: Event;
  };
  /** Every event, in the order to write them. */
  events: Event[];
}

const keyOf = (label: string): Uint8Array =>
  new Uint8Array(
    createHash('sha256').update(`toon-social-fixture:${label}`).digest()
  );

const hex = (label: string, length: number): string =>
  createHash('sha256').update(label).digest('hex').slice(0, length);

/** The fixture, its events created in the ten minutes before `now`. */
export function socialAppsFixture(
  base: SocialFixture,
  now: number
): SocialAppsFixture {
  const { keys, pubkeys, npubs, repos } = base;
  const spammerKey = keyOf('spammer');
  const spammer = getPublicKey(spammerKey);
  let at = now - 600;
  const events: Event[] = [];
  const sign = (
    key: Uint8Array,
    kind: number,
    content: string,
    tags: string[][] = []
  ): Event => {
    at += 5;
    const event = finalizeEvent({ kind, created_at: at, tags, content }, key);
    events.push(event);
    return event;
  };
  const image = (seed: string) => `https://picsum.photos/seed/${seed}/600/1200`;
  const d = 'dev.tide.tidepool';
  const app = `32267:${pubkeys.maya}:${d}`;

  const appEvent = sign(
    keys.maya,
    32267,
    'Tide charts for small-boat sailors, computed **offline** from NOAA harmonics.\n\n' +
      '- Seven-day charts for any station\n- Works at the dock with no signal',
    [
      ['d', d],
      ['name', 'tidepool'],
      ['summary', 'Offline tide charts for small-boat sailors.'],
      ['icon', 'https://picsum.photos/seed/tidepool-icon/192/192'],
      ['image', image('tidepool-shot-1')],
      ['image', image('tidepool-shot-2')],
      ['image', image('tidepool-shot-3')],
      ['t', 'sailing'],
      ['t', 'weather'],
      ['url', 'https://tidepool.example'],
      ['repository', 'https://github.com/maya/tidepool'],
      ['a', repos.tidepool, 'wss://relay.example'],
      ['f', 'android-arm64-v8a'],
      ['license', 'MIT'],
      ['zap', pubkeys.maya, 'wss://relay.example', '4'],
      ['zap', pubkeys.kofi, 'wss://relay.example', '1'],
    ]
  );
  const commit = createHash('sha1').update('tidepool-v1.2.0').digest('hex');
  const apkHash = hex('tidepool.apk', 64);
  const apk = sign(keys.maya, 3063, '', [
    ['i', d],
    ['url', `https://blossom.example/${apkHash}.apk`],
    ['m', 'application/vnd.android.package-archive'],
    ['x', apkHash],
    ['size', '8100000'],
    ['version', '1.2.0'],
    ['f', 'android-arm64-v8a'],
    ['commit', commit],
    ['version_code', '120'],
    ['apk_certificate_hash', hex('cert', 64)],
    ['supported_nip', '01'],
    ['supported_nip', '34'],
  ]);
  const wasmHash = hex('tidepool.wasm', 64);
  const wasm = sign(keys.maya, 3063, '', [
    ['i', d],
    ['m', 'application/wasm'],
    ['x', wasmHash],
    ['size', '1300000'],
    ['version', '1.2.0'],
    ['f', 'wasm32'],
    ['commit', commit],
  ]);
  const release = (
    version: string,
    channel: string,
    notes: string,
    assets: Event[]
  ) =>
    sign(keys.maya, 30063, notes, [
      ['i', d],
      ['version', version],
      ['d', `${d}@${version}`],
      ['c', channel],
      ...assets.map((asset) => ['e', asset.id, 'wss://relay.example']),
    ]);
  const main = release(
    'v1.2.0',
    'main',
    'Southern Hemisphere preview, a smaller WebAssembly bundle, and a chart that reads better in direct sun.',
    [apk, wasm]
  );
  const beta = release(
    '1.3.0-beta.1',
    'beta',
    'Wind overlays (experimental). Expect the chart layout to change.',
    []
  );
  // Published last, but an older version: the page still shows v1.2.0.
  const olderMain = release('1.1.9', 'main', 'A backport of the cache fix.', [
    apk,
  ]);

  // Who vouches: Dan stars it, Iris lists it in a set.
  sign(keys.dan, 7, '+', [
    ['a', app],
    ['k', '32267'],
    ['p', pubkeys.maya],
  ]);
  sign(keys.iris, 30267, '', [
    ['d', 'sailing-kit'],
    ['title', "Iris's sailing kit"],
    ['description', 'Apps I keep on the boat.'],
    ['a', app],
  ]);

  // Bounties: Ruth's new issue, resolved by Maya; an open one on the repo;
  // a claimed one on gitsail.
  const resolvedIssue = sign(
    keys.ruth,
    1621,
    'The cache key in the service worker ignores the station.',
    [
      ['a', repos.tidepool],
      ['p', pubkeys.maya],
      ['subject', 'Fix the cache key in the service worker'],
    ]
  );
  sign(keys.maya, 1631, 'Fixed in v1.2.0.', [
    ['e', resolvedIssue.id, '', 'root'],
    ['a', repos.tidepool],
    ['p', pubkeys.ruth],
  ]);
  const bounty = (
    who: keyof typeof keys,
    dTag: string,
    title: string,
    price: string[],
    tags: string[][]
  ) =>
    sign(keys[who], 30402, `${title}. Details in the linked issue.`, [
      ['d', dTag],
      ['title', title],
      ['summary', `${title}, with tests.`],
      ['t', 'bounty'],
      ['price', ...price],
      ['published_at', String(at)],
      ...tags,
    ]);
  const resolvedBounty = bounty(
    'ruth',
    'cache-key',
    'Fix the cache key in the service worker',
    ['25', 'USDC'],
    [['e', resolvedIssue.id]]
  );
  const openBounty = bounty(
    'dan',
    'southern-stations',
    'Southern Hemisphere tide stations',
    ['50', 'USDC'],
    [['a', repos.tidepool]]
  );
  const claimedBounty = bounty(
    'iris',
    'gitsail-windows',
    'Windows build for gitsail',
    ['120', 'USDC'],
    [
      ['a', repos.gitsail],
      ['status', 'sold'],
    ]
  );
  const goal = sign(keys.maya, 9041, 'Fund a year of tide data hosting.', [
    ['amount', '500000000'],
    ['relays', 'wss://relay.example'],
    ['summary', 'A year of tide data hosting'],
    ['a', repos.tidepool],
  ]);

  // Reports: Iris (whom Ruth follows) flags the wasm file; a stranger
  // calls the repo spam.
  sign(keys.ruth, 3, '', [['p', pubkeys.iris]]);
  const friendReport = sign(
    keys.iris,
    1984,
    'The wasm build phones home to an unknown host.',
    [
      ['x', wasmHash, 'malware'],
      ['e', wasm.id, 'malware'],
      ['p', pubkeys.maya],
    ]
  );
  const strangerReport = sign(spammerKey, 1984, '', [
    ['a', repos.tidepool, 'spam'],
    ['p', pubkeys.maya],
  ]);

  // A mute list: Dan hides the spammer and #airdrop.
  const spam = sign(spammerKey, 1, 'Free tokens for every sailor!', []);
  const airdrop = sign(keys.leo, 1, 'Is there an airdrop for surfcast users?', [
    ['t', 'airdrop'],
  ]);
  const muteList = sign(keys.dan, 10000, '', [
    ['p', spammer],
    ['t', 'airdrop'],
  ]);

  return {
    app,
    appNaddr: nip19.naddrEncode({
      kind: 32267,
      pubkey: pubkeys.maya,
      identifier: d,
    }),
    appPath: `/social/app/${npubs.maya}/${d}`,
    setNaddr: nip19.naddrEncode({
      kind: 30267,
      pubkey: pubkeys.iris,
      identifier: 'sailing-kit',
    }),
    spammer: { pubkey: spammer, npub: nip19.npubEncode(spammer) },
    named: {
      app: appEvent,
      main,
      olderMain,
      beta,
      apk,
      wasm,
      resolvedIssue,
      resolvedBounty,
      openBounty,
      claimedBounty,
      goal,
      friendReport,
      strangerReport,
      spam,
      airdrop,
      muteList,
    },
    events,
  };
}
