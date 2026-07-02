/**
 * TOON event codec.
 *
 * Encodes/decodes Nostr events to and from the TOON text format. Vendored from
 * `@toon-protocol/core` so the relay depends only on the lightweight
 * `@toon-format/toon` encoder rather than core's full transitive tree (which
 * pulls Arweave / web3 wallet stacks the relay does not use). Same MIT
 * license / org.
 *
 * NOTE: this codec is NOT used on the relay's NIP-01 read surface. Outbound
 * EVENT frames are canonical NIP-01 JSON (see
 * `websocket/ConnectionHandler.sendEvent`, #46) so that standard nostr clients
 * can parse events and verify signatures from the wire. The codec remains
 * exported for library consumers that exchange TOON-text events elsewhere.
 *
 * @module
 */

import { encode, decode } from '@toon-format/toon';
import type { NostrEvent } from 'nostr-tools/pure';

/** Thrown when a Nostr event cannot be encoded to TOON. */
export class ToonEncodeError extends Error {
  readonly code = 'TOON_ENCODE_ERROR';
  constructor(message: string, cause?: Error) {
    super(message, { cause });
    this.name = 'ToonEncodeError';
  }
}

/** Thrown when TOON data cannot be decoded into a valid Nostr event. */
export class ToonDecodeError extends Error {
  readonly code = 'TOON_DECODE_ERROR';
  constructor(message: string, cause?: Error) {
    super(message, { cause });
    this.name = 'ToonDecodeError';
  }
}

/** Encode a Nostr event to TOON bytes (UTF-8). */
export function encodeEventToToon(event: NostrEvent): Uint8Array {
  return new TextEncoder().encode(encodeEventToToonString(event));
}

/** Encode a Nostr event to a TOON string. */
export function encodeEventToToonString(event: NostrEvent): string {
  try {
    return encode(event);
  } catch (error) {
    throw new ToonEncodeError(
      `Failed to encode event to TOON: ${
        error instanceof Error ? error.message : String(error)
      }`,
      error instanceof Error ? error : undefined
    );
  }
}

function isValidHex(value: unknown, length: number): boolean {
  return (
    typeof value === 'string' &&
    value.length === length &&
    /^[0-9a-f]+$/i.test(value)
  );
}

function validateNostrEvent(obj: unknown): asserts obj is NostrEvent {
  if (typeof obj !== 'object' || obj === null) {
    throw new ToonDecodeError('Decoded value is not an object');
  }
  const event = obj as Record<string, unknown>;
  if (!isValidHex(event['id'], 64)) {
    throw new ToonDecodeError(
      'Invalid event id: must be a 64-character hex string'
    );
  }
  if (!isValidHex(event['pubkey'], 64)) {
    throw new ToonDecodeError(
      'Invalid event pubkey: must be a 64-character hex string'
    );
  }
  if (typeof event['kind'] !== 'number' || !Number.isInteger(event['kind'])) {
    throw new ToonDecodeError('Invalid event kind: must be an integer');
  }
  if (typeof event['content'] !== 'string') {
    throw new ToonDecodeError('Invalid event content: must be a string');
  }
  const tags = event['tags'];
  if (!Array.isArray(tags)) {
    throw new ToonDecodeError('Invalid event tags: must be an array');
  }
  for (let i = 0; i < tags.length; i++) {
    const tag: unknown = tags[i];
    if (!Array.isArray(tag)) {
      throw new ToonDecodeError(`Invalid event tags[${i}]: must be an array`);
    }
    for (let j = 0; j < tag.length; j++) {
      if (typeof tag[j] !== 'string') {
        throw new ToonDecodeError(
          `Invalid event tags[${i}][${j}]: must be a string`
        );
      }
    }
  }
  if (
    typeof event['created_at'] !== 'number' ||
    !Number.isInteger(event['created_at'])
  ) {
    throw new ToonDecodeError('Invalid event created_at: must be an integer');
  }
  if (!isValidHex(event['sig'], 128)) {
    throw new ToonDecodeError(
      'Invalid event sig: must be a 128-character hex string'
    );
  }
}

/** Decode TOON bytes into a validated Nostr event. */
export function decodeEventFromToon(data: Uint8Array): NostrEvent {
  let decoded: unknown;
  try {
    decoded = decode(new TextDecoder().decode(data));
  } catch (error) {
    throw new ToonDecodeError(
      `Failed to decode TOON data: ${
        error instanceof Error ? error.message : String(error)
      }`,
      error instanceof Error ? error : undefined
    );
  }
  validateNostrEvent(decoded);
  return decoded;
}
