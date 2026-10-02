/**
 * Guards the Rust workspace's standing rules (#192) — the ones that are
 * statements in files rather than behaviour, so no cargo command or
 * conformance case would notice them being undone.
 *
 * It reads the REAL files, like deploy/bundle.test.ts beside it:
 *
 * - one toolchain pin, on edition 2024, with the Rust image's builder on the
 *   same version (a Dockerfile `FROM` cannot read rust-toolchain.toml);
 * - unsafe code forbidden in the workspace, and every crate inheriting that;
 * - NIP-01 handling the relay's own: the framework it was built on stays out
 *   of the workspace, and the protocol crate is on an exact pin (#237);
 * - the connector's crate on one commit, named only for its self-description
 *   types, the attribution header names defined in one module, and the
 *   invariant types' fields private to their modules (#194);
 * - the Rust image's contract with a stack matching the TypeScript image's:
 *   ports, volume, environment defaults, healthcheck and user id;
 * - the Rust image owning `:release` and `:latest` (#205), published with its
 *   release handle built in, and the TypeScript image built but never pushed;
 * - the rollback to the last TypeScript image named as one tag.
 */

import { describe, it, expect } from 'vitest';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parse } from 'smol-toml';
import { parse as parseYaml } from 'yaml';

const REPO_ROOT = resolve(import.meta.dirname, '..');
const TYPESCRIPT_DOCKERFILE = 'packages/relay/Dockerfile';
const RUST_DOCKERFILE = 'crates/relay/Dockerfile';
const PUBLISH_WORKFLOW = 'publish-relay-image.yml';

function readFile(path: string): string {
  return readFileSync(resolve(REPO_ROOT, path), 'utf8');
}

interface Toolchain {
  toolchain: { channel: string; components: string[] };
}

interface WorkspaceManifest {
  workspace: {
    members: string[];
    package: { edition: string };
    lints: { rust: Record<string, string> };
    dependencies: Record<
      string,
      string | { version?: string; git?: string; rev?: string }
    >;
  };
}

interface CrateManifest {
  package: { edition?: { workspace?: boolean }; publish?: boolean };
  lints?: { workspace?: boolean };
}

const toolchain = parse(
  readFile('rust-toolchain.toml')
) as unknown as Toolchain;
const workspace = parse(readFile('Cargo.toml')) as unknown as WorkspaceManifest;

/** Every directory under crates/ that holds a manifest. */
function crateDirs(): string[] {
  return readdirSync(resolve(REPO_ROOT, 'crates'))
    .map((name) => `crates/${name}`)
    .filter((dir) => existsSync(resolve(REPO_ROOT, dir, 'Cargo.toml')));
}

/** A Dockerfile's instructions, comments dropped and continuations joined. */
function instructions(path: string): string[] {
  return readFile(path)
    .replace(/\\\n/g, ' ')
    .split('\n')
    .map((line) => line.trim().replace(/\s+/g, ' '))
    .filter((line) => line !== '' && !line.startsWith('#'));
}

/** The instructions of the final stage: what the shipped image is. */
function finalStage(path: string): string[] {
  const all = instructions(path);
  const last = all.map((line) => line.startsWith('FROM ')).lastIndexOf(true);
  return all.slice(last);
}

function instructionsOf(lines: string[], instruction: string): string[] {
  return lines.filter((line) => line.startsWith(`${instruction} `)).sort();
}

/** Every `.rs` file under `dir`, as a repo-relative path. */
function rustFiles(dir: string): string[] {
  return readdirSync(resolve(REPO_ROOT, dir), {
    withFileTypes: true,
  }).flatMap((entry) => {
    const path = `${dir}/${entry.name}`;
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith('.rs') ? [path] : [];
  });
}

describe('the Rust toolchain is pinned once', () => {
  it('pins an exact release with the two components the gate runs', () => {
    expect(toolchain.toolchain.channel).toMatch(/^\d+\.\d+\.\d+$/);
    expect(toolchain.toolchain.components).toEqual(
      expect.arrayContaining(['clippy', 'rustfmt'])
    );
  });

  it('builds the image with the same release', () => {
    const builders = instructions(RUST_DOCKERFILE)
      .filter((line) => line.startsWith('FROM rust:'))
      .map((line) => line.split(' ')[1]);
    expect(builders).toHaveLength(1);
    expect(builders[0]).toContain(`rust:${toolchain.toolchain.channel}-alpine`);
  });
});

describe('the workspace holds every crate to its rules', () => {
  it('is on edition 2024 and forbids unsafe code', () => {
    expect(workspace.workspace.package.edition).toBe('2024');
    // `forbid`, not `deny`: a crate cannot `#[allow]` its way back out.
    expect(workspace.workspace.lints.rust['unsafe_code']).toBe('forbid');
  });

  it('has every crate under crates/ as a member', () => {
    expect(crateDirs().length).toBeGreaterThan(0);
    expect([...workspace.workspace.members].sort()).toEqual(crateDirs().sort());
  });

  it('has every crate inherit the workspace edition and lints, unpublished', () => {
    for (const dir of crateDirs()) {
      const crate = parse(
        readFile(`${dir}/Cargo.toml`)
      ) as unknown as CrateManifest;
      expect(crate.lints?.workspace, `${dir}: [lints] workspace`).toBe(true);
      expect(crate.package.edition?.workspace, `${dir}: edition`).toBe(true);
      expect(crate.package.publish, `${dir}: publish`).toBe(false);
    }
  });
});

describe("NIP-01 handling is the relay's own", () => {
  // The relay was built on `nostr-sdk`'s `local_relay` (#185), which wraps
  // every stream it is handed in a WebSocket endpoint with a 128 KiB read
  // buffer it gives no way to size. #237 took the fallback #185 names: keep
  // only the `nostr` protocol crate and answer the messages in the relay
  // (crates/relay/src/session.rs). Bringing the framework back, as a
  // dependency of the relay's or of another crate's, brings that cost back.
  const FRAMEWORK = ['nostr-sdk', 'nostr-database'];
  const PROTOCOL = 'nostr';

  it('depends on no framework crate, directly or through another', () => {
    for (const name of FRAMEWORK) {
      expect(workspace.workspace.dependencies[name], name).toBeUndefined();
    }
    const locked = [...readFile('Cargo.lock').matchAll(/^name = "(.+)"$/gm)]
      .map((match) => match[1])
      .filter((name) => FRAMEWORK.includes(name ?? ''));
    expect(locked).toEqual([]);
  });

  it('names no framework crate in a Rust file', () => {
    const names = FRAMEWORK.map((name) => name.replace('-', '_'));
    const importers = crateDirs()
      .flatMap((dir) => rustFiles(dir))
      .filter((path) => {
        const source = readFile(path);
        return names.some((name) => new RegExp(`\\b${name}\\b`).test(source));
      });
    expect(importers).toEqual([]);
  });

  it('pins the protocol crate to one exact version', () => {
    // The store holds this crate's reading of a filter and an event, and the
    // session matches live events with it.
    const dependency = workspace.workspace.dependencies[PROTOCOL];
    const version =
      typeof dependency === 'string' ? dependency : dependency?.version;
    expect(version).toMatch(/^=\d+\.\d+\.\d+$/);
  });
});

describe("the connector's crate is read for its self-description only", () => {
  // `connector-domain` also holds claim and pricing types. All payment-claim
  // validation lives in the connector (CLAUDE.md), so the relay names that
  // crate for the shape of `GET /ilp` and for nothing else (#185).
  const CRATE = 'connector-domain';
  const SELF_DESCRIPTION = [
    // The document and what it is made of.
    'connector_domain::node::',
    // The entries of the document's `batchSettlements`.
    'connector_domain::x402::X402BatchSettlementTerms',
  ];

  it('is a git dependency on one full commit', () => {
    const dependency = workspace.workspace.dependencies[CRATE];
    expect(typeof dependency).toBe('object');
    const { git, rev } = dependency as { git?: string; rev?: string };
    expect(git).toBe('https://github.com/toon-protocol/connector');
    // A full hash: a branch, a tag or a short hash can come to mean another
    // commit.
    expect(rev).toMatch(/^[0-9a-f]{40}$/);
  });

  it('is the only thing any manifest takes from outside the registry', () => {
    // Another crate of the connector's, or this one under another name,
    // would not be seen by the scan of Rust files below.
    type Dependency = string | Record<string, unknown>;
    const tables = (manifest: Record<string, unknown>) =>
      ['dependencies', 'dev-dependencies', 'build-dependencies'].flatMap(
        (table) =>
          Object.entries((manifest[table] ?? {}) as Record<string, Dependency>)
      );
    const sourced = (dependency: Dependency) =>
      typeof dependency === 'object' &&
      ['git', 'path', 'package', 'registry'].some((key) => key in dependency);

    const fromElsewhere = tables(
      workspace.workspace as unknown as Record<string, unknown>
    )
      .filter(([, dependency]) => sourced(dependency))
      .map(([name]) => name);
    expect(fromElsewhere).toEqual([CRATE]);

    for (const dir of crateDirs()) {
      const manifest = parse(readFile(`${dir}/Cargo.toml`));
      const notInherited = tables(manifest)
        .filter(
          ([, dependency]) =>
            typeof dependency !== 'object' || dependency['workspace'] !== true
        )
        .map(([name]) => name);
      expect(
        notInherited,
        `${dir}: every dependency is the workspace's`
      ).toEqual([]);
    }
  });

  it('is named by Rust files only for its self-description types', () => {
    const others = crateDirs()
      .flatMap((dir) => rustFiles(dir))
      .flatMap((path) =>
        readFile(path)
          .split('\n')
          .filter((line) => /\bconnector_domain\b/.test(line))
          .filter(
            (line) =>
              // Every mention on the line must be one of the allowed paths,
              // spelled out: a grouped or renamed import hides what it takes.
              line.split('connector_domain').length - 1 !==
              SELF_DESCRIPTION.reduce(
                (count, allowed) => count + line.split(allowed).length - 1,
                0
              )
          )
          .map((line) => `${path}: ${line.trim()}`)
      );
    expect(others).toEqual([]);
  });
});

describe('the payment attribution headers are named in one module', () => {
  // The connector states a payment in three `X-TOON-*` headers (ADR 0040).
  // Two definitions of their names are two that drift, and a second reader
  // of them is a second place a payment could be claimed (#194).
  const ATTRIBUTION = 'crates/relay/src/write/payment.rs';

  const sources = () => crateDirs().flatMap((dir) => rustFiles(`${dir}/src`));

  it('has no other source file spell an X-TOON header', () => {
    const spelledIn = sources().filter((path) =>
      /x-toon/i.test(readFile(path))
    );
    expect(spelledIn).toEqual([ATTRIBUTION]);
  });

  it('has one reader of a payment statement, in the paid-write handler', () => {
    // The compile-fail tests see the crate from outside. Inside it, what
    // keeps a second handler from claiming a payment is the constructor's
    // visibility and its one call site.
    expect(readFile(ATTRIBUTION)).toContain('pub(super) fn stated_on(');
    const callers = sources().flatMap((path) =>
      readFile(path)
        .split('\n')
        .filter(
          (line) => /\bstated_on\(/.test(line) && !/\bfn stated_on\(/.test(line)
        )
        .map(() => path)
    );
    expect(callers).toEqual(['crates/relay/src/write.rs']);
  });
});

describe('an invariant type keeps its fields to its own module', () => {
  // trybuild shows a forbidden construction failing from outside the crate.
  // A field widened to `pub(crate)` would leave every one of those reasons
  // unchanged and let any module in the relay build the type by hand (#194).
  const INVARIANT_MODULES = [
    'crates/relay/src/verified.rs',
    'crates/relay/src/write/payment.rs',
    'crates/relay/src/edge.rs',
    'crates/relay/src/route.rs',
  ];

  it('declares no field with a visibility', () => {
    for (const path of INVARIANT_MODULES) {
      const source = readFile(path);
      const widened = source
        .split('\n')
        .filter((line) => /^\s+pub(\([^)]*\))?\s+\w+\s*:/.test(line));
      expect(widened, path).toEqual([]);
      // A tuple struct's field: `struct Name(pub …`.
      expect(source, path).not.toMatch(/struct\s+\w+\s*\(\s*pub\b/);
    }
  });

  it('derives no way in: not Default, not Deserialize', () => {
    for (const path of INVARIANT_MODULES) {
      const derives = readFile(path).match(/#\[derive\([^)]*\)\]/g) ?? [];
      expect(derives.length, path).toBeGreaterThan(0);
      for (const derive of derives) {
        expect(derive, path).not.toMatch(/\b(Default|Deserialize)\b/);
      }
    }
  });
});

describe('the Rust image is a drop-in for the TypeScript image', () => {
  const typescript = finalStage(TYPESCRIPT_DOCKERFILE);
  const rust = finalStage(RUST_DOCKERFILE);

  it('is an Alpine runtime carrying one binary', () => {
    expect(rust[0]).toMatch(/^FROM alpine:\d+\.\d+$/);
    expect(instructionsOf(rust, 'COPY')).toEqual([
      'COPY --from=builder /workspace/target/release/relay /usr/local/bin/relay',
    ]);
  });

  it('exposes the same ports, volume and healthcheck', () => {
    for (const instruction of ['EXPOSE', 'VOLUME', 'HEALTHCHECK']) {
      expect(instructionsOf(rust, instruction), instruction).toEqual(
        instructionsOf(typescript, instruction)
      );
      expect(instructionsOf(rust, instruction), instruction).toHaveLength(1);
    }
  });

  it('sets the same TOON_* defaults', () => {
    const toonDefaults = (lines: string[]) =>
      instructionsOf(lines, 'ENV').filter((line) =>
        line.startsWith('ENV TOON_')
      );
    expect(toonDefaults(rust)).toEqual(toonDefaults(typescript));
    expect(toonDefaults(rust).length).toBeGreaterThan(0);
  });

  it('runs as uid and gid 1000, which is `node` in the TypeScript image', () => {
    // node:*-alpine creates `node` as 1000:1000; the files in a deployed
    // /data volume are owned by it.
    expect(instructionsOf(typescript, 'USER')).toEqual(['USER node']);
    expect(instructionsOf(rust, 'USER')).toEqual(['USER relay']);
    const createsUser = rust.find((line) => line.includes('adduser'));
    expect(createsUser).toContain('addgroup -g 1000 relay');
    expect(createsUser).toContain('adduser -D -u 1000 -G relay relay');
    expect(createsUser).toContain('chown relay:relay /data');
  });
});

describe('the Rust image owns :release; the TypeScript image is built and never pushed', () => {
  interface Workflow {
    jobs: Record<
      string,
      { steps?: { uses?: string; with?: Record<string, unknown> }[] }
    >;
  }

  const workflowDir = '.github/workflows';
  const workflows = readdirSync(resolve(REPO_ROOT, workflowDir));
  const builds = workflows.flatMap((name) => {
    const workflow = parseYaml(readFile(`${workflowDir}/${name}`)) as Workflow;
    return Object.values(workflow.jobs)
      .flatMap((job) => job.steps ?? [])
      .filter((step) => step.uses?.startsWith('docker/build-push-action@'))
      .map((step) => ({ workflow: name, with: step.with ?? {} }));
  });

  it('pushes one image, the Rust one, from the publish workflow', () => {
    const pushed = builds.filter((build) => build.with['push'] !== false);
    expect(pushed.map((build) => [build.workflow, build.with['file']])).toEqual(
      [[PUBLISH_WORKFLOW, RUST_DOCKERFILE]]
    );
  });

  it('moves :release and :latest from main only, beside rust-<handle> and rust-sha-*', () => {
    const text = readFile(`${workflowDir}/${PUBLISH_WORKFLOW}`);
    const tagRules = [...text.matchAll(/^\s*type=\w+[^\n]*$/gm)].map((m) =>
      m[0].trim()
    );
    // Every rule of any type, since this file moves `:release`.
    // No unprefixed `sha-*`: those tags are the TypeScript builds, and the
    // last of them is what a rollback names (below). No `rust-candidate`
    // either: there is no candidate once every publish is the release.
    expect(tagRules).toEqual([
      'type=raw,value=release,enable={{is_default_branch}}',
      'type=raw,value=latest,enable={{is_default_branch}}',
      'type=raw,value=rust-${{ needs.handle.outputs.handle }}',
      'type=sha,prefix=rust-sha-',
    ]);
    // metadata-action adds `latest` by itself on some rules; the rule above
    // is the only thing that may.
    expect(text).toMatch(/flavor:\s*latest=false/);
    // The handle that names the tag is the one built into the binary, in the
    // image the suite runs against and in the one pushed.
    expect(
      text.match(
        /TOON_RELEASE_HANDLE=\$\{\{ needs\.handle\.outputs\.handle \}\}/g
      )
    ).toHaveLength(2);
  });

  it('publishes only after the suite passed against the Rust image with nothing expected to fail', () => {
    const workflow = parseYaml(
      readFile(`${workflowDir}/${PUBLISH_WORKFLOW}`)
    ) as {
      jobs: Record<
        string,
        {
          needs?: string | string[];
          steps?: { env?: Record<string, string> }[];
        }
      >;
    };
    expect(workflow.jobs['publish']?.needs).toContain('conformance');
    const env = Object.assign(
      {},
      ...(workflow.jobs['conformance']?.steps ?? []).map((s) => s.env ?? {})
    );
    expect(env['CONFORMANCE_IMPL']).toBe('rust');
  });

  it('declares no case expected to fail for the Rust image', () => {
    const suite = resolve(REPO_ROOT, 'packages/conformance/suite');
    const marked = readdirSync(suite)
      .filter((name) => name.endsWith('.test.ts'))
      .filter((name) =>
        /expectedFailureFor:\s*\[[^\]]*'rust'/.test(
          readFileSync(resolve(suite, name), 'utf8')
        )
      );
    expect(marked).toEqual([]);
  });

  it('reports the release handle as the Rust version, with the crate unpublished at 0.1.0', () => {
    const dockerfile = readFile(RUST_DOCKERFILE);
    expect(dockerfile).toMatch(/^ARG TOON_RELEASE_HANDLE=$/m);
    const manifest = readFile('crates/relay/Cargo.toml');
    expect(manifest).toMatch(/^publish\s*=\s*false/m);
    expect(manifest).toMatch(/^version\s*=\s*"0\.1\.0"/m);
  });

  it('builds both Dockerfiles in CI without pushing either', () => {
    const ci = builds.filter((build) => build.workflow === 'ci.yml');
    expect(ci.every((build) => build.with['push'] === false)).toBe(true);
    // The conformance matrix is where both are built, each under the suite's
    // name for it and the image's own command.
    const workflow = parseYaml(readFile(`${workflowDir}/ci.yml`)) as {
      jobs: {
        conformance: {
          strategy: { matrix: { include: Record<string, string>[] } };
        };
      };
    };
    const matrix = workflow.jobs.conformance.strategy.matrix.include;
    expect(matrix).toContainEqual(
      expect.objectContaining({
        implementation: 'rust',
        dockerfile: RUST_DOCKERFILE,
        command: 'relay',
      })
    );
    expect(matrix).toContainEqual(
      expect.objectContaining({
        implementation: 'typescript',
        dockerfile: TYPESCRIPT_DOCKERFILE,
      })
    );
    // Outside CI the Rust Dockerfile is named by the publish workflow alone,
    // and the TypeScript one by nothing: a workflow that named it could push
    // it back over `:release`.
    const naming = (dockerfile: string) =>
      workflows.filter(
        (name) =>
          name !== 'ci.yml' &&
          readFile(`${workflowDir}/${name}`).includes(dockerfile)
      );
    expect(naming(RUST_DOCKERFILE)).toEqual([PUBLISH_WORKFLOW]);
    expect(naming(TYPESCRIPT_DOCKERFILE)).toEqual([]);
  });
});

describe('a rollback to the TypeScript relay is written down in one tag', () => {
  // The last TypeScript image stays in GHCR under its immutable `sha-*` tag,
  // and pointing RELAY_IMAGE at it is the whole rollback (#205). The tag is
  // the build `:release` pointed at when it flipped, stated here and not read
  // back out of the documents: an operator meets it in several places, and a
  // second spelling would be one of them naming a different build.
  const LAST_TYPESCRIPT_TAG = 'sha-7b6bab5';
  const ROLLBACK_DOCS = [
    'README.md',
    'deploy/README.md',
    'deploy/.env.example',
    'packages/conformance/soak/README.md',
  ];
  // Every `sha-<hex>` a document spells that is not a Rust build's
  // `rust-sha-<hex>`, with or without the image name in front of it.
  const typescriptTagsIn = (path: string) => [
    ...new Set(readFile(path).match(/(?<!rust-)\bsha-[0-9a-f]{7,}\b/g) ?? []),
  ];

  it('names the same TypeScript build wherever an operator looks', () => {
    for (const path of ROLLBACK_DOCS) {
      expect(typescriptTagsIn(path), path).toEqual([LAST_TYPESCRIPT_TAG]);
    }
  });

  it('gives the rollback as the one RELAY_IMAGE line and the one command', () => {
    const runbook = readFile('deploy/README.md');
    expect(runbook).toContain(
      `RELAY_IMAGE=ghcr.io/toon-protocol/relay:${LAST_TYPESCRIPT_TAG}`
    );
    expect(runbook).toContain('docker compose up -d relay');
  });
});
