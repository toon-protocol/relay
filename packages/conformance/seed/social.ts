// Load the /social fixture (suite/harness/social-fixture.ts) into a relay
// running on this machine, so a person can browse /social with realistic
// data: profiles, two repos and a Shakespeare app, stars, follows, a ship
// post with screenshots, mentions by naddr, a boost, comments, a thread, a
// highlight, an article, a picture, a snippet, an issue and a merge; then a
// month of stars, forks, topic labels and NIP-85 ranks for /social/trending
// (suite/harness/trending-fixture.ts); then the builder profile's banner,
// status, identities, pins, badge, year of activity and bot
// (suite/harness/profile-fixture.ts).
//
//   WRITE_URL=http://127.0.0.1:3100 READ_URL=http://127.0.0.1:7100 \
//     pnpm exec tsx packages/conformance/seed/social.ts
//
// Each event is POSTed to the relay's /write as the connector would deliver
// it, so the relay must accept a write that carries no payment statement
// (any relay started without a connector does). The events are created in
// the hour before now, and the keys are fixed, so seeding twice stores the
// same people with a second set of posts. RELAY_HOST (default the host of
// READ_URL) is what the fixture's site manifest names as its relay.
import { socialFixture } from '../suite/harness/social-fixture.js';
import { trendingFixture } from '../suite/harness/trending-fixture.js';
import { profileFixture } from '../suite/harness/profile-fixture.js';

const writeUrl = process.env['WRITE_URL'] ?? 'http://127.0.0.1:3100';
const readUrl = process.env['READ_URL'] ?? 'http://127.0.0.1:7100';
const relayHost = process.env['RELAY_HOST'] ?? new URL(readUrl).host;

const now = Math.floor(Date.now() / 1000);
const fixture = socialFixture(now, relayHost);
// A month of activity for /social/trending and the topic pages.
const trending = trendingFixture(fixture, now);
// A banner, status, identities, pins, a badge, a year of activity, a bot.
const profile = profileFixture(fixture, now);
const events = [...fixture.events, ...trending.events, ...profile.events];
let stored = 0;
for (const event of events) {
  const response = await fetch(`${writeUrl}/write`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ event }),
  });
  const body = await response.text();
  if (response.status !== 200) {
    console.error(
      `kind ${event.kind} ${event.id}: HTTP ${response.status} ${body}`
    );
    process.exitCode = 1;
    continue;
  }
  stored += 1;
}
console.log(`stored ${stored} of ${events.length} events`);
console.log('');
console.log(`feed:          ${readUrl}/social`);
console.log(`following:     ${readUrl}/social/following/${fixture.npubs.dan}`);
console.log(
  `repo page:     ${readUrl}/social/r/${fixture.npubs.maya}/tidepool`
);
console.log(`ship post:     ${readUrl}/social/e/${fixture.named.ship.id}`);
console.log(`app launch:    ${readUrl}/social/e/${fixture.named.app.id}`);
console.log(`on /repo:      ${readUrl}/repo/${fixture.npubs.maya}/tidepool`);
console.log(`trending:      ${readUrl}/social/trending`);
console.log(
  `weighted:      ${readUrl}/social/trending?window=month&as=${trending.npubs.quinn}`
);
console.log(`topics:        ${readUrl}/social/t`);
console.log(`a topic:       ${readUrl}/social/t/built-with%3Aclaude`);
console.log(`profile:       ${readUrl}/social/p/${fixture.npubs.maya}`);
console.log(`a bot:         ${readUrl}/social/p/${profile.bot.npub}`);
