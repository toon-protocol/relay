import { describe, it, expect } from 'vitest';
import { encodeEventToToon, decodeEventFromToon, ToonError, ToonEncodeError } from './index.js';
import type { NostrEvent } from 'nostr-tools/pure';

/**
 * Create a test event with optional overrides.
 */
const createTestEvent = (overrides: Partial<NostrEvent> = {}): NostrEvent => ({
  id: 'a'.repeat(64),
  pubkey: 'b'.repeat(64),
  kind: 1,
  content: 'Hello, world!',
  tags: [],
  created_at: 1234567890,
  sig: 'c'.repeat(128),
  ...overrides,
});

/**
 * Create a kind:0 profile metadata event.
 */
const createProfileEvent = (): NostrEvent => ({
  id: 'd'.repeat(64),
  pubkey: 'e'.repeat(64),
  kind: 0,
  content: JSON.stringify({
    name: 'Alice',
    about: 'Nostr enthusiast',
    picture: 'https://example.com/pic.jpg',
  }),
  tags: [],
  created_at: 1234567890,
  sig: 'f'.repeat(128),
});

/**
 * Create a kind:1 text note event.
 */
const createTextNote = (): NostrEvent => ({
  id: 'a'.repeat(64),
  pubkey: 'b'.repeat(64),
  kind: 1,
  content: 'Hello, Nostr!',
  tags: [
    ['e', '1'.repeat(64)],
    ['p', '2'.repeat(64)],
  ],
  created_at: 1234567890,
  sig: 'c'.repeat(128),
});

/**
 * Create a kind:3 follow list event with many tags.
 */
const createFollowList = (): NostrEvent => ({
  id: '1'.repeat(64),
  pubkey: '2'.repeat(64),
  kind: 3,
  content: '',
  tags: Array.from({ length: 50 }, (_, i) => [
    'p',
    '3'.repeat(62) + i.toString(16).padStart(2, '0'),
  ]),
  created_at: 1234567890,
  sig: '4'.repeat(128),
});

/**
 * Create a kind:7 reaction event.
 */
const createReactionEvent = (): NostrEvent => ({
  id: '5'.repeat(64),
  pubkey: '6'.repeat(64),
  kind: 7,
  content: '+',
  tags: [
    ['e', '7'.repeat(64)],
    ['p', '8'.repeat(64)],
  ],
  created_at: 1234567890,
  sig: '9'.repeat(128),
});

/**
 * Create a kind:10032 ILP Peer Info event.
 */
const createIlpPeerInfo = (): NostrEvent => ({
  id: 'aa'.repeat(32),
  pubkey: 'bb'.repeat(32),
  kind: 10032,
  content: JSON.stringify({
    ilpAddress: 'g.agent.alice',
    btpEndpoint: 'ws://localhost:8080',
    assetCode: 'USD',
    assetScale: 9,
  }),
  tags: [],
  created_at: 1234567890,
  sig: 'cc'.repeat(64),
});

/**
 * Create a kind:10047 SPSP Info event.
 */
const createSpspInfo = (): NostrEvent => ({
  id: 'dd'.repeat(32),
  pubkey: 'ee'.repeat(32),
  kind: 10047,
  content: JSON.stringify({
    destination_account: 'g.agent.alice.receiver',
    shared_secret: 'base64secret==',
  }),
  tags: [['d', 'default']],
  created_at: 1234567890,
  sig: 'ff'.repeat(64),
});

describe('TOON Encoding', () => {
  describe('encodeEventToToon', () => {
    it('should encode a simple kind:1 event to Uint8Array', () => {
      const event = createTestEvent();
      const encoded = encodeEventToToon(event);
      expect(encoded).toBeInstanceOf(Uint8Array);
      expect(encoded.length).toBeGreaterThan(0);
    });

    it('should encode an event with empty tags', () => {
      const event = createTestEvent({ tags: [] });
      const encoded = encodeEventToToon(event);
      expect(encoded).toBeInstanceOf(Uint8Array);
      expect(encoded.length).toBeGreaterThan(0);
    });

    it('should encode an event with multiple tags (#e, #p tags)', () => {
      const event = createTextNote();
      const encoded = encodeEventToToon(event);
      expect(encoded).toBeInstanceOf(Uint8Array);
      expect(encoded.length).toBeGreaterThan(0);
    });

    it('should encode an event with special characters in content', () => {
      const event = createTestEvent({
        content: 'Hello\nWorld\t"Quoted"\u{1F600}',
      });
      const encoded = encodeEventToToon(event);
      expect(encoded).toBeInstanceOf(Uint8Array);
      expect(encoded.length).toBeGreaterThan(0);
    });

    it('should encode ILP event kind 10032', () => {
      const event = createIlpPeerInfo();
      const encoded = encodeEventToToon(event);
      expect(encoded).toBeInstanceOf(Uint8Array);
      expect(encoded.length).toBeGreaterThan(0);
    });

    it('should encode ILP event kind 10047', () => {
      const event = createSpspInfo();
      const encoded = encodeEventToToon(event);
      expect(encoded).toBeInstanceOf(Uint8Array);
      expect(encoded.length).toBeGreaterThan(0);
    });

    it('should produce reasonable output size for events with many uniform tags', () => {
      // Create a large follow list with 200 entries to test TOON efficiency
      const largeFollowList: NostrEvent = {
        id: '1'.repeat(64),
        pubkey: '2'.repeat(64),
        kind: 3,
        content: '',
        tags: Array.from({ length: 200 }, (_, i) => [
          'p',
          '3'.repeat(62) + i.toString(16).padStart(2, '0'),
        ]),
        created_at: 1234567890,
        sig: '4'.repeat(128),
      };
      const encoded = encodeEventToToon(largeFollowList);
      const jsonBytes = new TextEncoder().encode(JSON.stringify(largeFollowList));
      // TOON encoding should work and produce output
      expect(encoded.length).toBeGreaterThan(0);
      // For very large uniform arrays, TOON should approach or beat JSON size
      // The ratio should be reasonable (within 2x of JSON size)
      expect(encoded.length).toBeLessThan(jsonBytes.length * 2);
    });

    it('should throw ToonEncodeError for circular references', () => {
      // Create an object with circular reference
      const circular: Record<string, unknown> = { id: 'a'.repeat(64) };
      circular['self'] = circular;

      expect(() => encodeEventToToon(circular as unknown as NostrEvent)).toThrow(
        ToonEncodeError
      );
    });
  });

  describe('decodeEventFromToon', () => {
    it('should decode a simple encoded event', () => {
      const event = createTestEvent();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should decode an event with empty tags', () => {
      const event = createTestEvent({ tags: [] });
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should decode an event with multiple tags', () => {
      const event = createTextNote();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should decode an event with special characters in content', () => {
      const event = createTestEvent({
        content: 'Hello\nWorld\t"Quoted"\u{1F600}',
      });
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should decode ILP event kind 10032', () => {
      const event = createIlpPeerInfo();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should decode ILP event kind 10047', () => {
      const event = createSpspInfo();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should throw ToonError for invalid TOON data', () => {
      const invalidData = new TextEncoder().encode('not valid toon {{{');
      expect(() => decodeEventFromToon(invalidData)).toThrow(ToonError);
    });

    it('should throw ToonError for malformed event (missing id)', () => {
      const event = createTestEvent();
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { id, ...eventWithoutId } = event;
      const encoded = encodeEventToToon(eventWithoutId as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
      expect(() => decodeEventFromToon(encoded)).toThrow(
        'Invalid event id: must be a 64-character hex string'
      );
    });

    it('should throw ToonError for malformed event (missing pubkey)', () => {
      const event = createTestEvent();
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { pubkey, ...eventWithoutPubkey } = event;
      const encoded = encodeEventToToon(eventWithoutPubkey as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
    });

    it('should throw ToonError for invalid field types (kind is not a number)', () => {
      const invalidEvent = { ...createTestEvent(), kind: 'not-a-number' };
      const encoded = encodeEventToToon(invalidEvent as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
      expect(() => decodeEventFromToon(encoded)).toThrow(
        'Invalid event kind: must be an integer'
      );
    });

    it('should throw ToonError for invalid id length', () => {
      const invalidEvent = { ...createTestEvent(), id: 'short' };
      const encoded = encodeEventToToon(invalidEvent as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
    });

    it('should throw ToonError for invalid sig length', () => {
      const invalidEvent = { ...createTestEvent(), sig: 'short' };
      const encoded = encodeEventToToon(invalidEvent as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
    });

    it('should throw ToonError for invalid tags (not an array)', () => {
      const invalidEvent = { ...createTestEvent(), tags: 'not-an-array' };
      const encoded = encodeEventToToon(invalidEvent as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
      expect(() => decodeEventFromToon(encoded)).toThrow(
        'Invalid event tags: must be an array'
      );
    });

    it('should throw ToonError for invalid tag element (not a string)', () => {
      const invalidEvent = { ...createTestEvent(), tags: [['e', 123]] };
      const encoded = encodeEventToToon(invalidEvent as unknown as NostrEvent);
      expect(() => decodeEventFromToon(encoded)).toThrow(ToonError);
    });
  });

  describe('Round-trip tests', () => {
    it('should round-trip kind:0 (profile metadata) event', () => {
      const event = createProfileEvent();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should round-trip kind:1 (text note) event with content', () => {
      const event = createTextNote();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should round-trip kind:3 (follow list) event with many tags', () => {
      const event = createFollowList();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should round-trip kind:7 (reaction) event', () => {
      const event = createReactionEvent();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should round-trip kind:10032 (ILP Peer Info) event', () => {
      const event = createIlpPeerInfo();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should round-trip kind:10047 (SPSP Info) event', () => {
      const event = createSpspInfo();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded).toEqual(event);
    });

    it('should preserve event signature through round-trip', () => {
      const event = createTestEvent();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);
      expect(decoded.sig).toBe(event.sig);
    });

    it('should preserve all 7 event fields through round-trip', () => {
      const event = createTestEvent();
      const encoded = encodeEventToToon(event);
      const decoded = decodeEventFromToon(encoded);

      expect(decoded.id).toBe(event.id);
      expect(decoded.pubkey).toBe(event.pubkey);
      expect(decoded.kind).toBe(event.kind);
      expect(decoded.content).toBe(event.content);
      expect(decoded.tags).toEqual(event.tags);
      expect(decoded.created_at).toBe(event.created_at);
      expect(decoded.sig).toBe(event.sig);
    });
  });
});
