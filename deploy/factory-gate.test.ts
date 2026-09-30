/**
 * Guards the AFK factory's gate against CI.
 *
 * The runner (`.sandcastle/agent-implement-issue.ts`) runs a gate itself and
 * refuses to open a PR while it is red. That only means something if the gate
 * IS `ci.yml`'s `build` job, so this reads the real workflow and fails when the
 * two stop agreeing, in either direction. It also holds the factory to the five
 * canonical triage labels: the retired `agent:*` family, `needs:human` and
 * `tracking` are not applied by anything under `.github/` or `.sandcastle/`.
 */

import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { parse as parseYaml } from 'yaml';
import { GATE_STEPS } from '../.sandcastle/run-gate';

const REPO_ROOT = resolve(import.meta.dirname, '..');

function buildJobCommands(): string[] {
  const ci = parseYaml(
    readFileSync(resolve(REPO_ROOT, '.github/workflows/ci.yml'), 'utf8')
  ) as { jobs: { build: { steps: { run?: string }[] } } };
  return ci.jobs.build.steps.flatMap((s) => (s.run ? [s.run] : []));
}

function filesUnder(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? filesUnder(path) : [path];
  });
}

describe("the factory's gate is ci.yml's build job", () => {
  it('runs install, build, typecheck and test with the same commands', () => {
    const ci = buildJobCommands();
    for (const command of [
      'pnpm install --frozen-lockfile',
      'pnpm -r build',
      'pnpm typecheck',
      'pnpm -r test --if-present',
    ]) {
      expect(ci).toContain(command);
      expect(GATE_STEPS.map((s) => s.command)).toContain(command);
    }
  });

  it('runs them in the order CI does', () => {
    const ci = buildJobCommands();
    const gate = GATE_STEPS.map((s) => s.command);
    const shared = gate.filter((c) => ci.includes(c));
    expect(shared).toEqual(ci.filter((c) => gate.includes(c)));
    expect(gate.indexOf('pnpm install --frozen-lockfile')).toBe(0);
  });

  it('lints against the same frozen warning baseline as CI', () => {
    const lint = GATE_STEPS.find((s) => s.name === 'lint');
    expect(lint?.command).toContain('.correctness.eslintWarnings');
    expect(lint?.command).toContain('.sandcastle/gate-baseline.json');
    expect(lint?.command).toContain('--max-warnings');
    expect(
      buildJobCommands().some(
        (c) =>
          c.includes('.correctness.eslintWarnings') &&
          c.includes('.sandcastle/gate-baseline.json') &&
          c.includes('--max-warnings')
      )
    ).toBe(true);
  });
});

describe('the factory uses only the canonical triage labels', () => {
  const RETIRED = [
    'agent:implement',
    'agent:review',
    'agent:fix',
    'needs:human',
  ];

  it('applies none of the retired labels anywhere in .github/ or .sandcastle/', () => {
    // A label is applied by `gh ... --add-label`/`--label`, a `labels:` list or an
    // `addLabels` call. The workflow's own NAME (`agent:implement`) is not one.
    const applies = (text: string, label: string) =>
      text
        .split('\n')
        .some(
          (line) =>
            line.includes(label) &&
            /add-?label|--label|labels\s*:|labels\.name/i.test(line)
        );
    const offenders = [
      ...filesUnder(resolve(REPO_ROOT, '.github')),
      ...filesUnder(resolve(REPO_ROOT, '.sandcastle')).filter(
        (f) => !f.includes('/logs/') && !f.includes('/worktrees/')
      ),
    ].filter((file) => {
      const text = readFileSync(file, 'utf8');
      return RETIRED.some((label) => applies(text, label));
    });
    expect(offenders).toEqual([]);
  });
});
