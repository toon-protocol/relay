import { afterAll, beforeAll, describe, expect } from 'vitest';
import { getPublicKey } from 'nostr-tools/pure';
import {
  Client,
  generateSecretKey,
  publish,
  signed,
  sleep,
  type Frame,
} from './harness/client.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument, waitForEdge } from './harness/nip11.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

let plain: RunningRelay;
let groups: RunningRelay;

beforeAll(async () => {
  plain = await startRelay(imageUnderTest());
  groups = await startRelay(imageUnderTest(), {
    env: { TOON_NIP29_GROUPS: 'true' },
  });
});

afterAll(async () => {
  await plain?.stop();
  await groups?.stop();
});

const CREATE = 9007;
const PUT_USER = 9000;
const REMOVE_USER = 9001;
const EDIT_METADATA = 9002;
const DELETE_EVENT = 9005;
const DELETE_GROUP = 9008;
const JOIN = 9021;
const LEAVE = 9022;
const CHAT = 9;

let counter = 0;
/** A group id nothing else in this run has used. */
const freshId = (): string =>
  `g${Date.now().toString(36)}${(counter++).toString(36)}`;

/** POST an event to the write port; the status says whether it was accepted. */
async function write(relay: RunningRelay, event: object): Promise<number> {
  const response = await fetch(`${relay.writeUrl}/write`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ event }),
  });
  return response.status;
}

/** An event in `group` from `key`, with `extra` tags. */
function inGroup(
  key: Uint8Array,
  kind: number,
  group: string,
  extra: string[][] = []
) {
  return signed(key, { kind, tags: [['h', group], ...extra] });
}

/**
 * A chat message in `group` that is not the one `inGroup(key, CHAT, group)`
 * signed earlier in the same second: that one is the same event, and a retry
 * of an event already held is answered as stored.
 */
function laterChat(key: Uint8Array, group: string) {
  return signed(key, {
    kind: CHAT,
    tags: [['h', group]],
    content: 'a later message',
  });
}

/** Create `group` as `owner`, with `extra` tags (private, closed, name…). */
async function createGroup(
  relay: RunningRelay,
  owner: Uint8Array,
  group = freshId(),
  extra: string[][] = []
): Promise<string> {
  await publish(relay.writeUrl, inGroup(owner, CREATE, group, extra));
  return group;
}

async function withClient(
  relay: RunningRelay,
  body: (client: Client) => Promise<void>
): Promise<void> {
  const client = await Client.connect(relay.readWsUrl);
  try {
    await body(client);
  } finally {
    client.close();
  }
}

/** Answer the connection's NIP-42 challenge as `key`. */
async function authenticate(
  relay: RunningRelay,
  client: Client,
  key: Uint8Array
): Promise<void> {
  const challenge = await client.next((f) => f[0] === 'AUTH');
  const auth = signed(key, {
    kind: 22242,
    tags: [
      ['relay', relay.readWsUrl],
      ['challenge', String(challenge[1])],
    ],
    content: '',
  });
  client.send(['AUTH', auth]);
  const ok = await client.next((f) => f[0] === 'OK' && f[1] === auth.id);
  expect(ok[2]).toBe(true);
}

function must<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected a value');
  return value;
}

const tagValue = (event: { tags: string[][] }, name: string) =>
  event.tags.find((t) => t[0] === name)?.[1];

const isClosed = (id: string) => (f: Frame) => f[0] === 'CLOSED' && f[1] === id;

describe('NIP-29: off by default', () => {
  conformanceTest(
    'a relay with nothing set does not list 29 and treats an h tag as any tag',
    async () => {
      await waitForEdge(plain.readUrl);
      const { body } = await getDocument(plain.readUrl);
      expect(body['supported_nips']).not.toContain(29);
      const event = inGroup(generateSecretKey(), CHAT, freshId());
      expect(await write(plain, event)).toBe(200);
    }
  );
});

describe('NIP-29: enabled', () => {
  conformanceTest('lists 29 in supported_nips', async () => {
    await waitForEdge(groups.readUrl);
    const { body } = await getDocument(groups.readUrl);
    expect(body['supported_nips']).toContain(29);
    expect(body['supported_nips']).toContain(42);
  });

  conformanceTest(
    'a group is created by a 9007, whose author becomes its admin',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [
        ['name', 'Agents'],
      ]);
      await withClient(groups, async (client) => {
        const found = await client.req('meta', {
          kinds: [39000, 39001, 39002, 39003],
          '#d': [group],
        });
        const byKind = new Map(found.map((e) => [e.kind, e]));
        expect([...byKind.keys()].sort()).toEqual([39000, 39001, 39002, 39003]);
        const meta = must(byKind.get(39000));
        expect(tagValue(meta, 'name')).toBe('Agents');
        expect(meta.tags.some((t) => t[0] === 'public')).toBe(true);
        const admins = must(byKind.get(39001));
        expect(admins.tags).toContainEqual(
          expect.arrayContaining(['p', getPublicKey(owner), 'admin'])
        );
        const members = must(byKind.get(39002));
        expect(members.tags.map((t) => t[1])).toContain(getPublicKey(owner));
      });
    }
  );

  conformanceTest(
    'creating a group that exists, or with no valid id, is refused',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner);
      expect(
        await write(groups, inGroup(generateSecretKey(), CREATE, group))
      ).toBe(409);
      expect(
        await write(groups, inGroup(owner, CREATE, 'Not A Valid Id!'))
      ).toBe(422);
      expect(await write(groups, signed(owner, { kind: CREATE }))).toBe(422);
    }
  );

  conformanceTest(
    'a write to a group that does not exist is refused',
    async () => {
      expect(
        await write(groups, inGroup(generateSecretKey(), CHAT, freshId()))
      ).toBe(404);
    }
  );

  conformanceTest(
    'only a member may write to a group, judged by the event author',
    async () => {
      const owner = generateSecretKey();
      const outsider = generateSecretKey();
      const group = await createGroup(groups, owner);
      expect(await write(groups, inGroup(outsider, CHAT, group))).toBe(403);
      expect(await write(groups, inGroup(owner, CHAT, group))).toBe(200);
      // The group's admin adds the outsider; then they may write.
      await publish(
        groups.writeUrl,
        inGroup(owner, PUT_USER, group, [['p', getPublicKey(outsider)]])
      );
      expect(await write(groups, inGroup(outsider, CHAT, group))).toBe(200);
      await withClient(groups, async (client) => {
        const found = await client.req('chat', {
          kinds: [CHAT],
          '#h': [group],
        });
        expect(found.map((e) => e.pubkey).sort()).toEqual(
          [getPublicKey(owner), getPublicKey(outsider)].sort()
        );
      });
    }
  );

  conformanceTest(
    'moderation events need an admin; a member cannot add, remove or edit',
    async () => {
      const owner = generateSecretKey();
      const member = generateSecretKey();
      const other = generateSecretKey();
      const group = await createGroup(groups, owner);
      await publish(
        groups.writeUrl,
        inGroup(owner, PUT_USER, group, [['p', getPublicKey(member)]])
      );
      for (const event of [
        inGroup(member, PUT_USER, group, [['p', getPublicKey(other)]]),
        inGroup(member, PUT_USER, group, [
          ['p', getPublicKey(member), 'admin'],
        ]),
        inGroup(member, REMOVE_USER, group, [['p', getPublicKey(owner)]]),
        inGroup(member, EDIT_METADATA, group, [['name', 'mine now']]),
        inGroup(member, DELETE_GROUP, group),
        inGroup(other, PUT_USER, group, [['p', getPublicKey(other)]]),
      ]) {
        expect(await write(groups, event), JSON.stringify(event)).toBe(403);
      }
    }
  );

  conformanceTest(
    'removing a user ends their right to write and updates the members list',
    async () => {
      const owner = generateSecretKey();
      const member = generateSecretKey();
      const group = await createGroup(groups, owner);
      await publish(
        groups.writeUrl,
        inGroup(owner, PUT_USER, group, [['p', getPublicKey(member)]])
      );
      expect(await write(groups, inGroup(member, CHAT, group))).toBe(200);
      await publish(
        groups.writeUrl,
        inGroup(owner, REMOVE_USER, group, [['p', getPublicKey(member)]])
      );
      expect(await write(groups, laterChat(member, group))).toBe(403);
      await withClient(groups, async (client) => {
        const [members] = await client.req('members', {
          kinds: [39002],
          '#d': [group],
        });
        expect(members?.tags.map((t) => t[1])).not.toContain(
          getPublicKey(member)
        );
      });
    }
  );

  conformanceTest('editing the metadata replaces the 39000 event', async () => {
    const owner = generateSecretKey();
    const group = await createGroup(groups, owner);
    await publish(
      groups.writeUrl,
      inGroup(owner, EDIT_METADATA, group, [
        ['name', 'Renamed'],
        ['about', 'for agents'],
        ['closed'],
      ])
    );
    await withClient(groups, async (client) => {
      const found = await client.req('meta', {
        kinds: [39000],
        '#d': [group],
      });
      expect(found).toHaveLength(1);
      expect(tagValue(must(found[0]), 'name')).toBe('Renamed');
      expect(tagValue(must(found[0]), 'about')).toBe('for agents');
      expect(must(found[0]).tags.some((t) => t[0] === 'closed')).toBe(true);
    });
  });

  conformanceTest(
    'clients cannot write the relay-generated metadata kinds',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner);
      for (const kind of [39000, 39001, 39002, 39003]) {
        expect(
          await write(groups, signed(owner, { kind, tags: [['d', group]] }))
        ).toBe(403);
      }
    }
  );

  conformanceTest(
    'a join request is accepted at once into an open group and kept out of a closed one',
    async () => {
      const owner = generateSecretKey();
      const joiner = generateSecretKey();
      const open = await createGroup(groups, owner);
      const closed = await createGroup(groups, owner, freshId(), [['closed']]);
      await publish(groups.writeUrl, inGroup(joiner, JOIN, open));
      await publish(groups.writeUrl, inGroup(joiner, JOIN, closed));
      expect(await write(groups, inGroup(joiner, CHAT, open))).toBe(200);
      expect(await write(groups, inGroup(joiner, CHAT, closed))).toBe(403);
    }
  );

  conformanceTest('a member who leaves can no longer write', async () => {
    const owner = generateSecretKey();
    const joiner = generateSecretKey();
    const group = await createGroup(groups, owner);
    await publish(groups.writeUrl, inGroup(joiner, JOIN, group));
    expect(await write(groups, inGroup(joiner, CHAT, group))).toBe(200);
    await publish(groups.writeUrl, inGroup(joiner, LEAVE, group));
    expect(await write(groups, laterChat(joiner, group))).toBe(403);
  });

  conformanceTest(
    'a 9005 by an admin deletes an event of the group',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner);
      const message = inGroup(owner, CHAT, group);
      await publish(groups.writeUrl, message);
      await publish(
        groups.writeUrl,
        inGroup(owner, DELETE_EVENT, group, [['e', message.id]])
      );
      await withClient(groups, async (client) => {
        const found = await client.req('chat', {
          kinds: [CHAT],
          '#h': [group],
        });
        expect(found.map((e) => e.id)).not.toContain(message.id);
      });
    }
  );

  conformanceTest(
    'deleting a group removes it, its events and its metadata',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner);
      await publish(groups.writeUrl, inGroup(owner, CHAT, group));
      await publish(groups.writeUrl, inGroup(owner, DELETE_GROUP, group));
      expect(await write(groups, inGroup(owner, CHAT, group))).toBe(404);
      await withClient(groups, async (client) => {
        expect(await client.req('chat', { '#h': [group] })).toEqual([]);
        expect(
          await client.req('meta', {
            kinds: [39000, 39001, 39002, 39003],
            '#d': [group],
          })
        ).toEqual([]);
      });
    }
  );

  conformanceTest(
    'a live subscription to a group is sent its new events and metadata changes',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner);
      await withClient(groups, async (client) => {
        await client.req(
          'live',
          { '#h': [group] },
          { kinds: [39002], '#d': [group] }
        );
        const message = inGroup(owner, CHAT, group);
        await publish(groups.writeUrl, message);
        await client.next(
          (f) => f[0] === 'EVENT' && (f[2] as { id: string }).id === message.id
        );
        const joiner = generateSecretKey();
        await publish(groups.writeUrl, inGroup(joiner, JOIN, group));
        const updated = await client.next(
          (f) => f[0] === 'EVENT' && (f[2] as { kind: number }).kind === 39002
        );
        expect(
          (updated[2] as { tags: string[][] }).tags.map((t) => t[1])
        ).toContain(getPublicKey(joiner));
      });
    }
  );
});

describe('NIP-29: reading closed and private groups', () => {
  conformanceTest(
    'a REQ for a closed group is closed auth-required until the connection authenticates',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [['closed']]);
      await publish(groups.writeUrl, inGroup(owner, CHAT, group));
      await withClient(groups, async (client) => {
        client.send(['REQ', 'closed', { '#h': [group] }]);
        const closed = await client.next(isClosed('closed'));
        expect(String(closed[2])).toMatch(/^auth-required:/);
        expect(client.frames.some((f) => f[0] === 'EOSE')).toBe(false);
      });
    }
  );

  conformanceTest(
    'a member that has authenticated reads a closed group',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [['closed']]);
      const message = inGroup(owner, CHAT, group);
      await publish(groups.writeUrl, message);
      await withClient(groups, async (client) => {
        await authenticate(groups, client, owner);
        const found = await client.req('closed', { '#h': [group] });
        expect(found.map((e) => e.id)).toContain(message.id);
      });
    }
  );

  conformanceTest(
    'an authenticated key that is not a member is refused a closed group',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [['closed']]);
      await publish(groups.writeUrl, inGroup(owner, CHAT, group));
      await withClient(groups, async (client) => {
        await authenticate(groups, client, generateSecretKey());
        client.send(['REQ', 'closed', { '#h': [group] }]);
        const closed = await client.next(isClosed('closed'));
        expect(String(closed[2])).toMatch(/^restricted:/);
      });
    }
  );

  conformanceTest(
    "a closed group's events never reach a reader through a filter that does not name it",
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [['closed']]);
      const message = inGroup(owner, CHAT, group);
      await publish(groups.writeUrl, message);
      await withClient(groups, async (client) => {
        await client.req('live', { kinds: [CHAT] });
        const found = await client.req('sweep', { kinds: [CHAT] });
        expect(found.map((e) => e.id)).not.toContain(message.id);
        const second = inGroup(owner, CHAT, group);
        await publish(groups.writeUrl, second);
        await sleep(500);
        expect(
          client.frames.some(
            (f) => f[0] === 'EVENT' && (f[2] as { id: string }).id === second.id
          )
        ).toBe(false);
      });
    }
  );

  conformanceTest(
    'a private group hides its metadata from a reader who is not a member',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [['private']]);
      await withClient(groups, async (client) => {
        expect(
          await client.req('meta', {
            kinds: [39000, 39001, 39002, 39003],
            '#d': [group],
          })
        ).toEqual([]);
      });
      await withClient(groups, async (client) => {
        await authenticate(groups, client, owner);
        const found = await client.req('meta', {
          kinds: [39000],
          '#d': [group],
        });
        expect(found).toHaveLength(1);
      });
    }
  );

  conformanceTest(
    'a closed group stays readable for a member after the relay is asked again later',
    async () => {
      const owner = generateSecretKey();
      const group = await createGroup(groups, owner, freshId(), [['closed']]);
      await withClient(groups, async (client) => {
        await authenticate(groups, client, owner);
        await client.req('live', { '#h': [group] });
        const message = inGroup(owner, CHAT, group);
        await publish(groups.writeUrl, message);
        await client.next(
          (f) => f[0] === 'EVENT' && (f[2] as { id: string }).id === message.id
        );
      });
    }
  );
});
