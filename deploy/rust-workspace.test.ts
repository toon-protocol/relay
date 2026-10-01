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
 * - the Rust image's contract with a stack matching the TypeScript image's:
 *   ports, volume, environment defaults, healthcheck and user id;
 * - the TypeScript image staying the only one any workflow publishes.
 */

import { describe, it, expect } from 'vitest';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parse } from 'smol-toml';
import { parse as parseYaml } from 'yaml';

const REPO_ROOT = resolve(import.meta.dirname, '..');
const TYPESCRIPT_DOCKERFILE = 'packages/relay/Dockerfile';
const RUST_DOCKERFILE = 'crates/relay/Dockerfile';

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
    expect(builders[0]).toMatch(
      new RegExp(
        `^rust:${toolchain.toolchain.channel.replace(/\./g, '\\.')}-alpine`
      )
    );
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

describe('the TypeScript image is the only one published', () => {
  interface Workflow {
    jobs: Record<
      string,
      { steps?: { uses?: string; with?: Record<string, unknown> }[] }
    >;
  }

  const workflowDir = '.github/workflows';
  const builds = readdirSync(resolve(REPO_ROOT, workflowDir)).flatMap(
    (name) => {
      const workflow = parseYaml(
        readFile(`${workflowDir}/${name}`)
      ) as Workflow;
      return Object.values(workflow.jobs)
        .flatMap((job) => job.steps ?? [])
        .filter((step) => step.uses?.startsWith('docker/build-push-action@'))
        .map((step) => ({ workflow: name, with: step.with ?? {} }));
    }
  );

  it('pushes only builds of the TypeScript Dockerfile', () => {
    const pushed = builds.filter((build) => build.with['push'] !== false);
    expect(pushed.length).toBeGreaterThan(0);
    for (const build of pushed) {
      expect(build.with['file'], build.workflow).toBe(TYPESCRIPT_DOCKERFILE);
    }
  });

  it('builds the Rust Dockerfile in CI without pushing it', () => {
    const ci = builds.filter((build) => build.workflow === 'ci.yml');
    expect(ci.every((build) => build.with['push'] === false)).toBe(true);
    // The conformance matrix is where it is built, under the suite's `rust`
    // implementation and the image's own command; no other workflow names it.
    const workflow = parseYaml(readFile(`${workflowDir}/ci.yml`)) as {
      jobs: {
        conformance: {
          strategy: { matrix: { include: Record<string, string>[] } };
        };
      };
    };
    expect(workflow.jobs.conformance.strategy.matrix.include).toContainEqual(
      expect.objectContaining({
        implementation: 'rust',
        dockerfile: RUST_DOCKERFILE,
        command: 'relay',
      })
    );
    const elsewhere = readdirSync(resolve(REPO_ROOT, workflowDir)).filter(
      (name) =>
        name !== 'ci.yml' &&
        readFile(`${workflowDir}/${name}`).includes(RUST_DOCKERFILE)
    );
    expect(elsewhere).toEqual([]);
  });
});
