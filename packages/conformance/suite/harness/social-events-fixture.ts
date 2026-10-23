import type { Event } from 'nostr-tools';
import { finalizeEvent } from 'nostr-tools/pure';
import type { Person, SocialFixture } from './social-fixture.js';

/**
 * Events and rooms for the /social fixture's community, as signed events:
 * a hackathon that has ended (its room, RSVPs, submissions, badges and
 * awards), a build night that is live now with a NIP-53 stream, an upcoming
 * launch day and a date-based meetup, a private group, and a repo's room.
 *
 * The rooms are NIP-29 groups: they hold together only on a relay started
 * with TOON_NIP29_GROUPS=true, where the relay signs their metadata. Write
 * these after the base fixture's events (they reuse its people and repos).
 * The data is made up; the media URLs are placeholder images from
 * picsum.photos.
 */

export interface SocialEventsFixture {
  /** `<kind>:<pubkey>:<d>` of each calendar event. */
  calendar: {
    hackathon: string;
    buildNight: string;
    launchDay: string;
    meetup: string;
  };
  /** The `/social/ev/<npub>/<d>` path of each calendar event. */
  paths: {
    hackathon: string;
    buildNight: string;
    launchDay: string;
    meetup: string;
  };
  /** Group ids. */
  rooms: { hackathon: string; secret: string; tidepool: string };
  /** The `30617:<owner>:<d>` of the repo made for the hackathon. */
  forge: string;
  /** Events the cases look for, by role. */
  named: {
    doorsOpen: Event;
    remixQuestion: Event;
    remixAnswer: Event;
    judging: Event;
    mayaSubmission: Event;
    danSubmission: Event;
    talk: Event;
    secretMessage: Event;
    secretNote: Event;
    stream: Event;
  };
  /** Every event, in the order to write them. */
  events: Event[];
}

const DAY = 86_400;

/** The day number NIP-52's `D` tag names for a Unix time. */
const dayOf = (seconds: number): string => String(Math.floor(seconds / DAY));

/** `YYYY-MM-DD` of a Unix time, UTC. */
const isoDate = (seconds: number): string =>
  new Date(seconds * 1000).toISOString().slice(0, 10);

export function socialEventsFixture(
  now: number,
  base: SocialFixture
): SocialEventsFixture {
  const { keys, pubkeys, npubs, repos } = base;
  let at = now - 1_800;
  const events: Event[] = [];
  const sign = (
    who: Person,
    kind: number,
    content: string,
    tags: string[][] = []
  ): Event => {
    at += 5;
    const event = finalizeEvent(
      { kind, created_at: at, tags, content },
      keys[who]
    );
    events.push(event);
    return event;
  };
  const image = (seed: string) => `https://picsum.photos/seed/${seed}/1200/675`;

  // The hackathon's room: open and public, made by its host.
  const room = 'shipathon';
  sign('iris', 9007, '', [
    ['h', room],
    ['name', 'shipathon'],
    ['about', 'The Weekend Ship-a-thon room. Post your repo early.'],
  ]);
  for (const who of ['maya', 'leo', 'kofi', 'dan'] as const) {
    sign(who, 9021, '', [['h', room]]);
  }

  // A hackathon that ran from three days ago to yesterday.
  const hackStart = now - 3 * DAY;
  const hackEnd = now - DAY;
  const hackathon = sign(
    'iris',
    31923,
    'Ship something people can run in 48 hours. Post a repo and a ship post that ' +
      'link to this event. Badges for the winners.',
    [
      ['d', 'weekend-shipathon'],
      ['title', 'Weekend Ship-a-thon'],
      ['summary', 'Ship something people can run in 48 hours.'],
      ['image', image('shipathon')],
      ['start', String(hackStart)],
      ['end', String(hackEnd)],
      ['D', dayOf(hackStart)],
      ['D', dayOf(hackStart + DAY)],
      ['start_tzid', 'UTC'],
      ['location', 'online'],
      ['p', pubkeys.kofi, '', 'co-host'],
      ['p', pubkeys.ruth, '', 'judge'],
      ['t', 'hackathon'],
      ['h', room],
    ]
  );
  const hackathonA = `31923:${pubkeys.iris}:weekend-shipathon`;

  const doorsOpen = sign(
    'iris',
    9,
    'Doors open. Post your repo early so it shows in the gallery.',
    [['h', room]]
  );
  const remixQuestion = sign('leo', 9, 'remixing tidepool, is that allowed?', [
    ['h', room],
  ]);
  const remixAnswer = sign('kofi', 9, 'Yes, the lineage shows on your repo.', [
    ['h', room],
    ['q', remixQuestion.id, '', pubkeys.leo],
  ]);
  sign('dan', 9, 'anyone else on wasm audio?', [['h', room]]);
  const judging = sign('kofi', 11, 'What the judges look for: it runs.', [
    ['h', room],
    ['title', 'Judging criteria'],
  ]);

  // RSVPs: three going, one maybe.
  for (const [who, status] of [
    ['maya', 'accepted'],
    ['leo', 'accepted'],
    ['dan', 'accepted'],
    ['ruth', 'tentative'],
  ] as const) {
    sign(who, 31925, '', [
      ['d', `rsvp-${who}-shipathon`],
      ['a', hackathonA],
      ['e', hackathon.id],
      ['status', status],
      ['p', pubkeys.iris],
    ]);
  }

  // Submissions: Maya's ship post for tidepool, Leo's new repo, Dan's ship
  // post with no repo; and a note that names the event, which is talk.
  const mayaSubmission = sign(
    'maya',
    1,
    `Shipped tidepool for the Ship-a-thon: offline tide charts.\n\n${image('tidepool-jam')}`,
    [
      ['a', repos.tidepool],
      ['a', hackathonA],
      ['t', 'ship'],
      [
        'imeta',
        `url ${image('tidepool-jam')}`,
        'm image/jpeg',
        'alt The chart',
      ],
    ]
  );
  sign('leo', 30617, '', [
    ['d', 'pixel-forge'],
    ['name', 'pixel-forge'],
    ['description', 'Turn a sprite sheet into a level.'],
    ['t', 'games'],
    ['a', hackathonA],
  ]);
  const forge = `30617:${pubkeys.leo}:pixel-forge`;
  const danSubmission = sign(
    'dan',
    1,
    `Made a synth in the browser for the Ship-a-thon.\n\n${image('wasm-synth')}`,
    [
      ['a', hackathonA],
      ['t', 'ship'],
      ['imeta', `url ${image('wasm-synth')}`, 'm image/jpeg', 'alt The synth'],
    ]
  );
  const talk = sign('ruth', 1, 'Judging the Ship-a-thon tonight.', [
    ['a', hackathonA],
  ]);

  // Badges: one by each host, and their awards.
  sign('iris', 30009, '', [
    ['d', 'shipathon-winner'],
    ['name', 'Winner'],
    ['description', 'Won the Weekend Ship-a-thon'],
    ['a', hackathonA],
  ]);
  sign('kofi', 30009, '', [
    ['d', 'shipathon-crowd'],
    ['name', 'Crowd pick'],
    ['description', 'The crowd’s favourite'],
    ['a', hackathonA],
  ]);
  sign('iris', 8, '', [
    ['a', `30009:${pubkeys.iris}:shipathon-winner`],
    ['p', pubkeys.maya],
  ]);
  sign('kofi', 8, '', [
    ['a', `30009:${pubkeys.kofi}:shipathon-crowd`],
    ['p', pubkeys.leo],
  ]);
  // A stranger cannot award the host's badge.
  sign('ruth', 8, '', [
    ['a', `30009:${pubkeys.iris}:shipathon-winner`],
    ['p', pubkeys.dan],
  ]);

  // A build night live now, with a stream.
  const buildStart = now - 3_600;
  const buildEnd = now + 3 * 3_600;
  sign('kofi', 31923, 'Hack on gitsail together. Bring a bug.', [
    ['d', 'build-night'],
    ['title', 'gitsail build night'],
    ['start', String(buildStart)],
    ['end', String(buildEnd)],
    ['D', dayOf(buildStart)],
    ['location', 'https://meet.example/gitsail'],
  ]);
  const buildNightA = `31923:${pubkeys.kofi}:build-night`;
  const stream = sign('kofi', 30311, '', [
    ['d', 'build-night-stream'],
    ['title', 'Build night stream'],
    ['status', 'live'],
    ['streaming', 'https://stream.example/build-night.m3u8'],
    ['starts', String(buildStart)],
    ['current_participants', '12'],
    ['p', pubkeys.kofi, '', 'Host'],
    ['a', buildNightA],
  ]);
  sign('dan', 31925, '', [
    ['d', 'rsvp-dan-build'],
    ['a', buildNightA],
    ['status', 'accepted'],
  ]);

  // An upcoming launch day, and a date-based meetup.
  const launchStart = now + 9 * DAY;
  sign('maya', 31923, 'tidepool 2.0 goes live. Come ask questions.', [
    ['d', 'tidepool-launch'],
    ['title', 'tidepool 2.0 launch day'],
    ['start', String(launchStart)],
    ['end', String(launchStart + 2 * 3_600)],
    ['D', dayOf(launchStart)],
    ['start_tzid', 'Europe/Lisbon'],
    ['location', 'https://meet.example/tidepool'],
    ['a', repos.tidepool],
  ]);
  for (const [who, status] of [
    ['dan', 'accepted'],
    ['iris', 'tentative'],
  ] as const) {
    sign(who, 31925, '', [
      ['d', `rsvp-${who}-launch`],
      ['a', `31923:${pubkeys.maya}:tidepool-launch`],
      ['status', status],
    ]);
  }
  sign('iris', 31922, 'Relay operators, in person.', [
    ['d', 'operators-meetup'],
    ['title', 'Relay operators meetup'],
    ['start', isoDate(now + 20 * DAY)],
    ['end', isoDate(now + 21 * DAY)],
    ['location', 'Lisbon'],
  ]);

  // A private group: nothing of it shows on /social.
  sign('iris', 9007, '', [
    ['h', 'judges'],
    ['name', 'Judges only'],
    ['private'],
  ]);
  const secretMessage = sign(
    'iris',
    9,
    'The shortlist is tidepool and pixel-forge.',
    [['h', 'judges']]
  );
  const secretNote = sign('iris', 1, 'A note for the judges alone.', [
    ['h', 'judges'],
  ]);

  // tidepool's room: the group named its d, kept by its owner.
  sign('maya', 9007, '', [
    ['h', 'tidepool'],
    ['name', 'tidepool'],
    ['about', 'Talk about tidepool: tides, stations, bugs.'],
  ]);
  sign('maya', 9, 'Welcome! Ask anything about tidepool here.', [
    ['h', 'tidepool'],
  ]);
  sign('dan', 9021, '', [['h', 'tidepool']]);
  sign('dan', 9, 'Is there a station list for Lisbon?', [['h', 'tidepool']]);

  const path = (npub: string, d: string) => `/social/ev/${npub}/${d}`;
  return {
    calendar: {
      hackathon: hackathonA,
      buildNight: buildNightA,
      launchDay: `31923:${pubkeys.maya}:tidepool-launch`,
      meetup: `31922:${pubkeys.iris}:operators-meetup`,
    },
    paths: {
      hackathon: path(npubs.iris, 'weekend-shipathon'),
      buildNight: path(npubs.kofi, 'build-night'),
      launchDay: path(npubs.maya, 'tidepool-launch'),
      meetup: path(npubs.iris, 'operators-meetup'),
    },
    rooms: { hackathon: room, secret: 'judges', tidepool: 'tidepool' },
    forge,
    named: {
      doorsOpen,
      remixQuestion,
      remixAnswer,
      judging,
      mayaSubmission,
      danSubmission,
      talk,
      secretMessage,
      secretNote,
      stream,
    },
    events,
  };
}
