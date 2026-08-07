/**
 * Guards the deploy bundle (../../../../deploy — this repo's published
 * relay-connector artifacts) against the exact defects fixed in relay#113 and
 * relay#114: a settlement registry, a route price, or a CONNECTOR_TAG copy
 * silently drifting from what the live TOON devnet fleet actually runs, and a
 * port published on every interface instead of behind the deployment
 * environment's TLS terminator.
 *
 * Reads the REAL files under `deploy/`, not fixtures — a fixture would keep
 * passing while the shipped artifact regressed. Expected values are literals
 * declared here, never read back out of the file under test, so a reverted
 * fix fails this suite instead of quietly agreeing with itself.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parse } from 'smol-toml';
import { parse as parseYaml } from 'yaml';

const REPO_ROOT = resolve(import.meta.dirname, '../../../..');
const CONNECTOR_TOML_PATH = resolve(REPO_ROOT, 'deploy/connector.toml');
const DOCKER_COMPOSE_PATH = resolve(REPO_ROOT, 'deploy/docker-compose.yml');

// The relay's own paid-write store port (POST /write). It must never be
// host-published — only the connector may reach it, over the compose network
// (relay#114).
const PAID_WRITE_PORT = '3100';

interface DockerCompose {
  services: Record<string, { ports?: string[]; expose?: (string | number)[] }>;
}

function readDockerCompose(): DockerCompose {
  return parseYaml(readFileSync(DOCKER_COMPOSE_PATH, 'utf8')) as DockerCompose;
}

// Host IPs that mean "every interface", including the empty string a mapping
// with no IP field at all leaves in that slot.
const WILDCARD_HOST_IPS = new Set(['', '0.0.0.0', '::']);

// Compose substitutes `${VAR:-default}` from the operator's .env; with no .env
// — the shape this repo ships — every substitution becomes its default, and a
// substitution with no default becomes empty. Resolving them yields the
// mapping compose actually publishes out of the box, which is the thing under
// test. It also keeps the ':' of ':-' out of the field split below.
function resolveComposeDefaults(portEntry: string): string {
  return portEntry.replace(
    /\$\{([^}]+)\}/g,
    (_match, reference: string) => reference.split(':-')[1] ?? ''
  );
}

// A docker short-form port mapping is either `container` / `host:container`
// (both bind ALL interfaces) or `ip:host:container` (binds only that
// interface). An IPv6 host IP is bracketed in this syntax, so peel it off
// before splitting the rest on ':'.
function hostIpOf(portEntry: string): string {
  const entry = resolveComposeDefaults(portEntry);
  const bracketedIpv6 = entry.match(/^\[([^\]]*)\]:/);
  if (bracketedIpv6) return bracketedIpv6[1] ?? '';

  const fields = entry.split(':');
  if (fields.length < 3) return '';
  return fields[0] ?? '';
}

// The shared TOON devnet's TokenNetworkRegistry — the same registry, token
// and decimals both live boxes settle through (relay#113). A node pointed at
// a different registry cannot resolve the channels buyers actually opened
// against this one.
const EXPECTED_CONTRACT_ADDRESS = '0x8263BdD4eB4862395Cb4ef5dA5d637F4b047Eea1';
// 6-decimal devnet USDC, the fleet-wide settlement asset (ADR 0010).
const EXPECTED_TOKEN_ADDRESS = '0x49beE1Bca5d15Fb0963117923403F9498119a9Ce';
const EXPECTED_DECIMALS = 6;

// The price the live TOON devnet apex charges per prefix (owner decision,
// 2026-08-04 for g.toon.relay — see connector.toml's own route comment).
const EXPECTED_ROUTE_PRICES: Record<string, number> = {
  'g.toon.relay': 1,
};

// The fleet's pin of record, chosen in toon-protocol/connector#848 (merged
// as connector PR #859): rust-sha-440eab7 is the earliest Rust connector
// build at or after the announce-identity fixes (connector#833/#839), and the
// first pin this bundle has shipped that understands `[announce]`
// (connector#784).
const EXPECTED_CONNECTOR_TAG = 'rust-sha-440eab7';

// Every site that names the pin, and how to pull the value back out of each
// one's own syntax. A textual scan rather than a parser because no two of
// these are the same language, and none of them is TOML. The publish workflow
// is deliberately absent: it passes no CONNECTOR_TAG build-arg, so the
// Dockerfile ARG default below is what it ships (relay#113).
const CONNECTOR_TAG_SITES: { file: string; pattern: RegExp }[] = [
  { file: 'deploy/Dockerfile', pattern: /ARG CONNECTOR_TAG=(\S+)/ },
  { file: 'deploy/.env.example', pattern: /^CONNECTOR_TAG=(\S+)/m },
  {
    file: 'deploy/docker-compose.yml',
    pattern: /CONNECTOR_TAG:\s*\$\{CONNECTOR_TAG:-([^}\s]+)\}/,
  },
  // Prose, but it quotes the literal, so it drifts like any other copy — and a
  // README naming a pin the image does not carry is how an operator ends up
  // deploying one connector while reading about another.
  {
    file: 'deploy/README.md',
    pattern: /`CONNECTOR_TAG` ARG, currently `([^`]+)`/,
  },
];

const PUBLISH_WORKFLOW_PATH =
  '.github/workflows/publish-relay-connector-image.yml';

interface ConnectorToml {
  routes: { prefix: string; price: number }[];
  settlement: {
    evm: {
      contract_address: string;
      token_address: string;
      decimals: number;
    };
  };
}

function readConnectorToml(): ConnectorToml {
  return parse(
    readFileSync(CONNECTOR_TOML_PATH, 'utf8')
  ) as unknown as ConnectorToml;
}

describe('deploy bundle', () => {
  it('settles against the live fleet registry, token, and decimals', () => {
    const { evm } = readConnectorToml().settlement;

    expect(
      evm.contract_address,
      `settlement.evm.contract_address: expected ${EXPECTED_CONTRACT_ADDRESS}, found ${evm.contract_address}`
    ).toBe(EXPECTED_CONTRACT_ADDRESS);
    expect(
      evm.token_address,
      `settlement.evm.token_address: expected ${EXPECTED_TOKEN_ADDRESS}, found ${evm.token_address}`
    ).toBe(EXPECTED_TOKEN_ADDRESS);
    expect(
      evm.decimals,
      `settlement.evm.decimals: expected ${EXPECTED_DECIMALS}, found ${evm.decimals}`
    ).toBe(EXPECTED_DECIMALS);
  });

  it('charges the documented price on every route', () => {
    const { routes } = readConnectorToml();
    const seenPrefixes = routes.map((route) => route.prefix);

    expect(
      seenPrefixes.sort(),
      `unexpected set of route prefixes: found ${JSON.stringify(seenPrefixes)}, expected ${JSON.stringify(Object.keys(EXPECTED_ROUTE_PRICES).sort())}`
    ).toEqual(Object.keys(EXPECTED_ROUTE_PRICES).sort());

    for (const route of routes) {
      const expectedPrice = EXPECTED_ROUTE_PRICES[route.prefix];
      expect(
        route.price,
        `route ${route.prefix}: expected price ${expectedPrice}, found ${route.price}`
      ).toBe(expectedPrice);
    }
  });

  it('pins the same CONNECTOR_TAG everywhere it appears', () => {
    for (const site of CONNECTOR_TAG_SITES) {
      const content = readFileSync(resolve(REPO_ROOT, site.file), 'utf8');
      const match = content.match(site.pattern);

      expect(
        match,
        `${site.file}: CONNECTOR_TAG not found matching ${site.pattern}`
      ).not.toBeNull();
      expect(
        match?.[1],
        `${site.file}: expected CONNECTOR_TAG=${EXPECTED_CONNECTOR_TAG}, found CONNECTOR_TAG=${match?.[1]}`
      ).toBe(EXPECTED_CONNECTOR_TAG);
    }
  });

  it('leaves the publish workflow with no CONNECTOR_TAG of its own', () => {
    // The other half of "the ARG default is the pin of record": a build-arg
    // reintroduced here would silently outrank deploy/Dockerfile and decide
    // what actually ships to GHCR, and it is the one copy the sweep above
    // cannot compare because there is nothing to compare it against.
    const content = readFileSync(
      resolve(REPO_ROOT, PUBLISH_WORKFLOW_PATH),
      'utf8'
    );
    const assignment = content.match(/^[^#\n]*CONNECTOR_TAG\s*[=:].*$/m);

    expect(
      assignment,
      `${PUBLISH_WORKFLOW_PATH}: expected no CONNECTOR_TAG build-arg, found "${assignment?.[0].trim()}" — deploy/Dockerfile's ARG default is the pin of record`
    ).toBeNull();
  });

  it('never publishes a port on all interfaces', () => {
    const { services } = readDockerCompose();

    for (const [serviceName, service] of Object.entries(services)) {
      for (const portEntry of service.ports ?? []) {
        const hostIp = hostIpOf(portEntry);
        expect(
          WILDCARD_HOST_IPS.has(hostIp),
          `docker-compose.yml service "${serviceName}": port "${portEntry}" binds every interface (host IP "${hostIp}") — prefix it with a host IP such as 127.0.0.1 (relay#114)`
        ).toBe(false);
      }
    }
  });

  it('never publishes the paid-write store port; it stays on expose', () => {
    const { services } = readDockerCompose();

    for (const [serviceName, service] of Object.entries(services)) {
      const leakingEntry = (service.ports ?? []).find((portEntry) =>
        resolveComposeDefaults(portEntry).split(':').includes(PAID_WRITE_PORT)
      );
      expect(
        leakingEntry,
        `docker-compose.yml service "${serviceName}": paid-write port :${PAID_WRITE_PORT} is under \`ports:\` ("${leakingEntry}") — it must stay \`expose:\`-only`
      ).toBeUndefined();
    }

    const exposedByRelay = (services['relay']?.expose ?? []).map(String);
    expect(
      exposedByRelay,
      `docker-compose.yml relay service: expected :${PAID_WRITE_PORT} under \`expose:\`, found ${JSON.stringify(exposedByRelay)}`
    ).toContain(PAID_WRITE_PORT);
  });
});
