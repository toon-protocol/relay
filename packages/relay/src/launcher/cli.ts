#!/usr/bin/env node

/**
 * CLI entrypoint for @toon-protocol/relay.
 *
 * Thin wrapper around startRelay() that parses CLI flags and environment
 * variables, then delegates to relay.ts. The relay is a plain read/write app:
 * free NIP-01 WebSocket reads plus an HTTP `POST /write` surface. Payment is
 * enforced upstream by an external terminator, so there are no connector,
 * ILP, chain, or pricing options here.
 *
 * Usage:
 *   relay --secret-key <hex>
 *   NOSTR_SECRET_KEY=<hex> relay
 *
 * Environment variables override defaults; CLI flags override environment variables.
 */

import { parseArgs } from 'node:util';
import { startRelay } from './relay.js';
import type { RelayConfig, RelayInstance } from './relay.js';
import { DEFAULT_EPHEMERAL_RATE_LIMIT } from './handlers/write-ephemeral-handler.js';
import { parseBlockedEventIds } from '../nips/blocklist.js';

// ---------- CLI Parsing ----------

function printHelp(): void {
  console.log(
    `
Usage: relay [options]

Options:
  --mnemonic <words>       BIP-39 mnemonic (12 or 24 words; NIP-06 derivation)
  --secret-key <hex>       32-byte secret key in hex
  --relay-port <port>      WebSocket read port (default: 7100)
  --bls-port <port>        HTTP write/health port (default: 3100)
  --host <host>            WebSocket bind host (default: 0.0.0.0)
  --write-host <host>      HTTP write/health bind host (default: 0.0.0.0).
                           The write port must only be reachable via the
                           payment-gating connector; bind it to a loopback/
                           internal address when not isolated by docker
                           networking (relay#85 exposure guard)
  --data-dir <path>        Data directory for the SQLite store (default: ./data)
  --dev-mode               Skip event-signature verification on POST /write
  --verify-ephemeral       Run FULL schnorr verification on ephemeral kinds
                           (20000 <= kind < 30000). Default OFF: the write
                           path is payment-gated and clients verify
                           signatures themselves, so ephemeral kinds skip
                           schnorr and keep only the SHA-256 id check
                           (relay#85). Set this when the write port is
                           fronted by anything other than a payment-gating
                           connector
  --verify-workers <n>     Worker threads for persistent-kind signature
                           verification (default: CPU count - 1, minimum 0).
                           0 = verify inline on the event loop (automatic on
                           1-core boxes; the escape hatch back to the
                           pre-pool behavior, relay#85)
  --max-connections <n>    Maximum concurrent WebSocket read connections
                           (default: 4096; each costs one file descriptor --
                           mind ulimit -n, relay#90)
  --ephemeral-rate-limit <n>
                           Free ephemeral write lane (POST /write-ephemeral,
                           relay#129): max requests per key per window
                           (default: 200). This lane has no payment gate, so
                           this bound IS its admission control
  --ephemeral-rate-window-ms <n>
                           Free ephemeral write lane: rate-limit window in
                           milliseconds (default: 10000)
  --ephemeral-max-body-bytes <n>
                           Free ephemeral write lane: request body size cap
                           in bytes (default: 8192)
  --log-writes             Log one line per accepted POST /write (debug; off
                           by default -- per-event logging is write-path
                           tail jitter, relay#85)
  --no-enforce-expiration  Serve events past their NIP-40 expiration tag.
                           KILL SWITCH back to the pre-relay#137 behaviour;
                           enforcement is ON by default. Also disables the
                           reaper -- a relay still serving expired events
                           must not be silently deleting them
  --expiration-reap-grace-seconds <n>
                           How long an expired event stays on disk before the
                           reaper deletes it (default: 86400). The window in
                           which flipping enforcement back off is a real
                           recovery rather than an apology. 0 = reap at once
  --expiration-reap-interval-seconds <n>
                           Reaper sweep interval (default: 3600). 0 disables
                           reaping; serve-time filtering is unaffected
  --blocked-event-ids <ids>
                           Comma-separated 64-hex event ids this relay
                           refuses to store or serve. The escape hatch for an
                           event whose author key is gone, so neither NIP-01
                           replacement nor NIP-09 deletion can reach it. Ids
                           only, never pubkeys; every id is logged at startup
  --help                   Show this help message

Environment Variables:
  TOON_MNEMONIC            Same as --mnemonic
  TOON_SECRET_KEY          Same as --secret-key
  NOSTR_SECRET_KEY         Alias for TOON_SECRET_KEY (identity); TOON_SECRET_KEY wins
  TOON_RELAY_PORT          Same as --relay-port
  TOON_BLS_PORT            Same as --bls-port
  TOON_HOST                Same as --host
  TOON_WRITE_HOST          Same as --write-host
  TOON_DATA_DIR            Same as --data-dir
  TOON_DEV_MODE            Same as --dev-mode (set to "true")
  TOON_VERIFY_EPHEMERAL    Same as --verify-ephemeral (set to "true")
  TOON_VERIFY_WORKERS      Same as --verify-workers
  TOON_MAX_CONNECTIONS     Same as --max-connections
  TOON_EPHEMERAL_RATE_LIMIT       Same as --ephemeral-rate-limit
  TOON_EPHEMERAL_RATE_WINDOW_MS   Same as --ephemeral-rate-window-ms
  TOON_EPHEMERAL_MAX_BODY_BYTES   Same as --ephemeral-max-body-bytes
  TOON_LOG_WRITES          Same as --log-writes (set to "true")
  TOON_ENFORCE_EXPIRATION  Set to "false" for --no-enforce-expiration
  TOON_EXPIRATION_REAP_GRACE_SECONDS     Same as --expiration-reap-grace-seconds
  TOON_EXPIRATION_REAP_INTERVAL_SECONDS  Same as --expiration-reap-interval-seconds
  TOON_BLOCKED_EVENT_IDS   Same as --blocked-event-ids

Security:
  Prefer TOON_MNEMONIC / TOON_SECRET_KEY / NOSTR_SECRET_KEY environment
  variables over --mnemonic / --secret-key CLI flags. CLI arguments are visible
  to other users on the system via process listings (e.g. ps aux). See CWE-214.
`.trim()
  );
}

/**
 * Parse an optional numeric option that must be a positive integer.
 *
 * @param flag - The flag name, used verbatim in the error message.
 * @param raw - The flag value, or its env-var fallback, or undefined.
 * @returns The parsed value, or undefined when neither source is set (so the
 *   caller leaves the field off `RelayConfig` and `startRelay()` applies its
 *   own default). Exits the process on a non-positive/non-numeric value.
 */
function parsePositiveIntOption(
  flag: string,
  raw: string | undefined
): number | undefined {
  if (!raw) {
    return undefined;
  }
  const parsed = parseInt(raw, 10);
  if (Number.isNaN(parsed) || parsed <= 0) {
    console.error(`Error: --${flag} must be a positive integer`);
    process.exit(1);
  }
  return parsed;
}

/**
 * Parse an optional numeric option that must be a non-negative integer.
 *
 * Separate from `parsePositiveIntOption` because 0 is MEANINGFUL for the
 * retention knobs: a 0 reap grace means "delete the moment it expires" and a
 * 0 reap interval means "never sweep", both deliberate operator choices.
 *
 * @param flag - The flag name, used verbatim in the error message.
 * @param raw - The flag value, or its env-var fallback, or undefined.
 * @returns The parsed value, or undefined when neither source is set.
 */
function parseNonNegativeIntOption(
  flag: string,
  raw: string | undefined
): number | undefined {
  if (raw === undefined || raw === '') {
    return undefined;
  }
  const parsed = parseInt(raw, 10);
  if (Number.isNaN(parsed) || parsed < 0) {
    console.error(`Error: --${flag} must be an integer >= 0`);
    process.exit(1);
  }
  return parsed;
}

function parseCli(): RelayConfig {
  const { values } = parseArgs({
    options: {
      mnemonic: { type: 'string' },
      'secret-key': { type: 'string' },
      'relay-port': { type: 'string' },
      'bls-port': { type: 'string' },
      host: { type: 'string' },
      'write-host': { type: 'string' },
      'data-dir': { type: 'string' },
      'dev-mode': { type: 'boolean' },
      'verify-ephemeral': { type: 'boolean' },
      'verify-workers': { type: 'string' },
      'max-connections': { type: 'string' },
      'ephemeral-rate-limit': { type: 'string' },
      'ephemeral-rate-window-ms': { type: 'string' },
      'ephemeral-max-body-bytes': { type: 'string' },
      'log-writes': { type: 'boolean' },
      'no-enforce-expiration': { type: 'boolean' },
      'expiration-reap-grace-seconds': { type: 'string' },
      'expiration-reap-interval-seconds': { type: 'string' },
      'blocked-event-ids': { type: 'string' },
      help: { type: 'boolean' },
    },
    strict: true,
    allowPositionals: false,
  });

  if (values.help) {
    printHelp();
    process.exit(0);
  }

  // Warn about process-listing exposure (CWE-214) when secrets are passed via CLI flags
  if (values.mnemonic) {
    console.warn(
      'Warning: --mnemonic is visible in process listings. ' +
        'Prefer TOON_MNEMONIC environment variable for production use.'
    );
  }
  if (values['secret-key']) {
    console.warn(
      'Warning: --secret-key is visible in process listings. ' +
        'Prefer TOON_SECRET_KEY environment variable for production use.'
    );
  }

  const mnemonic = values.mnemonic ?? process.env['TOON_MNEMONIC'] ?? undefined;

  // Identity secret key. NOSTR_SECRET_KEY is accepted as an alias for
  // TOON_SECRET_KEY so the container honors the same identity env the connector
  // compose uses. TOON_SECRET_KEY wins when both are set.
  const secretKeyHex =
    values['secret-key'] ??
    process.env['TOON_SECRET_KEY'] ??
    process.env['NOSTR_SECRET_KEY'] ??
    undefined;

  let secretKey: Uint8Array | undefined;
  if (secretKeyHex) {
    if (secretKeyHex.length !== 64 || !/^[0-9a-fA-F]{64}$/.test(secretKeyHex)) {
      console.error('Error: --secret-key must be a 64-character hex string');
      process.exit(1);
    }
    secretKey = Uint8Array.from(Buffer.from(secretKeyHex, 'hex'));
  }

  if (!mnemonic && !secretKey) {
    console.error(
      'Error: one of --mnemonic (or TOON_MNEMONIC) or --secret-key ' +
        '(or TOON_SECRET_KEY / NOSTR_SECRET_KEY) is required'
    );
    process.exit(1);
  }
  if (mnemonic && secretKey) {
    console.error('Error: provide either a mnemonic or a secret key, not both');
    process.exit(1);
  }

  const relayPortStr =
    values['relay-port'] ?? process.env['TOON_RELAY_PORT'] ?? undefined;
  const relayPort = relayPortStr ? parseInt(relayPortStr, 10) : undefined;
  if (
    relayPort !== undefined &&
    (Number.isNaN(relayPort) || relayPort <= 0 || relayPort > 65535)
  ) {
    console.error('Error: --relay-port must be an integer between 1 and 65535');
    process.exit(1);
  }

  const blsPortStr =
    values['bls-port'] ?? process.env['TOON_BLS_PORT'] ?? undefined;
  const blsPort = blsPortStr ? parseInt(blsPortStr, 10) : undefined;
  if (
    blsPort !== undefined &&
    (Number.isNaN(blsPort) || blsPort <= 0 || blsPort > 65535)
  ) {
    console.error('Error: --bls-port must be an integer between 1 and 65535');
    process.exit(1);
  }

  const host = values.host ?? process.env['TOON_HOST'] ?? undefined;

  const writeHost =
    values['write-host'] ?? process.env['TOON_WRITE_HOST'] ?? undefined;

  const dataDir =
    values['data-dir'] ?? process.env['TOON_DATA_DIR'] ?? undefined;

  const devMode =
    values['dev-mode'] ??
    (process.env['TOON_DEV_MODE'] === 'true' ? true : undefined);

  // Paid-ephemeral verify skip (relay#85): full verification on ephemeral
  // kinds is opt-in (the skip is the default; see RelayConfig.verifyEphemeral
  // for the payment-gated invariant that makes that safe).
  const verifyEphemeral =
    values['verify-ephemeral'] ??
    (process.env['TOON_VERIFY_EPHEMERAL'] === 'true' ? true : undefined);

  // Verify-pool size (relay#85). 0 is a meaningful value (inline path), so
  // only an absent/blank setting falls through to the default.
  const verifyWorkersStr =
    values['verify-workers'] ?? process.env['TOON_VERIFY_WORKERS'] ?? undefined;
  const verifyWorkers = verifyWorkersStr
    ? parseInt(verifyWorkersStr, 10)
    : undefined;
  if (
    verifyWorkers !== undefined &&
    (Number.isNaN(verifyWorkers) || verifyWorkers < 0 || verifyWorkers > 256)
  ) {
    console.error('Error: --verify-workers must be an integer >= 0');
    process.exit(1);
  }

  // WS connection cap (relay#90). Each connection costs one fd; the server
  // logs an advisory warning at startup if the cap exceeds the soft limit.
  const maxConnectionsStr =
    values['max-connections'] ??
    process.env['TOON_MAX_CONNECTIONS'] ??
    undefined;
  const maxConnections = maxConnectionsStr
    ? parseInt(maxConnectionsStr, 10)
    : undefined;
  if (
    maxConnections !== undefined &&
    (Number.isNaN(maxConnections) || maxConnections <= 0)
  ) {
    console.error('Error: --max-connections must be a positive integer');
    process.exit(1);
  }

  // Free ephemeral write lane bounds (relay#129) -- POST /write-ephemeral has
  // no payment gate, so these ARE its admission control. All three share the
  // same shape: unset falls through to startRelay()'s own default, anything
  // that is not a positive integer is a startup error.
  const ephemeralRateLimitMax = parsePositiveIntOption(
    'ephemeral-rate-limit',
    values['ephemeral-rate-limit'] ?? process.env['TOON_EPHEMERAL_RATE_LIMIT']
  );
  const ephemeralRateWindowMs = parsePositiveIntOption(
    'ephemeral-rate-window-ms',
    values['ephemeral-rate-window-ms'] ??
      process.env['TOON_EPHEMERAL_RATE_WINDOW_MS']
  );
  const ephemeralMaxBodyBytes = parsePositiveIntOption(
    'ephemeral-max-body-bytes',
    values['ephemeral-max-body-bytes'] ??
      process.env['TOON_EPHEMERAL_MAX_BODY_BYTES']
  );

  // maxRequests/windowMs travel together as one RelayConfig field -- only
  // build it once at least one half was actually provided, so an unset pair
  // still falls through to startRelay()'s own default.
  const ephemeralRateLimit =
    ephemeralRateLimitMax !== undefined || ephemeralRateWindowMs !== undefined
      ? {
          maxRequests:
            ephemeralRateLimitMax ?? DEFAULT_EPHEMERAL_RATE_LIMIT.maxRequests,
          windowMs:
            ephemeralRateWindowMs ?? DEFAULT_EPHEMERAL_RATE_LIMIT.windowMs,
        }
      : undefined;

  const logWrites =
    values['log-writes'] ??
    (process.env['TOON_LOG_WRITES'] === 'true' ? true : undefined);

  // --- Retention (relay#137) ---
  // Enforcement is ON by default, so the only thing worth expressing here is
  // turning it OFF. `TOON_ENFORCE_EXPIRATION` is compared against the exact
  // string 'false' so a typo ('False', 'no', '0') fails SAFE -- towards
  // enforcing -- rather than silently reopening the hole this closed.
  const enforceExpiration =
    values['no-enforce-expiration'] === true ||
    process.env['TOON_ENFORCE_EXPIRATION'] === 'false'
      ? false
      : undefined;

  const expirationReapGraceSeconds = parseNonNegativeIntOption(
    'expiration-reap-grace-seconds',
    values['expiration-reap-grace-seconds'] ??
      process.env['TOON_EXPIRATION_REAP_GRACE_SECONDS']
  );
  const expirationReapIntervalSeconds = parseNonNegativeIntOption(
    'expiration-reap-interval-seconds',
    values['expiration-reap-interval-seconds'] ??
      process.env['TOON_EXPIRATION_REAP_INTERVAL_SECONDS']
  );

  // Operator blocklist. A malformed id is a hard startup error, not a warning:
  // its failure mode is an operator who believes an event is blocked while the
  // relay keeps serving it.
  const blocklist = parseBlockedEventIds(
    values['blocked-event-ids'] ?? process.env['TOON_BLOCKED_EVENT_IDS']
  );
  if (blocklist.invalid.length > 0) {
    console.error(
      'Error: --blocked-event-ids entries must be 64-character hex event ids; ' +
        `rejected: ${blocklist.invalid.join(', ')}`
    );
    process.exit(1);
  }
  const blockedEventIds = blocklist.ids.length > 0 ? blocklist.ids : undefined;

  const config: RelayConfig = {
    ...(mnemonic && { mnemonic }),
    ...(secretKey && { secretKey }),
    ...(relayPort !== undefined && { relayPort }),
    ...(blsPort !== undefined && { blsPort }),
    ...(host && { host }),
    ...(writeHost && { writeHost }),
    ...(dataDir && { dataDir }),
    ...(devMode !== undefined && { devMode }),
    ...(verifyEphemeral !== undefined && { verifyEphemeral }),
    ...(verifyWorkers !== undefined && { verifyWorkers }),
    ...(maxConnections !== undefined && { maxConnections }),
    ...(ephemeralRateLimit !== undefined && { ephemeralRateLimit }),
    ...(ephemeralMaxBodyBytes !== undefined && { ephemeralMaxBodyBytes }),
    ...(logWrites !== undefined && { logWrites }),
    ...(enforceExpiration !== undefined && { enforceExpiration }),
    ...(expirationReapGraceSeconds !== undefined && {
      expirationReapGraceSeconds,
    }),
    ...(expirationReapIntervalSeconds !== undefined && {
      expirationReapIntervalSeconds,
    }),
    ...(blockedEventIds !== undefined && { blockedEventIds }),
  };

  return config;
}

// ---------- Main ----------

async function main(): Promise<void> {
  const config = parseCli();

  console.log('\n' + '='.repeat(50));
  console.log('TOON Relay Starting');
  console.log('='.repeat(50) + '\n');

  const instance: RelayInstance = await startRelay(config);

  console.log('\n' + '='.repeat(50));
  console.log('TOON Relay Ready');
  console.log('='.repeat(50));
  console.log(`  Pubkey:  ${instance.pubkey}`);
  console.log(`  Reads:      ws://localhost:${instance.config.relayPort}`);
  console.log(
    `  Writes:     http://localhost:${instance.config.blsPort}/write`
  );
  console.log(
    `  Ephemeral:  http://localhost:${instance.config.blsPort}/write-ephemeral (free, relay#129)`
  );
  console.log(
    `  Health:     http://localhost:${instance.config.blsPort}/health`
  );
  console.log('='.repeat(50) + '\n');

  // Wire graceful shutdown
  const shutdown = async (signal: string): Promise<void> => {
    console.log(`\n[Shutdown] Received ${signal}`);
    await instance.stop();
    console.log('[Shutdown] Complete');
    process.exit(0);
  };

  process.on('SIGINT', () => {
    shutdown('SIGINT').catch(console.error);
  });
  process.on('SIGTERM', () => {
    shutdown('SIGTERM').catch(console.error);
  });
}

main().catch((error: unknown) => {
  console.error('[Fatal] Startup error:', error);
  process.exit(1);
});
