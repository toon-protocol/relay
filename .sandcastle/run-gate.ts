// The gate, run DETERMINISTICALLY by the runner, not asked of the agent.
//
// An agent's "the tests pass" is a claim, not evidence, and a red gate found by CI
// after a PR is open costs a human a round trip. So the runner runs the gate itself
// and will not open a PR while it is red.
//
// The steps are the commands of ci.yml's `build` job, in the same order, so passing
// here means what passing there means. relay's CI has no path filter, so neither does
// this: every run pays for the whole gate. What CI does that is not a command (the
// ESLint cache, the wall-clock guard) is left out; the lint step reads the same frozen
// warning baseline CI reads. deploy/factory-gate.test.ts fails the build if the two
// drift apart.

import type * as sandcastle from '@ai-hero/sandcastle';

type Sandbox = Awaited<ReturnType<typeof sandcastle.createSandbox>>;

export interface GateStep {
  readonly name: string;
  readonly command: string;
}

export interface GateFailure {
  readonly step: string;
  readonly command: string;
  readonly exitCode: number;
  /** Tail of combined output: enough for an agent to act on, bounded so it cannot blow a prompt. */
  readonly output: string;
}

export interface GateResult {
  readonly passed: boolean;
  readonly ran: readonly string[];
  readonly failure: GateFailure | null;
}

/** Keep fed-back output useful but bounded. A full build log is megabytes. */
const MAX_OUTPUT_CHARS = 12_000;

/** ci.yml's `build` job, step for step. */
export const GATE_STEPS: readonly GateStep[] = [
  { name: 'pnpm install', command: 'pnpm install --frozen-lockfile' },
  {
    name: 'lint',
    command:
      `pnpm exec eslint . --max-warnings "$(jq -r '.correctness.eslintWarnings' ` +
      `.sandcastle/gate-baseline.json)"`,
  },
  { name: 'pnpm build', command: 'pnpm -r build' },
  { name: 'typecheck', command: 'pnpm typecheck' },
  { name: 'pnpm test', command: 'pnpm -r test --if-present' },
];

/**
 * Run `steps` in order, stopping at the first failure.
 *
 * Failure is returned, not thrown, so the caller can decide between a fix
 * iteration and failing the job.
 */
export async function runGate(sandbox: Sandbox, steps: readonly GateStep[]): Promise<GateResult> {
  const ran: string[] = [];

  for (const step of steps) {
    console.log(`  [gate] ${step.name}: ${step.command}`);
    const lines: string[] = [];
    const result = await sandbox.exec(step.command, {
      onLine: (line) => {
        lines.push(line);
        // Stream sparingly: full build output would bury the runner log.
        if (lines.length <= 40) console.log(`    | ${line}`);
      },
    });
    ran.push(step.name);

    if (result.exitCode !== 0) {
      const combined = [result.stdout, result.stderr].filter(Boolean).join('\n');
      const output =
        combined.length > MAX_OUTPUT_CHARS
          ? `...(truncated to the last ${MAX_OUTPUT_CHARS} chars)...\n` +
            combined.slice(-MAX_OUTPUT_CHARS)
          : combined;

      console.log(`  [gate] FAILED at ${step.name} (exit ${result.exitCode}).`);
      return {
        passed: false,
        ran,
        failure: { step: step.name, command: step.command, exitCode: result.exitCode, output },
      };
    }
  }

  console.log(`  [gate] PASSED (${ran.length} step(s): ${ran.join(', ') || 'none applicable'}).`);
  return { passed: true, ran, failure: null };
}

/** The prompt handed to a fix iteration. Concrete failure, no room to reinterpret the task. */
export function fixPrompt(failure: GateFailure, attempt: number, maxAttempts: number): string {
  return [
    `The repository gate is RED. This is fix attempt ${attempt} of ${maxAttempts}.`,
    '',
    `Failing step: ${failure.step}`,
    `Command:      ${failure.command}`,
    `Exit code:    ${failure.exitCode}`,
    '',
    'Output:',
    '```',
    failure.output,
    '```',
    '',
    'Fix the cause and commit. Rules:',
    `- Re-run \`${failure.command}\` yourself and confirm it passes before you finish.`,
    '- Fix the code. Do NOT weaken, skip, delete or ignore a test, and do not',
    '  loosen a lint to make this pass — if the test is genuinely wrong, say so',
    '  explicitly in the commit message and explain why.',
    '- Change only what this failure requires. Do not refactor beyond it.',
    '- If you cannot fix it, commit nothing and explain what is blocking you.',
  ].join('\n');
}
