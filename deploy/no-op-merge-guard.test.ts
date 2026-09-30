/**
 * Guards the no-op merge guard (`.github/scripts/no-op-merge-guard.sh`, run by
 * ci.yml's `no-op-merge` job): it fails a PR whose merge result changes zero
 * files and passes one with a real diff.
 *
 * It builds real throwaway repos shaped like `refs/pull/N/merge` — a merge
 * commit whose first parent is the base tip and second is the PR head — and
 * runs the real script in them.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { parse as parseYaml } from 'yaml';

const REPO_ROOT = resolve(import.meta.dirname, '..');
const SCRIPT = resolve(REPO_ROOT, '.github/scripts/no-op-merge-guard.sh');

let dir: string;

function git(...args: string[]): string {
  return execFileSync('git', args, { cwd: dir, encoding: 'utf8' }).trim();
}

function commitFile(
  name: string,
  content: string,
  msg = `write ${name}`
): string {
  writeFileSync(join(dir, name), content);
  git('add', '-A');
  git('commit', '-q', '-m', msg);
  return git('rev-parse', 'HEAD');
}

function runGuard(headSha: string, changedFiles: number) {
  const summary = join(dir, '..', `${Date.now()}-summary.md`);
  const r = spawnSync('bash', [SCRIPT], {
    cwd: dir,
    encoding: 'utf8',
    env: {
      ...process.env,
      GITHUB_EVENT_NAME: 'pull_request',
      GITHUB_STEP_SUMMARY: summary,
      PR_HEAD_SHA: headSha,
      PR_BASE_REF: 'main',
      PR_NUMBER: '1',
      PR_CHANGED_FILES: String(changedFiles),
    },
  });
  return { status: r.status, out: r.stdout + r.stderr };
}

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'noop-guard-'));
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 't@example.com');
  git('config', 'user.name', 't');
  git('config', 'commit.gpgsign', 'false');
  commitFile('a.txt', 'one\n');
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe('no-op merge guard', () => {
  it('passes a PR whose merge result has a real diff', () => {
    git('checkout', '-q', '-b', 'pr');
    const head = commitFile('b.txt', 'new\n');
    git('checkout', '-q', 'main');
    git('checkout', '-q', '--detach');
    git('merge', '-q', '--no-ff', '-m', 'merge', 'pr');
    const r = runGuard(head, 1);
    expect(r.status).toBe(0);
    expect(r.out).toContain('changes 1 file(s)');
  });

  it('fails a PR whose content is already on the base (empty merge)', () => {
    git('checkout', '-q', '-b', 'pr');
    const head = commitFile('b.txt', 'new\n');
    // The same content lands on main by another route.
    git('checkout', '-q', 'main');
    commitFile('b.txt', 'new\n', 'same content, landed by another PR');
    git('checkout', '-q', '--detach');
    git('merge', '-q', '--no-ff', '-m', 'merge', 'pr');
    const r = runGuard(head, 1);
    expect(r.status).toBe(1);
    expect(r.out).toContain('EMPTY commit');
    expect(r.out).toContain('already on main');
  });

  it('fails a PR whose own commits cancel out', () => {
    git('checkout', '-q', '-b', 'pr');
    commitFile('b.txt', 'new\n');
    git('rm', '-q', 'b.txt');
    git('commit', '-q', '-m', 'revert');
    const head = git('rev-parse', 'HEAD');
    git('checkout', '-q', 'main');
    commitFile('c.txt', 'other\n');
    git('checkout', '-q', '--detach');
    git('merge', '-q', '--no-ff', '-m', 'merge', 'pr');
    const r = runGuard(head, 0);
    expect(r.status).toBe(1);
    expect(r.out).toContain('cancel out');
  });

  it('warns and passes when there is no merge ref', () => {
    const r = runGuard(git('rev-parse', 'HEAD'), 0);
    expect(r.status).toBe(0);
    expect(r.out).toContain('no merge ref');
  });
});

describe('ci.yml wiring', () => {
  const ci = parseYaml(
    readFileSync(resolve(REPO_ROOT, '.github/workflows/ci.yml'), 'utf8')
  ) as {
    jobs: Record<string, { name?: string; needs?: string[]; uses?: string }>;
  };

  it('runs the guard in-house, under its check name, with no `uses:` call', () => {
    expect(ci.jobs['no-op-merge']?.name).toBe('No-op merge guard');
    expect(ci.jobs['no-op-merge']?.uses).toBeUndefined();
  });

  it('feeds the CI OK aggregate', () => {
    expect(ci.jobs['ci-ok']?.needs).toContain('no-op-merge');
  });
});
