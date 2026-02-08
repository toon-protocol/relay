import { decode } from '@toon-format/toon';
import type { NostrEvent } from 'nostr-tools/pure';
import { BlsBaseError } from '../errors.js';

/**
 * Error thrown when TOON decoding or validation fails.
 */
export class ToonError extends BlsBaseError {
  constructor(message: string, cause?: Error) {
    super(message, 'TOON_DECODE_ERROR');
    this.name = 'ToonError';
    if (cause) {
      this.cause = cause;
    }
  }
}

/**
 * Validate that a value is a valid hex string of expected length.
 */
function isValidHex(value: unknown, length: number): value is string {
  if (typeof value !== 'string') return false;
  if (value.length !== length) return false;
  return /^[0-9a-f]+$/i.test(value);
}

/**
 * Validate that a decoded object is a valid NostrEvent.
 */
function validateNostrEvent(obj: unknown): asserts obj is NostrEvent {
  if (typeof obj !== 'object' || obj === null) {
    throw new ToonError('Decoded value is not an object');
  }

  const event = obj as Record<string, unknown>;

  // Validate id (64-char hex)
  if (!isValidHex(event['id'], 64)) {
    throw new ToonError('Invalid event id: must be a 64-character hex string');
  }

  // Validate pubkey (64-char hex)
  if (!isValidHex(event['pubkey'], 64)) {
    throw new ToonError('Invalid event pubkey: must be a 64-character hex string');
  }

  // Validate kind (number)
  if (typeof event['kind'] !== 'number' || !Number.isInteger(event['kind'])) {
    throw new ToonError('Invalid event kind: must be an integer');
  }

  // Validate content (string)
  if (typeof event['content'] !== 'string') {
    throw new ToonError('Invalid event content: must be a string');
  }

  // Validate tags (array of string arrays)
  const tags = event['tags'];
  if (!Array.isArray(tags)) {
    throw new ToonError('Invalid event tags: must be an array');
  }
  for (let i = 0; i < tags.length; i++) {
    const tag = tags[i];
    if (!Array.isArray(tag)) {
      throw new ToonError(`Invalid event tags[${i}]: must be an array`);
    }
    for (let j = 0; j < tag.length; j++) {
      if (typeof tag[j] !== 'string') {
        throw new ToonError(`Invalid event tags[${i}][${j}]: must be a string`);
      }
    }
  }

  // Validate created_at (number)
  if (typeof event['created_at'] !== 'number' || !Number.isInteger(event['created_at'])) {
    throw new ToonError('Invalid event created_at: must be an integer');
  }

  // Validate sig (128-char hex)
  if (!isValidHex(event['sig'], 128)) {
    throw new ToonError('Invalid event sig: must be a 128-character hex string');
  }
}

/**
 * Decode a TOON-encoded Uint8Array back to a NostrEvent.
 *
 * Used for extracting Nostr events from ILP packets.
 *
 * @param data - The TOON-encoded Uint8Array
 * @returns The decoded NostrEvent
 * @throws ToonError if decoding or validation fails
 */
export function decodeEventFromToon(data: Uint8Array): NostrEvent {
  try {
    const toonString = new TextDecoder().decode(data);
    const decoded = decode(toonString);
    validateNostrEvent(decoded);
    return decoded;
  } catch (error) {
    if (error instanceof ToonError) {
      throw error;
    }
    throw new ToonError(
      `Failed to decode TOON data: ${error instanceof Error ? error.message : String(error)}`,
      error instanceof Error ? error : undefined
    );
  }
}
