/**
 * Guards the deploy bundle — the files this repo hands an operator to run a
 * node, and the one image it publishes to carry the app.
 *
 * It reads the REAL files, not fixtures: a fixture would keep passing while
 * the shipped artifact regressed. Expected values are literals declared here
 * and never read back out of the file under test, so a reverted fix fails
 * this suite instead of quietly agreeing with itself.
 *
 * What it holds still, and why each one is worth a test:
 *
 * - the settlement deployment, the route prices and the handler each route
 *   terminates at — the facts a buyer's channel resolves against, and the
 *   difference between a free lane and a free ride;
 * - `[node]`, because a node that cannot say where it is cannot be paid;
 * - the connector pin, in exactly one place, because two copies drift;
 * - the privacy invariant: nothing is reachable from the internet except the
 *   TLS front, and the relay's write port is not published at all;
 * - healthchecks dialling 127.0.0.1, because "localhost" in a container can
 *   resolve to ::1 where an IPv4-bound listener never answers.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parse } from 'smol-toml';
import { parse as parseYaml } from 'yaml';

const REPO_ROOT = resolve(import.meta.dirname, '..');
const CONNECTOR_TOML_PATH = resolve(REPO_ROOT, 'deploy/connector.toml');
const DOCKER_COMPOSE_PATH = resolve(REPO_ROOT, 'deploy/docker-compose.yml');
const CADDYFILE_PATH = resolve(REPO_ROOT, 'deploy/Caddyfile');

// The relay's own write ports. Neither may ever be host-published in any
// form: they are the payment-oblivious surface, and the relay skips schnorr
// verification for paid ephemeral kinds precisely because the only route to
// them is through the payment-gating connector.
const PRIVATE_RELAY_PORTS = ['3100'];
// The connector's client edge. Public traffic reaches it through Caddy, which
// can see, log and terminate TLS for it. It is published to LOOPBACK for
// on-box operator calls, which is what the store and gas-station bundles do
// too — and a loopback bind is not reachable from the internet.
const CONNECTOR_EDGE_PORT = '3000';

// The one service allowed an UNQUALIFIED publish, because it is the TLS front
// and being reachable is its whole job.
const PUBLISHING_SERVICE = 'caddy';
const EXPECTED_PUBLISHED_PORTS = ['80:80', '443:443'];

// The prefix that makes a publish local-only. A `ports:` entry without a host
// IP — or with 0.0.0.0 — binds every interface, and Docker's iptables chain
// runs ahead of ufw, so such a publish is internet-reachable even with ufw
// locked to 22/80/443. That is the thing the invariant forbids.
const LOOPBACK_PUBLISH_PREFIX = '127.0.0.1:';

interface DockerCompose {
  services: Record<
    string,
    {
      image?: string;
      build?: unknown;
      ports?: string[];
      expose?: (string | number)[];
      labels?: Record<string, string>;
      volumes?: string[];
    }
  >;
}

function readDockerCompose(): DockerCompose {
  return parseYaml(readFileSync(DOCKER_COMPOSE_PATH, 'utf8')) as DockerCompose;
}

// Compose substitutes `${VAR:-default}` from the operator's .env; with no
// .env — the shape this repo ships — every substitution becomes its default.
function resolveComposeDefaults(portEntry: string): string {
  return portEntry.replace(
    /\$\{([^}]+)\}/g,
    (_match, reference: string) => reference.split(':-')[1] ?? ''
  );
}

// The shared TOON devnet's TokenNetworkRegistry (the 2026-08-28 ADR 0059 cutover's;
// connector docs/evm-deployment.md) — the same registry, token
// and decimals the fleet settles through. A node pointed at a different
// registry cannot resolve the channels buyers actually opened against it.
const EXPECTED_CONTRACT_ADDRESS = '0x0c41D9D424d6B075A3cEa1068a694f7847a8CCa5';
// 6-decimal devnet USDC, the fleet-wide settlement asset.
const EXPECTED_TOKEN_ADDRESS = '0x49beE1Bca5d15Fb0963117923403F9498119a9Ce';
const EXPECTED_DECIMALS = 6;

// The Solana half of the same statement, and pinned for the same reason: a
// claim resolves against ONE deployment, so a node naming a different program
// or mint cannot settle the channels buyers opened against the fleet.
//
// This pin is late. The EVM leg above has been asserted since this file was
// written; the Solana leg was not, and it drifted to a mint that had become
// unusable -- the mock USDC deployed 2026-07-18 whose MINT AUTHORITY was a key
// held outside any repository and since lost. Nobody could issue that token or
// refill a treasury holding it, so the devnet faucet's Solana leg served 503s
// for weeks while this bundle went on claiming to settle in it. The
// replacement's authority is the faucet box's own treasury, so the faucet
// mints per drip and there is no irreplaceable key left in the arrangement.
// See connector's packages/solana-program/deployments/devnet-public.md.
const EXPECTED_SOLANA_PROGRAM_ID = '2aEVJ8koKD8LTZrLRSGtAtU7LBt4e7QjjCgf1kzQ7Rip';
const EXPECTED_SOLANA_TOKEN_ADDRESS =
  '34eSxY7qxQ4GzyhDJ8GpUcTz1WWzruGbJbR8q6TtxfQU';

// `g.toon.relay` is 1 micro-USDC per write (owner decision, 2026-08-04).
// `g.toon.relay.ephemeral` is deliberately, explicitly 0 — the free lane.
const EXPECTED_ROUTE_PRICES: Record<string, number> = {
  'g.toon.relay': 1,
  'g.toon.relay.ephemeral': 0,
};

// Each route must terminate at the relay endpoint that enforces that route's
// semantics: a route pointed at the wrong handler carries the wrong price's
// guarantees, and a free route pointed at `/write` is a free ride around
// pay-to-write.
const EXPECTED_ROUTE_HANDLER_URLS: Record<string, string> = {
  'g.toon.relay': 'http://relay:3100/write',
  'g.toon.relay.ephemeral': 'http://relay:3100/write-ephemeral',
};

// The pin of record. `rust-sha-c714551` is the first build this bundle has
// shipped on which a peering established over the operator surface can pay
// for what it forwards (connector#1230); its predecessor `rust-sha-6ea6009`
// was the first to speak `[node]` (ADR 0050) and state a verified payment to
// the app on delivery (ADR 0040).
const EXPECTED_CONNECTOR_TAG = 'rust-2026.08.28.1';

// The one file that may name a connector build. It used to be
// deploy/Dockerfile's `ARG CONNECTOR_TAG`, back when this bundle published a
// derived `relay-connector` image with connector.toml baked in; the bundle now
// runs the STOCK connector image with connector.toml mounted, exactly as the
// store and gas-station bundles do, so the pin of record is the compose file's
// `image:`.
const PIN_OF_RECORD_PATH = 'deploy/docker-compose.yml';

// Every file that could name a connector build. Exactly one of them may.
const FILES_THAT_COULD_NAME_A_CONNECTOR_BUILD = [
  PIN_OF_RECORD_PATH,
  'deploy/.env.example',
  'deploy/README.md',
  'README.md',
];

// Every wget-based healthcheck this repo ships, and how to pull the target
// host out of each site's own syntax.
const HEALTHCHECK_WGET_SITES: { file: string; pattern: RegExp }[] = [
  {
    file: 'deploy/docker-compose.yml',
    pattern: /wget -q --spider http:\/\/([^:/]+):3100\/health/,
  },
  {
    file: 'deploy/docker-compose.yml',
    pattern: /wget -q --spider http:\/\/([^:/]+):3000\/ilp\/identity/,
  },
  {
    file: 'packages/relay/Dockerfile',
    pattern:
      /wget -q --spider "http:\/\/([^:/]+):\$\{TOON_BLS_PORT:-3100\}\/health"/,
  },
];

interface ConnectorToml {
  operator: { bearer_token_file: string; write_keys_file: string };
  client_edge_addr: string;
  state_dir: string;
  signer: { key_file: string };
  node: {
    addresses: string[];
    http_endpoint: string;
    btp_endpoint: string;
  };
  routes: { prefix: string; price: number; handler_url: string }[];
  settlement: {
    evm: {
      contract_address: string;
      token_address: string;
      decimals: number;
    };
    solana: {
      program_id: string;
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

function readFile(relativePath: string): string {
  return readFileSync(resolve(REPO_ROOT, relativePath), 'utf8');
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

  it('settles against the live fleet Solana program, mint, and decimals', () => {
    const { solana } = readConnectorToml().settlement;

    expect(
      solana.program_id,
      `settlement.solana.program_id: expected ${EXPECTED_SOLANA_PROGRAM_ID}, found ${solana.program_id}`
    ).toBe(EXPECTED_SOLANA_PROGRAM_ID);
    expect(
      solana.token_address,
      `settlement.solana.token_address: expected ${EXPECTED_SOLANA_TOKEN_ADDRESS}, found ${solana.token_address}`
    ).toBe(EXPECTED_SOLANA_TOKEN_ADDRESS);
    expect(
      solana.decimals,
      `settlement.solana.decimals: expected ${EXPECTED_DECIMALS}, found ${solana.decimals}`
    ).toBe(EXPECTED_DECIMALS);
  });

  it('charges the documented price on every route', () => {
    const { routes } = readConnectorToml();
    const seenPrefixes = routes.map((route) => route.prefix);

    expect(
      seenPrefixes.sort(),
      `unexpected set of route prefixes: found ${JSON.stringify(seenPrefixes)}`
    ).toEqual(Object.keys(EXPECTED_ROUTE_PRICES).sort());

    for (const route of routes) {
      const expectedPrice = EXPECTED_ROUTE_PRICES[route.prefix];
      expect(
        route.price,
        `route ${route.prefix}: expected price ${expectedPrice}, found ${route.price}`
      ).toBe(expectedPrice);
    }
  });

  it('terminates each route at the relay endpoint that enforces its price', () => {
    const { routes } = readConnectorToml();

    for (const route of routes) {
      const expectedHandlerUrl = EXPECTED_ROUTE_HANDLER_URLS[route.prefix];
      expect(
        route.handler_url,
        `route ${route.prefix}: expected handler_url ${expectedHandlerUrl}, found ${route.handler_url}`
      ).toBe(expectedHandlerUrl);
    }

    // The free lane and the paid lane must never collapse onto one
    // handler_url: the connector refuses a handler reachable at two prices,
    // because the cheaper door would take every packet.
    const handlerUrls = routes.map((route) => route.handler_url);
    expect(new Set(handlerUrls).size).toBe(handlerUrls.length);
  });

  it('publishes every route it terminates in its node self-description', () => {
    const { node, routes } = readConnectorToml();

    // A prefix this node terminates but never advertises is a route no
    // client can discover — which is how a free lane ships invisible.
    expect(
      node.addresses.slice().sort(),
      `[node].addresses must list every terminated prefix; found ${JSON.stringify(node.addresses)}`
    ).toEqual(routes.map((route) => route.prefix).sort());

    // These are the facts a node cannot introspect, so they are worth
    // asserting the SHAPE of: a container-internal address here would
    // advertise an unreachable node to the whole network.
    expect(node.http_endpoint).toMatch(/^https:\/\/[^/]+\/ilp$/);
    expect(node.btp_endpoint).toMatch(/^wss:\/\/[^/]+\/ilp\/btp$/);
  });

  it('keeps its durable claim state and both identities on mounted paths', () => {
    const config = readConnectorToml();

    // A state_dir inside the container's writable layer loses every replay
    // watermark on restart, and a channel with no watermark accepts a claim
    // its payer already spent.
    expect(config.state_dir).toBe('/app/state');
    const stateMount = readDockerCompose().services['connector']?.volumes?.find(
      (volume) => volume.endsWith(':/app/state')
    );
    expect(
      stateMount,
      'docker-compose.yml connector: /app/state must be a named volume'
    ).toBe('connector_state:/app/state');

    // A key is a LOCATION here, never a value — nothing secret is ever
    // committed or baked into an image.
    expect(config.signer.key_file).toMatch(/^\/app\/data\/.+\.key$/);
  });

  it('enables the operator surface by file, and mounts both files', () => {
    // Every write on this surface -- establishing a peering above all -- is
    // RFC 9421-signed against this allowlist, and every read carries the
    // bearer token. This config is committed to a public repository, so the
    // two values may only ever be named by PATH here; the files themselves
    // are mounted beside the keys and gitignored.
    const { operator } = readConnectorToml();
    expect(operator.bearer_token_file).toBe('/app/data/operator-bearer.token');
    expect(operator.write_keys_file).toBe('/app/data/operator-write.keys');
    expect(readFile('deploy/connector.toml')).not.toMatch(/^\s*bearer_token\s*=/m);
    expect(readFile('deploy/connector.toml')).not.toMatch(/^\s*write_keys\s*=/m);

    const volumes = readDockerCompose().services['connector']?.volumes ?? [];
    for (const file of ['operator-bearer.token', 'operator-write.keys']) {
      expect(
        volumes,
        `docker-compose.yml connector: ${file} must be mounted read-only at /app/data`
      ).toContain(`./${file}:/app/data/${file}:ro`);
      expect(readFile('deploy/.gitignore')).toContain(file);
    }
  });

  it('names the connector build in exactly one place', () => {
    const pinned = readDockerCompose().services['connector']?.image;

    expect(
      pinned,
      `${PIN_OF_RECORD_PATH}: the connector service has no \`image:\` — it is the pin of record`
    ).toBe(`ghcr.io/toon-protocol/connector:${EXPECTED_CONNECTOR_TAG}`);

    // A moving tag here would make the pin a pointer someone else controls,
    // which is what a `rust-sha-` pin exists to avoid.
    expect(
      pinned,
      `${PIN_OF_RECORD_PATH}: pin an immutable build — a rust-sha- build or a rust-<release handle> — never a moving tag`
    ).toMatch(/:(rust-sha-[0-9a-f]{7,40}|rust-\d{4}\.\d{2}\.\d{2}\.\d+)$/);

    // The image must be the STOCK connector — the same one the store and
    // gas-station bundles run. A derived image would put the config somewhere
    // this repo's tests cannot see.
    expect(pinned).toMatch(/^ghcr\.io\/toon-protocol\/connector:/);

    // Every other site is checked for a `rust-sha-`/`rust-main`/`rust-release`
    // literal, in prose or in config. A second copy is how an operator ends
    // up deploying one connector while reading about another.
    for (const file of FILES_THAT_COULD_NAME_A_CONNECTOR_BUILD) {
      if (file === PIN_OF_RECORD_PATH) continue;
      const content = readFile(file);
      const literal = content.match(/rust-(?:sha-[0-9a-f]{7}|main|release)/);
      expect(
        literal,
        `${file}: names connector build "${literal?.[0]}" — ${PIN_OF_RECORD_PATH}'s connector \`image:\` is the only place a build may be pinned`
      ).toBeNull();
    }
  });

  it('mounts connector.toml rather than baking it into a derived image', () => {
    const connector = readDockerCompose().services['connector'];

    // The bundle used to publish `relay-connector`: the stock connector with
    // this connector.toml COPYed in. The property that bought — a build can
    // never reach a box ahead of the config it needs — is now supplied by the
    // immutable pin itself, since the pin and the config are one commit here
    // and the box takes both with one `git pull`. What baking cost was an
    // extra image, an extra publish workflow, and a deploy model unlike the
    // other two node bundles.
    expect(
      connector?.volumes ?? [],
      'docker-compose.yml connector: connector.toml must be mounted read-only'
    ).toContain('./connector.toml:/app/config/connector.toml:ro');

    // A `build:` key would reintroduce a second, unreviewable source for what
    // this service runs.
    expect(
      connector?.build,
      'docker-compose.yml connector: must run the published image, never a local build'
    ).toBeUndefined();
  });

  it('exposes nothing to the internet but the TLS front', () => {
    const { services } = readDockerCompose();

    // The substance of this invariant is "nothing is reachable from the
    // internet except the TLS front" — so what it forbids is an UNQUALIFIED
    // publish, on any service, not a publish as such. A `ports:` entry with
    // no host IP (or 0.0.0.0) binds every interface, and Docker's iptables
    // chain runs ahead of ufw, so it is internet-reachable even with ufw
    // locked to 22/80/443. A `127.0.0.1:`-prefixed entry is not reachable
    // off-box at all, and is how an operator reaches the connector's operator
    // surface — the same shape the store and gas-station bundles ship. This
    // test used to forbid a connector publish outright; it now forbids the
    // thing that was actually dangerous about one.
    for (const [serviceName, service] of Object.entries(services)) {
      const published = (service.ports ?? []).map(resolveComposeDefaults);
      if (serviceName === PUBLISHING_SERVICE) {
        expect(
          published,
          `docker-compose.yml ${PUBLISHING_SERVICE}: expected ${JSON.stringify(EXPECTED_PUBLISHED_PORTS)}`
        ).toEqual(EXPECTED_PUBLISHED_PORTS);
        continue;
      }
      for (const entry of published) {
        expect(
          entry.startsWith(LOOPBACK_PUBLISH_PREFIX),
          `docker-compose.yml service "${serviceName}": publishes "${entry}" with no host IP — only ${PUBLISHING_SERVICE} may be reachable off-box, and a bare docker publish beats ufw. Prefix it "${LOOPBACK_PUBLISH_PREFIX}" or use \`expose:\`.`
        ).toBe(true);
      }
    }
  });

  it('never publishes the relay write port, and binds the connector edge to loopback', () => {
    const { services } = readDockerCompose();
    const everyPublishedField = Object.values(services)
      .flatMap((service) => service.ports ?? [])
      .map(resolveComposeDefaults);

    // The write port is different from the edge: it has no authentication of
    // its own, so not even a loopback publish is acceptable. It must stay
    // `expose:`-only, reachable from the connector and nothing else.
    for (const port of PRIVATE_RELAY_PORTS) {
      const leaking = everyPublishedField.find((entry) =>
        entry.split(':').includes(port)
      );
      expect(
        leaking,
        `docker-compose.yml: the relay's write port :${port} is published ("${leaking}") — it must stay \`expose:\`-only`
      ).toBeUndefined();
    }

    const edgePublishes = everyPublishedField.filter((entry) =>
      entry.split(':').includes(CONNECTOR_EDGE_PORT)
    );
    expect(
      edgePublishes,
      `docker-compose.yml: the connector edge must be published on loopback exactly once, found ${JSON.stringify(edgePublishes)}`
    ).toEqual([
      `${LOOPBACK_PUBLISH_PREFIX}${CONNECTOR_EDGE_PORT}:${CONNECTOR_EDGE_PORT}`,
    ]);

    const exposedByRelay = (services['relay']?.expose ?? []).map(String);
    expect(
      exposedByRelay,
      `docker-compose.yml relay: expected the write port under \`expose:\`, found ${JSON.stringify(exposedByRelay)}`
    ).toContain(PRIVATE_RELAY_PORTS[0]);
  });

  it('routes TLS to the two public surfaces and never to the write port', () => {
    const caddyfile = readFileSync(CADDYFILE_PATH, 'utf8');

    expect(caddyfile).toContain('reverse_proxy connector:3000');
    expect(caddyfile).toContain('reverse_proxy relay:7100');
    // The one line that must never appear in this file.
    for (const port of PRIVATE_RELAY_PORTS) {
      expect(
        caddyfile.includes(`relay:${port}`),
        `deploy/Caddyfile: routes to the relay's write port :${port} — that is a public, unauthenticated write door`
      ).toBe(false);
    }
  });

  it('lets Watchtower recreate the app containers but never the TLS front', () => {
    const { services } = readDockerCompose();
    const WATCHTOWER_LABEL = 'com.centurylinklabs.watchtower.enable';

    for (const service of ['connector', 'relay']) {
      expect(
        services[service]?.labels?.[WATCHTOWER_LABEL],
        `docker-compose.yml ${service}: expected the Watchtower enable label`
      ).toBe('true');
    }

    // Caddy holds the certificates and the ACME account, and its job is
    // surviving the others being replaced. It must never opt in.
    expect(
      services[PUBLISHING_SERVICE]?.labels?.[WATCHTOWER_LABEL],
      `docker-compose.yml ${PUBLISHING_SERVICE}: must NOT carry the Watchtower enable label`
    ).toBeUndefined();
  });

  it('healthchecks dial 127.0.0.1, never localhost', () => {
    for (const site of HEALTHCHECK_WGET_SITES) {
      const content = readFile(site.file);
      const match = content.match(site.pattern);

      expect(
        match,
        `${site.file}: healthcheck target not found matching ${site.pattern}`
      ).not.toBeNull();
      expect(
        match?.[1],
        `${site.file}: healthcheck targets "${match?.[1]}" — inside a container "localhost" can resolve to ::1, which an IPv4-bound listener never answers on`
      ).toBe('127.0.0.1');
    }
  });
});
