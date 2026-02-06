import { encode } from '@toon-format/toon';
import type { NostrEvent } from 'nostr-tools/pure';
import { RelayError } from '../storage/index.js';

/**
 * Error thrown when TOON encoding fails.
 */
export class ToonEncodeError extends RelayError {
  constructor(message: string, cause?: Error) {
    super(message, 'TOON_ENCODE_ERROR');
    this.name = 'ToonEncodeError';
    if (cause) {
      this.cause = cause;
    }
  }
}

/**
 * Encode a NostrEvent to TOON format as a Uint8Array.
 *
 * Used for embedding Nostr events in ILP packets where compact encoding
 * reduces bytes and cost.
 *
 * @param event - The NostrEvent to encode
 * @returns Uint8Array containing the TOON-encoded event
 * @throws ToonEncodeError if encoding fails
 */
export function encodeEventToToon(event: NostrEvent): Uint8Array {
  try {
    const toonString = encode(event);
    return new TextEncoder().encode(toonString);
  } catch (error) {
    throw new ToonEncodeError(
      `Failed to encode event to TOON: ${error instanceof Error ? error.message : String(error)}`,
      error instanceof Error ? error : undefined
    );
  }
}
