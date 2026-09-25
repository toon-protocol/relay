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
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { parse } from 'smol-toml';
import { parse as parseYaml } from 'yaml';

const REPO_ROOT = resolve(import.meta.dirname, '..');
const CONNECTOR_TOML_PATH = resolve(REPO_ROOT, 'deploy/connector.toml');
const DOCKER_COMPOSE_PATH = resolve(REPO_ROOT, 'deploy/docker-compose.yml');
const CADDYFILE_PATH = resolve(REPO_ROOT, 'deploy/Caddyfile');
const SHARED_EDGE_OVERLAY_PATH = resolve(
  REPO_ROOT,
  'deploy/docker-compose.shared-edge.yml'
);

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
      environment?: Record<string, string>;
      profiles?: string[];
      mem_limit?: string;
      networks?: Record<string, { aliases?: string[] } | null>;
    }
  >;
  networks?: Record<string, { external?: boolean } | null>;
}

function readDockerCompose(): DockerCompose {
  return parseYaml(readFileSync(DOCKER_COMPOSE_PATH, 'utf8')) as DockerCompose;
}

function readSharedEdgeOverlay(): DockerCompose {
  return parseYaml(
    readFileSync(SHARED_EDGE_OVERLAY_PATH, 'utf8')
  ) as DockerCompose;
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

// The mint the leg above used to name, held here so the bundle can be checked
// for it by name. The pin above already fails a straight revert of
// connector.toml; this value exists because the retired mint is not merely an
// OLD address but a KNOWN-BAD one — its mint authority is lost, so a node that
// settles in it can never be funded — and a known-bad address is worth
// forbidding everywhere an operator could copy it from, not only in the one
// key the positive pin reads. The compose files, the Caddyfile, .env.example
// and the README are all things a second box gets set up from, and none of
// them is covered by a pin on `settlement.solana.token_address`.
const RETIRED_SOLANA_TOKEN_ADDRESS =
  'xyc5J8MgKFiEN13PnfftdXxUzYH34FEvw1LCrFwN7in';

// The bundle directory, scanned rather than listed: a file added here later is
// part of what an operator is handed, and enumerating the guard's inputs by
// hand is how the Solana leg went unasserted in the first place.
const BUNDLE_DIR = resolve(REPO_ROOT, 'deploy');

// This suite is the one bundle file that MUST name the retired mint — the
// constant above is how the check knows what to look for — so it is the one
// file the scan skips, the same exemption the connector-pin scan makes for the
// file that holds the pin.
const RETIRED_MINT_SCAN_EXEMPT = ['bundle.test.ts'];

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
const EXPECTED_CONNECTOR_TAG = 'rust-2026.09.11.1';

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
  routes: {
    prefix: string;
    price: number;
    handler_url: string;
    transport?: string;
  }[];
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

  it('names the retired Solana mint nowhere in the bundle', () => {
    // Asserting the live mint present and asserting the dead one absent are
    // not the same guard. The first covers exactly one key in one file; this
    // covers every file an operator reads, copies or runs — because the
    // failure being prevented is a box brought up on a settlement asset that
    // nobody, including its own operator, can ever obtain.
    const offenders = readdirSync(BUNDLE_DIR)
      .filter((entry) => !RETIRED_MINT_SCAN_EXEMPT.includes(entry))
      .filter((entry) => statSync(resolve(BUNDLE_DIR, entry)).isFile())
      .filter((entry) =>
        readFileSync(resolve(BUNDLE_DIR, entry), 'utf8').includes(
          RETIRED_SOLANA_TOKEN_ADDRESS
        )
      );

    expect(
      offenders,
      `deploy/${offenders.join(', deploy/')}: names the retired Solana mint ${RETIRED_SOLANA_TOKEN_ADDRESS}. Its mint authority is lost, so a node settling in it can never be funded — use ${EXPECTED_SOLANA_TOKEN_ADDRESS}`
    ).toEqual([]);
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

  it('points the relay at the very route that reaches its own POST /write', () => {
    // TOON_Network#121: the relay serves a NIP-11 document naming where a
    // write to it is paid for, and it READS that from the connector rather
    // than holding a copy. It is told exactly one thing — which prefix
    // arrives at its `/write` — because a self-description publishes route
    // prefixes and prices and never their handler_url (connector rule ND-08).
    //
    // That one pin is what this holds still. The failure it prevents is a
    // relay advertising an address that reaches somebody else's app: at
    // runtime the relay checks the address against its connector and refuses
    // an unknown one, but a prefix that exists and terminates ELSEWHERE would
    // pass that check and send every client's money down the wrong route.
    const { routes } = readConnectorToml();
    const relay = readDockerCompose().services['relay'];
    const environment = relay?.environment ?? {};

    const paidRoute = routes.find(
      (route) => route.handler_url === EXPECTED_ROUTE_HANDLER_URLS['g.toon.relay']
    );
    expect(
      paidRoute,
      `connector.toml: no route terminates at ${EXPECTED_ROUTE_HANDLER_URLS['g.toon.relay']}`
    ).toBeDefined();

    expect(
      environment['TOON_WRITE_ILP_ADDRESS'],
      `docker-compose.yml relay: TOON_WRITE_ILP_ADDRESS must be the prefix whose handler_url is ${EXPECTED_ROUTE_HANDLER_URLS['g.toon.relay']}`
    ).toBe(paidRoute?.prefix);

    // And the connector it asks is the one in this file, on the compose
    // network. A public URL here would make the relay's own advertisement
    // depend on DNS and TLS it does not need, and an address off this network
    // would be asking a different node entirely.
    expect(environment['TOON_CONNECTOR_URL']).toBe(
      'http://connector:3000/ilp'
    );

    // The carriage stopgap for TOON_Network#111 (see docker-compose.yml). It
    // is a value the relay states on its connector's behalf, so it is held
    // equal to what the connector actually pins; when #111 lands, the
    // connector publishes the pin itself, this env goes, and so does this
    // assertion.
    expect(
      resolveComposeDefaults(environment['TOON_WRITE_CARRIAGE'] ?? ''),
      `docker-compose.yml relay: TOON_WRITE_CARRIAGE must equal connector.toml's transport on ${paidRoute?.prefix}`
    ).toBe(paidRoute?.transport ?? '');
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

// ── The shared-edge overlay (toon-protocol/relay#166, infra ADR 0001) ───────
//
// The devnet is moving onto one host behind one shared Caddy edge. This
// node's own Caddy goes away — the edge terminates TLS for it instead — but
// everything else about the node (its connector, its keys, its hostnames)
// stays exactly as it is. The overlay is off by default (a bare `.env` with
// no `COMPOSE_FILE` line runs the bundle exactly as `docker-compose.yml`
// alone always has), so it can merge and ride along on the box's auto-apply
// timer while that box is still on its own Linode, well before the edge or
// the `edge-relay` network exist anywhere.
//
// Shared contract v2, point 1: one network PER NODE, not one flat network
// every node's containers share — on a flat network every node could reach
// every other node's connector operator surface and the gateway's handover
// port, where a per-node network limits that to the edge alone. This node's
// is `edge-relay`. The old flat `edge` name must not appear anywhere in this
// bundle any more (see the last test in this describe block).
//
// This suite reads the overlay file directly, the same way the tests above
// read `docker-compose.yml` directly: literals declared here, never read
// back out of the file under test.
describe('the shared-edge overlay (docker-compose.shared-edge.yml, toon-protocol/relay#166)', () => {
  // The external, per-node network the host-level edge (infra#24) creates
  // and this node's overlay alone joins. This repo does not own it — it only
  // joins it.
  const EDGE_NETWORK = 'edge-relay';
  // infra#24's contract: which alias serves which of this node's hostnames.
  const EDGE_ALIASES: Record<string, string> = {
    connector: 'relay-proxy', // proxy.relay.devnet — the connector's client edge
    relay: 'relay-ws', // relay-ws.devnet — the relay's free WS reads
  };

  it('disables caddy, so the box never binds a host TLS port of its own once the shared edge fronts it', () => {
    const overlay = readSharedEdgeOverlay();

    expect(
      overlay.services[PUBLISHING_SERVICE]?.profiles,
      "docker-compose.shared-edge.yml caddy: expected `profiles: ['never']` (the sentinel docker-compose.local.yml already uses for this service) — the shared edge (infra#24) terminates TLS instead"
    ).toEqual(['never']);
  });

  it('disables the Watchtower overlay too, when it is also in the file set', () => {
    const overlay = readSharedEdgeOverlay();

    expect(
      overlay.services['watchtower']?.profiles,
      "docker-compose.shared-edge.yml watchtower: expected `profiles: ['never']`, the same way it disables caddy — inert unless docker-compose.watchtower.yml is also named in COMPOSE_FILE"
    ).toEqual(['never']);
  });

  it('declares `edge-relay` as an external, per-node network, owned by infra#24 and never created here', () => {
    const overlay = readSharedEdgeOverlay();

    expect(
      overlay.networks?.[EDGE_NETWORK],
      `docker-compose.shared-edge.yml: expected a top-level \`networks.${EDGE_NETWORK}\` entry`
    ).toBeDefined();
    expect(
      overlay.networks?.[EDGE_NETWORK]?.external,
      `docker-compose.shared-edge.yml: \`networks.${EDGE_NETWORK}\` must be \`external: true\` — this bundle joins it, it does not create it`
    ).toBe(true);
  });

  it('never names the old flat `edge` network (shared contract v2, point 1)', () => {
    const overlay = readSharedEdgeOverlay();

    // The flat, every-node-shares-it `edge` network let any node's containers
    // reach any other node's connector operator surface or the gateway's
    // handover port. Contract v2 replaces it with one network per node
    // (`edge-relay` here) — a straight rename, not an addition, so the old
    // key must be gone everywhere this file could carry it: the top-level
    // declaration and every service's own `networks:` map.
    expect(
      Object.keys(overlay.networks ?? {}),
      'docker-compose.shared-edge.yml: the old flat `edge` network must not be declared any more'
    ).not.toContain('edge');

    for (const [serviceName, service] of Object.entries(overlay.services)) {
      expect(
        Object.keys(service.networks ?? {}),
        `docker-compose.shared-edge.yml ${serviceName}: must join \`${EDGE_NETWORK}\`, not the old flat \`edge\``
      ).not.toContain('edge');
    }
  });

  it.each(Object.entries(EDGE_ALIASES))(
    'joins %s to the edge network under the %s alias, without dropping the default network',
    (service, alias) => {
      const overlay = readSharedEdgeOverlay();
      const networks = overlay.services[service]?.networks;

      expect(
        networks?.[EDGE_NETWORK]?.aliases,
        `docker-compose.shared-edge.yml ${service}: expected networks.${EDGE_NETWORK}.aliases to include "${alias}"`
      ).toContain(alias);

      // A service's `networks:` key, once any file sets it, REPLACES that
      // service's network membership rather than adding to it. Without an
      // explicit `default: {}` here, joining `edge-relay` would silently
      // drop this service off the project's own network — the one connector
      // and relay use to reach each other today.
      expect(
        networks,
        `docker-compose.shared-edge.yml ${service}: must keep \`default: {}\` alongside \`${EDGE_NETWORK}\`, or joining the edge network drops it off the network it reaches its peer on`
      ).toHaveProperty('default');
    }
  );

  it('adds a mem_limit to every service the base bundle defines', () => {
    const base = readDockerCompose();
    const overlay = readSharedEdgeOverlay();

    for (const serviceName of Object.keys(base.services)) {
      const limit = overlay.services[serviceName]?.mem_limit;
      expect(
        limit,
        `docker-compose.shared-edge.yml ${serviceName}: expected a \`mem_limit\` — the base bundle defines this service but the overlay gives it no limit`
      ).toBeDefined();
      expect(
        limit,
        `docker-compose.shared-edge.yml ${serviceName}: mem_limit "${limit}" does not look like a Compose memory value (e.g. "192m")`
      ).toMatch(/^\d+[bkmg]$/i);
    }
  });

  it('leaves the base bundle carrying no mem_limit of its own — the overlay is the only source of one', () => {
    const base = readDockerCompose();

    for (const [serviceName, service] of Object.entries(base.services)) {
      expect(
        service.mem_limit,
        `docker-compose.yml ${serviceName}: must not set mem_limit directly — that belongs in docker-compose.shared-edge.yml, or a bare .env (no overlay) stops being byte-for-byte unchanged`
      ).toBeUndefined();
    }
  });

  it('binds no host port when merged with the base bundle and no profile is activated', () => {
    // A lightweight simulation of `docker compose config`'s own merge and
    // profile-filtering, not a reimplementation of Compose: per-service keys
    // an overlay sets replace the base's (the real behaviour for `profiles`,
    // `networks` and `mem_limit`, verified against a real `docker compose
    // config` while writing this overlay), and a service naming a non-empty
    // `profiles` list is excluded unless one of its profiles is activated —
    // which nothing here does, matching a bare `docker compose up -d`.
    const base = readDockerCompose();
    const overlay = readSharedEdgeOverlay();
    const serviceNames = new Set([
      ...Object.keys(base.services),
      ...Object.keys(overlay.services),
    ]);

    const active = [...serviceNames]
      .map((name) => ({
        name,
        ...base.services[name],
        ...overlay.services[name],
      }))
      .filter((service) => (service.profiles ?? []).length === 0);

    expect(
      active.map((service) => service.name).sort(),
      "expected caddy and watchtower to be filtered out by `profiles: ['never']`, leaving only connector and relay active"
    ).toEqual(['connector', 'relay']);

    for (const service of active) {
      const published = (service.ports ?? []).map(resolveComposeDefaults);

      // No active service may bind 80 or 443 — that is now the shared
      // edge's job, off this box entirely (infra#24's "Done when").
      for (const port of EXPECTED_PUBLISHED_PORTS.map(
        (p) => p.split(':')[0] ?? p
      )) {
        const bound = published.find((entry) => entry.split(':').includes(port));
        expect(
          bound,
          `docker-compose.shared-edge.yml: service "${service.name}" binds host port ${port} ("${bound}") with the overlay on — only the shared edge, off-box, may serve TLS now`
        ).toBeUndefined();
      }

      // Any publish this service still has (the connector's on-box operator
      // loopback, unchanged from the default bundle) must stay loopback-only
      // — the same invariant `docker-compose.yml` holds without the overlay.
      for (const entry of published) {
        expect(
          entry.startsWith(LOOPBACK_PUBLISH_PREFIX),
          `docker-compose.shared-edge.yml: service "${service.name}" publishes "${entry}" with no host IP — with the overlay on, nothing but the shared edge may be reachable off-box`
        ).toBe(true);
      }
    }
  });
});

// ── Per-node auto-apply units (shared contract v2, point 2) ─────────────────
//
// Several nodes share one host once the shared edge lands, so the systemd
// pair and the flock this box takes must be scoped to THIS node, not shared
// across every node on the box — a shared name/lock would serialize this
// node's apply against every other node's, or worse, have one node's timer
// silently manage another's units.
describe('the per-node auto-apply units (toon-protocol/relay#166, shared contract v2 point 2)', () => {
  const SERVICE_PATH = resolve(REPO_ROOT, 'deploy/toon-auto-apply-relay.service');
  const TIMER_PATH = resolve(REPO_ROOT, 'deploy/toon-auto-apply-relay.timer');

  it('ships the unit pair under the per-node name, and not under the old shared name', () => {
    expect(
      existsSync(SERVICE_PATH),
      'deploy/toon-auto-apply-relay.service: expected this file to exist'
    ).toBe(true);
    expect(
      existsSync(TIMER_PATH),
      'deploy/toon-auto-apply-relay.timer: expected this file to exist'
    ).toBe(true);

    for (const oldName of ['toon-auto-apply.service', 'toon-auto-apply.timer']) {
      expect(
        existsSync(resolve(REPO_ROOT, 'deploy', oldName)),
        `deploy/${oldName}: the old shared-name unit must be RENAMED, not kept alongside the new one — an existing box's already-installed copy in /etc/systemd/system keeps working regardless (see deploy/README.md's migration section)`
      ).toBe(false);
    }
  });

  it('the timer points at the per-node service name', () => {
    const timer = readFile('deploy/toon-auto-apply-relay.timer');
    expect(
      timer,
      'deploy/toon-auto-apply-relay.timer: [Timer] Unit= must name the per-node service'
    ).toMatch(/^Unit=toon-auto-apply-relay\.service$/m);
  });

  it('the lock auto-apply.sh takes by default is scoped to this node', () => {
    const script = readFile('deploy/auto-apply.sh');
    expect(
      script,
      'deploy/auto-apply.sh: the default LOCK_FILE must be per-node (/var/lock/toon-auto-apply-relay.lock), or two nodes on one host would serialize their applies against each other'
    ).toMatch(
      /LOCK_FILE=\$\{TOON_AUTOAPPLY_LOCK:-\/var\/lock\/toon-auto-apply-relay\.lock\}/
    );
  });
});
