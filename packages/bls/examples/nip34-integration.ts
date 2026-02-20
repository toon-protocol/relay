/**
 * Example: Integrating NIP-34 Git Handler with BLS
 *
 * This example shows how to configure the BusinessLogicServer
 * to automatically process NIP-34 events and execute Git operations
 * on a Forgejo instance.
 */

import { BusinessLogicServer } from '../src/bls/BusinessLogicServer.js';
import { InMemoryEventStore } from '../src/storage/InMemoryEventStore.js';
import { NIP34Handler } from '@crosstown/core';

/**
 * Example configuration for NIP-34 integration
 */
async function createBLSWithNIP34() {
  // Initialize NIP-34 handler
  const nip34Handler = new NIP34Handler({
    forgejoUrl: process.env.FORGEJO_URL || 'http://forgejo:3000',
    forgejoToken: process.env.FORGEJO_TOKEN || '',
    defaultOwner: process.env.FORGEJO_OWNER || 'crosstown',
    gitConfig: {
      userName: 'Crosstown Node',
      userEmail: 'node@crosstown.nostr',
    },
    verbose: true,
  });

  // Create BLS with NIP-34 handler
  const bls = new BusinessLogicServer(
    {
      basePricePerByte: 10n,
      spspMinPrice: 0n, // Free SPSP requests
      ownerPubkey: process.env.OWNER_PUBKEY, // Optional: bypass payment for owner

      // NIP-34 integration: called after storing NIP-34 events
      onNIP34Event: async (event) => {
        console.log(
          `Processing NIP-34 event: kind=${event.kind} id=${event.id.substring(0, 8)}`
        );

        const result = await nip34Handler.handleEvent(event);

        if (result.success) {
          console.log(`✓ ${result.operation}: ${result.message}`);
          if (result.metadata) {
            console.log('  Metadata:', result.metadata);
          }
        } else {
          console.error(`✗ ${result.operation}: ${result.message}`);
        }
      },
    },
    new InMemoryEventStore()
  );

  return bls;
}

/**
 * Example: Handling different NIP-34 event types
 */
async function handleNIP34Events() {
  const bls = await createBLSWithNIP34();

  // Example 1: Repository Announcement (kind 30617)
  // User sends ILP payment with TOON-encoded event:
  // {
  //   "kind": 30617,
  //   "tags": [
  //     ["d", "my-awesome-repo"],
  //     ["name", "My Awesome Repo"],
  //     ["description", "A cool project built on Nostr"],
  //   ],
  //   ...
  // }
  // → BLS validates payment
  // → Stores event
  // → Calls NIP34Handler.handleEvent()
  // → Creates repository in Forgejo: http://forgejo:3000/crosstown/my-awesome-repo

  // Example 2: Patch Submission (kind 1617)
  // User sends ILP payment with patch event:
  // {
  //   "kind": 1617,
  //   "tags": [
  //     ["a", "30617:<pubkey>:my-awesome-repo"],
  //     ["commit", "<sha>"],
  //   ],
  //   "content": "<git format-patch output>"
  // }
  // → BLS validates payment
  // → Stores event
  // → Calls NIP34Handler.handleEvent()
  // → Clones repo, applies patch, creates PR in Forgejo

  // Example 3: Issue Creation (kind 1621)
  // User sends ILP payment with issue event:
  // {
  //   "kind": 1621,
  //   "tags": [
  //     ["a", "30617:<pubkey>:my-awesome-repo"],
  //     ["subject", "Bug: Something is broken"],
  //   ],
  //   "content": "## Description\n\nI found a bug..."
  // }
  // → BLS validates payment
  // → Stores event
  // → Calls NIP34Handler.handleEvent()
  // → Creates issue in Forgejo

  console.log('BLS with NIP-34 integration ready!');
  console.log('NIP-34 events will automatically trigger Git operations.');

  return bls;
}

/**
 * Example: Environment variable configuration
 */
function exampleEnvConfig() {
  return `
# Forgejo Configuration
FORGEJO_URL=http://forgejo:3000
FORGEJO_TOKEN=your-forgejo-api-token-here
FORGEJO_OWNER=crosstown

# Optional: Owner pubkey for bypassing payments
OWNER_PUBKEY=your-nostr-pubkey-hex

# BLS Configuration
BLS_PORT=3100
BASE_PRICE_PER_BYTE=10
SPSP_MIN_PRICE=0
  `.trim();
}

/**
 * Example: Docker Compose Integration
 */
function exampleDockerCompose() {
  return `
services:
  crosstown:
    image: crosstown:latest
    environment:
      # NIP-34 Configuration
      FORGEJO_URL: http://forgejo:3000
      FORGEJO_TOKEN: \${FORGEJO_TOKEN}
      FORGEJO_OWNER: crosstown

      # Other BLS config...
      BLS_PORT: 3100
      BASE_PRICE_PER_BYTE: 10
    depends_on:
      - forgejo
    networks:
      - crosstown-network

  forgejo:
    image: codeberg.org/forgejo/forgejo:14
    container_name: crosstown-forgejo
    ports:
      - "3003:3000"
    volumes:
      - forgejo-data:/data
    networks:
      - crosstown-network
  `.trim();
}

// Export examples
export {
  createBLSWithNIP34,
  handleNIP34Events,
  exampleEnvConfig,
  exampleDockerCompose,
};
