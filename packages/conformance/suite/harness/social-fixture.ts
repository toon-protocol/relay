import { createHash } from 'node:crypto';
import { nip19 } from 'nostr-tools';
import type { Event } from 'nostr-tools';
import { finalizeEvent, getPublicKey } from 'nostr-tools/pure';

/**
 * A small builder community, as signed events: profiles, two repos and
 * their states, stars, follows, a ship post with imeta media and its live
 * demo, notes that mention a repo by naddr, a boost, comments, a thread, a
 * highlight, an article, a picture, a snippet, an issue, a merged pull
 * request, and a Shakespeare-style app event. The /social conformance cases
 * seed it, and `seed/social.ts` loads it into a relay a person can browse.
 *
 * Keys are derived from fixed labels, so the npubs (and the pages' URLs)
 * are the same on every run. The data is made up; the media URLs are
 * placeholder images a browser loads from picsum.photos.
 */

export type Person = 'maya' | 'kofi' | 'dan' | 'iris' | 'leo' | 'ruth';

export interface SocialFixture {
  /** Each person's secret key. */
  keys: Record<Person, Uint8Array>;
  /** Each person's hex pubkey. */
  pubkeys: Record<Person, string>;
  /** Each person's npub. */
  npubs: Record<Person, string>;
  /** The two repos' `30617:<owner>:<d>` addresses. */
  repos: { tidepool: string; gitsail: string; surfcast: string };
  /** Events the cases look for, by role. */
  named: {
    ship: Event;
    shipReply: Event;
    makerReply: Event;
    naddrNote: Event;
    boost: Event;
    comment: Event;
    thread: Event;
    highlight: Event;
    article: Event;
    picture: Event;
    snippet: Event;
    issue: Event;
    pull: Event;
    merge: Event;
    app: Event;
    deletedStar: Event;
  };
  /** Every event, in the order to write them. */
  events: Event[];
}

const keyOf = (label: string): Uint8Array =>
  new Uint8Array(
    createHash('sha256').update(`toon-social-fixture:${label}`).digest()
  );

const commit = (label: string): string =>
  createHash('sha1').update(label).digest('hex');

/**
 * The fixture, its events created in the hour before `now` (Unix seconds),
 * so "today" holds them all.
 */
export function socialFixture(
  now: number,
  relayHost = 'relay.example'
): SocialFixture {
  const people: Person[] = ['maya', 'kofi', 'dan', 'iris', 'leo', 'ruth'];
  const keys = Object.fromEntries(people.map((p) => [p, keyOf(p)])) as Record<
    Person,
    Uint8Array
  >;
  const pubkeys = Object.fromEntries(
    people.map((p) => [p, getPublicKey(keys[p])])
  ) as Record<Person, string>;
  const npubs = Object.fromEntries(
    people.map((p) => [p, nip19.npubEncode(pubkeys[p])])
  ) as Record<Person, string>;
  let at = now - 3_600;
  const events: Event[] = [];
  const sign = (
    who: Person,
    kind: number,
    content: string,
    tags: string[][] = []
  ): Event => {
    at += 10;
    const event = finalizeEvent(
      { kind, created_at: at, tags, content },
      keys[who]
    );
    events.push(event);
    return event;
  };
  const repos = {
    tidepool: `30617:${pubkeys.maya}:tidepool`,
    gitsail: `30617:${pubkeys.kofi}:gitsail`,
    surfcast: `30617:${pubkeys.leo}:surfcast`,
  };
  const image = (seed: string) => `https://picsum.photos/seed/${seed}/1200/675`;

  const profiles: Record<Person, Record<string, string>> = {
    maya: {
      name: 'Maya Okafor',
      about: 'Sails a 19-foot boat; builds tide tools.',
      nip05: 'maya@tide.dev',
    },
    kofi: { name: 'kofi', about: 'git over nostr', nip05: 'kofi@kofi.dev' },
    dan: { name: 'dan' },
    iris: { name: 'iris', about: 'relay-kit maintainer' },
    leo: { name: 'leo', about: 'vibe-codes surf apps with Shakespeare' },
    ruth: { name: 'ruth' },
  };
  for (const person of people) {
    sign(person, 0, JSON.stringify(profiles[person]));
  }

  sign('maya', 30617, '', [
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
    ['maintainers', pubkeys.kofi],
  ]);
  sign('kofi', 30618, '', [
    ['d', 'tidepool'],
    ['HEAD', 'ref: refs/heads/main'],
    ['refs/heads/main', commit('tidepool-main')],
  ]);
  sign('kofi', 30617, '', [
    ['d', 'gitsail'],
    ['name', 'gitsail'],
    ['description', 'A small git remote helper for nostr:// URLs.'],
    ['t', 'rust'],
    ['t', 'nostr'],
    ['t', 'git'],
  ]);
  sign('kofi', 30618, '', [
    ['d', 'gitsail'],
    ['HEAD', 'ref: refs/heads/main'],
    ['refs/heads/main', commit('gitsail-main')],
  ]);

  // Stars (kind 7 '+' with a and k=30617) and follows (10018, kind 3).
  const star = (who: Person, repo: string) =>
    sign(who, 7, '+', [
      ['a', repo],
      ['k', '30617'],
    ]);
  for (const who of ['dan', 'iris', 'leo', 'ruth'] as const) {
    star(who, repos.tidepool);
  }
  star('dan', repos.gitsail);
  star('iris', repos.gitsail);
  // A star taken back: the deletion removes it from every count.
  const deletedStar = star('kofi', repos.gitsail);
  sign('kofi', 5, '', [
    ['e', deletedStar.id],
    ['k', '7'],
  ]);
  sign('dan', 10018, '', [
    ['a', repos.tidepool],
    ['a', repos.gitsail],
  ]);
  sign('iris', 10018, '', [['a', repos.tidepool]]);
  sign('dan', 3, '', [
    ['p', pubkeys.maya],
    ['p', pubkeys.kofi],
  ]);

  // Kofi opens a pull request; Maya merges it. Ruth files an issue.
  const pull = sign('kofi', 1618, 'Caches the harmonics file for a year.', [
    ['a', repos.tidepool],
    ['p', pubkeys.maya],
    ['subject', 'Cache harmonic constants offline'],
    ['c', commit('tidepool-pr')],
  ]);
  const merge = sign('maya', 1631, 'Merged, thanks.', [
    ['e', pull.id, '', 'root'],
    ['a', repos.tidepool],
    ['p', pubkeys.kofi],
    ['merge-commit', commit('tidepool-merge')],
  ]);
  const issue = sign(
    'ruth',
    1621,
    'Wellington shows the tide 40 minutes late.',
    [
      ['a', repos.tidepool],
      ['p', pubkeys.maya],
      ['subject', 'Wellington is 40 minutes late'],
    ]
  );

  // The launch: Maya's note, tagged ship, with screenshots, and its demo.
  sign('maya', 35128, '', [
    ['d', 'tidepool'],
    ['path', '/index.html', 'f'.repeat(64)],
    ['source', `nostr://${npubs.maya}/${relayHost}/tidepool`],
  ]);
  const ship = sign(
    'maya',
    1,
    'Shipped tidepool: tide charts for small-boat sailors, offline-first. ' +
      'Vibe-coded the first version in a weekend, then rewrote the parser by hand. ' +
      `Try it, remix it, tell me what breaks.\n\n${image('tidepool-chart')}`,
    [
      ['a', repos.tidepool, `wss://${relayHost}`],
      ['t', 'ship'],
      ['t', 'sailing'],
      [
        'imeta',
        `url ${image('tidepool-chart')}`,
        'm image/jpeg',
        'alt A seven-day tide chart',
        'dim 1200x675',
      ],
      [
        'imeta',
        `url ${image('tidepool-stations')}`,
        'm image/jpeg',
        'alt The station picker',
        'dim 1200x675',
      ],
    ]
  );
  const shipReply = sign(
    'dan',
    1,
    'Works offline at the dock. Any plan for wind overlays?',
    [
      ['e', ship.id, '', 'root'],
      ['p', pubkeys.maya],
    ]
  );
  const makerReply = sign(
    'maya',
    1,
    'Wind is next. Opened an issue for it, PRs welcome.',
    [
      ['e', ship.id, '', 'root'],
      ['e', shipReply.id, '', 'reply'],
      ['p', pubkeys.dan],
    ]
  );
  sign('ruth', 1, 'The chart reads well in direct sun. Nice.', [
    ['e', ship.id, '', 'root'],
  ]);
  for (const [who, content] of [
    ['dan', '+'],
    ['iris', '+'],
    ['leo', ':shipit:'],
  ] as const) {
    sign(who, 7, content, [
      ['e', ship.id],
      ['p', pubkeys.maya],
      ['k', '1'],
    ]);
  }

  // Talk about the repos.
  const naddr = nip19.naddrEncode({
    kind: 30617,
    pubkey: pubkeys.kofi,
    identifier: 'gitsail',
  });
  const naddrNote = sign(
    'dan',
    1,
    `Cleanest NIP-34 remote helper I've used. Push, open a PR, done. nostr:${naddr}`
  );
  const boost = sign(
    'iris',
    16,
    JSON.stringify({ kind: 30617, pubkey: pubkeys.kofi }),
    [
      ['a', repos.gitsail],
      ['k', '30617'],
      ['p', pubkeys.kofi],
    ]
  );
  const comment = sign(
    'leo',
    1111,
    'The harmonics parser is worth reading on its own.',
    [
      ['A', repos.tidepool],
      ['K', '30617'],
      ['P', pubkeys.maya],
      ['a', repos.tidepool],
      ['k', '30617'],
      ['p', pubkeys.maya],
    ]
  );
  const thread = sign(
    'leo',
    11,
    'Remixed tidepool into a surf forecast. Lineage is on the repo.',
    [
      ['title', 'Show TOON: a surf forecast from tidepool'],
      ['a', repos.tidepool],
    ]
  );
  const highlight = sign(
    'ruth',
    9802,
    'Harmonic constants change slowly, so cache them for a year.',
    [['a', repos.tidepool]]
  );
  const article = sign(
    'maya',
    30023,
    '## What broke\n\nThe service worker cached the wrong harmonics file for a week.\n\n' +
      'Here is how I found it, and what `tidepool` does now.',
    [
      ['d', 'tidepool-nsite'],
      ['title', 'Moving tidepool to an nsite, and what broke'],
      [
        'summary',
        'The service worker cached the wrong harmonics file for a week.',
      ],
      ['image', image('tidepool-article')],
      ['published_at', String(at)],
      ['a', repos.tidepool],
    ]
  );
  const picture = sign('dan', 20, 'tidepool in dark mode', [
    ['title', 'Dark mode'],
    ['imeta', `url ${image('tidepool-dark')}`, 'm image/jpeg', 'alt Dark mode'],
    ['a', repos.tidepool],
  ]);
  const snippet = sign(
    'maya',
    1337,
    'fn height(t: f64, cs: &[Constituent]) -> f64 {\n    cs.iter().map(|c| c.amp * (c.speed * t + c.phase).cos()).sum()\n}',
    [
      ['l', 'rust'],
      ['name', 'harmonic.rs'],
      ['repo', repos.tidepool],
    ]
  );

  // Leo's Shakespeare app: a repo, its site and the app event tying them.
  sign('leo', 30617, '', [
    ['d', 'surfcast'],
    ['name', 'surfcast'],
    ['description', 'Surf forecasts, remixed from tidepool.'],
    ['t', 'shakespeare'],
  ]);
  sign('leo', 35128, '', [
    ['d', 'surfcast'],
    ['path', '/index.html', 'e'.repeat(64)],
  ]);
  const app = sign(
    'leo',
    31990,
    JSON.stringify({
      name: 'surfcast',
      about: 'Surf forecasts for your break, from tide harmonics.',
    }),
    [
      ['d', 'surfcast-app'],
      ['a', repos.surfcast],
      ['a', `35128:${pubkeys.leo}:surfcast`],
      ['t', 'shakespeare'],
      ['image', image('surfcast')],
    ]
  );

  return {
    keys,
    pubkeys,
    npubs,
    repos,
    named: {
      ship,
      shipReply,
      makerReply,
      naddrNote,
      boost,
      comment,
      thread,
      highlight,
      article,
      picture,
      snippet,
      issue,
      pull,
      merge,
      app,
      deletedStar,
    },
    events,
  };
}
