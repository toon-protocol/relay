import { nip19 } from 'nostr-tools';
import type { Event } from 'nostr-tools';
import { finalizeEvent } from 'nostr-tools/pure';
import type { SocialFixture } from './social-fixture.js';

/**
 * Devlogs on top of the /social fixture (`social-fixture.ts`), signed by
 * the same people: Maya's tidepool devlog as a NIP-51 series (kind 30004)
 * of three articles and one not written yet, the newest article with a
 * cover, a mention, an embedded code snippet and passages readers
 * highlighted (kind 9802), reactions, a boost and NIP-22 comments on it, a
 * fan's write-up of the repo, and a second snippet (NIP-C0) that names its
 * repo by a `nostr://` URL. The /social devlog conformance cases seed it
 * after the base fixture, and `seed/social.ts` loads both.
 *
 * Its events are created in the ten minutes before `now`, after the base
 * fixture's. The data is made up; the cover is a placeholder image a
 * browser loads from picsum.photos.
 */

export interface DevlogFixture {
  /** The articles' `30023:<maya>:<d>` addresses and `naddr`s. */
  articles: {
    why: { address: string; naddr: string };
    cache: { address: string; naddr: string };
  };
  /** The passages readers highlighted, most-highlighted first. */
  passages: { most: string; second: string; once: string };
  named: {
    why: Event;
    cache: Event;
    series: Event;
    snippet: Event;
    review: Event;
    comment: Event;
    makerComment: Event;
  };
  /** Every event, in the order to write them. */
  events: Event[];
}

export function devlogFixture(
  base: SocialFixture,
  now: number,
  relayHost = 'relay.example'
): DevlogFixture {
  const { keys, pubkeys, npubs, repos } = base;
  let at = now - 600;
  const events: Event[] = [];
  const sign = (
    who: keyof typeof keys,
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
  const article = (d: string) => ({
    address: `30023:${pubkeys.maya}:${d}`,
    naddr: nip19.naddrEncode({
      kind: 30023,
      pubkey: pubkeys.maya,
      identifier: d,
    }),
  });
  const articles = {
    why: article('tidepool-why'),
    cache: article('tidepool-cache'),
  };
  const passages = {
    most: 'Harmonic constants change slowly, so cache them for a year.',
    second:
      'If the hash changes, the cache name changes, and the old one is simply never read again.',
    once: 'There is no host to go down.',
  };

  const why = sign(
    'maya',
    30023,
    'I sail a 19-foot boat out of a harbour with a bar. Every tide app I tried ' +
      'needed a signal I do not have at the mooring, so I wrote one that does not.',
    [
      ['d', 'tidepool-why'],
      ['title', 'Why I built a tide app'],
      ['summary', 'Offline first, because the mooring has no signal.'],
      ['published_at', String(at)],
      ['a', repos.tidepool],
      ['t', 'devlog'],
    ]
  );
  const snippet = sign(
    'maya',
    1337,
    'const cache = await caches.open(`harmonics-${blobHash}`);\n' +
      'await cache.put(request, response.clone()); // <never the path>',
    [
      ['l', 'typescript'],
      ['name', 'sw.ts'],
      ['extension', 'ts'],
      ['description', 'Name the cache after the blob hash, not the path.'],
      ['runtime', 'service worker'],
      ['license', 'MIT'],
      ['dep', 'workbox-core@7'],
      ['repo', `nostr://${npubs.maya}/${relayHost}/tidepool`],
    ]
  );
  const snippetRef = nip19.neventEncode({
    id: snippet.id,
    author: pubkeys.maya,
  });
  const cache = sign(
    'maya',
    30023,
    `An nsite is just a list of file hashes signed by my key. Any gateway can serve it. ${passages.once}\n\n` +
      `${passages.most} I wrote that in the README and forgot that the service worker ` +
      'cached the wrong file too, for a week, on every phone that had opened the app.\n\n' +
      '## The fix\n\n' +
      `Name the cache after the blob hash, not the path. ${passages.second} ` +
      `Thanks to nostr:${npubs.kofi} for spotting it.\n\n` +
      `nostr:${snippetRef}\n\n` +
      'Raw <b>HTML</b> stays text, and `code` stays code.',
    [
      ['d', 'tidepool-cache'],
      ['title', 'Caching harmonics without a host'],
      ['summary', 'The service worker cached the wrong file for a week.'],
      ['image', 'https://picsum.photos/seed/tidepool-cache/1200/600'],
      ['published_at', String(at)],
      ['a', repos.tidepool],
      ['e', snippet.id, '', 'mention'],
      ['p', pubkeys.kofi],
      ['t', 'devlog'],
      ['t', 'nsite'],
    ]
  );
  const series = sign('maya', 30004, '', [
    ['d', 'tidepool-devlog'],
    ['title', 'tidepool devlog'],
    ['description', 'Building an offline tide app in public.'],
    ['a', articles.why.address],
    ['a', `30023:${pubkeys.maya}:tidepool-nsite`],
    ['a', articles.cache.address],
    ['a', `30023:${pubkeys.maya}:tidepool-next`],
  ]);

  // Highlights: three people, two people and one person.
  const highlight = (who: keyof typeof keys, text: string) =>
    sign(who, 9802, text, [
      ['a', articles.cache.address],
      ['p', pubkeys.maya, '', 'author'],
    ]);
  for (const who of ['ruth', 'dan', 'iris'] as const) {
    highlight(who, passages.most);
  }
  for (const who of ['kofi', 'leo'] as const) {
    highlight(who, passages.second);
  }
  highlight('leo', passages.once);

  for (const [who, content] of [
    ['dan', '+'],
    ['iris', '+'],
    ['leo', '🌊'],
  ] as const) {
    sign(who, 7, content, [
      ['a', articles.cache.address],
      ['e', cache.id],
      ['p', pubkeys.maya],
      ['k', '30023'],
    ]);
  }
  sign('iris', 16, '', [
    ['a', articles.cache.address],
    ['k', '30023'],
    ['p', pubkeys.maya],
  ]);
  const comment = sign(
    'dan',
    1111,
    'Same bug bit me on a PWA last year. Hash-named caches are the way.',
    [
      ['A', articles.cache.address],
      ['K', '30023'],
      ['P', pubkeys.maya],
      ['a', articles.cache.address],
      ['k', '30023'],
      ['p', pubkeys.maya],
    ]
  );
  const makerComment = sign(
    'maya',
    1111,
    'It only took one bad week to learn it.',
    [
      ['A', articles.cache.address],
      ['K', '30023'],
      ['P', pubkeys.maya],
      ['e', comment.id],
      ['k', '1111'],
      ['p', pubkeys.dan],
    ]
  );

  // A fan writes about the repo too: it is Writing, not the maker's devlog.
  const review = sign(
    'dan',
    30023,
    'I took tidepool out for a week of evening sails. It never once needed a signal.',
    [
      ['d', 'a-week-with-tidepool'],
      ['title', 'A week with tidepool'],
      ['a', repos.tidepool],
    ]
  );

  return {
    articles,
    passages,
    named: { why, cache, series, snippet, review, comment, makerComment },
    events,
  };
}
