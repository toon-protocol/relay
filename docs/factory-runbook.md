# Sandcastle factory — first-run runbook (relay)

This is the maintainer runbook for **triggering** relay's sandcastle software
factory. Authoring the runners is done (workflows + labels + entry scripts);
pulling the trigger is a deliberate human action. Nothing in this repo starts an
agent run on its own.

Related:
- Trigger-label + engine spec: `FACTORY.md` (in `toon-meta`).
- Engine reference + verified-on-first-run corrections:
  `scratchpad-sandcastle-engine.md` (in `toon-meta`).
- The runners: `.github/workflows/agent-implement.yml`,
  `.github/workflows/agent-review.yml`.
- The entry scripts: `.sandcastle/agent-implement-issue.ts`,
  `.sandcastle/agent-review-pr.ts`.

---

## Prerequisites (one-time)

- `agent:implement` (`#1D76DB`) and `agent:review` (`#B392F0`) labels exist in
  the relay repo. (Created as part of this slice.)
- Org/repo Actions secrets present:
  - `CLAUDE_CODE_OAUTH_TOKEN` — Claude Max-plan credential (`claude setup-token`).
  - `APP_ID` + `APP_PRIVATE_KEY` — the GitHub App the existing loops already use.
    The engine opens the PR with this App token so the PR triggers `ci.yml`
    (the green gate). A PR opened with the default `GITHUB_TOKEN` would NOT
    trigger CI.
- `agent-implement.yml` / `agent-review.yml` are merged to `main`. Issue- and
  PR-`labeled` workflows only fire from the default branch, so nothing runs
  until these land on `main`.

---

## Trigger the implement checkpoint (the pilot proof)

1. **Pick a small, self-contained issue.** It MUST NOT be an epic/PRD parent:
   no `epic`, `tracking`, or `needs:human` label, and no sub-issues. The runner
   guards refuse those, but pick a clean target anyway. A tightly-scoped bugfix
   or a single small feature is ideal for the first run.

2. **Apply the `agent:implement` label** to that issue. That single act is the
   trigger.

3. **Watch the run** under the repo's Actions tab → **agent:implement**. Two
   jobs:
   - `guard` — evaluates the target (actor write-access, no epic/tracking/
     needs:human label, no sub-issues). If it decides not to proceed you'll see
     a `::warning::` explaining why and the `implement` job is skipped.
   - `implement` — the real work, in order:
     1. checkout + node 22 + `corepack`/`pnpm install`
     2. **build the agent image** (`sandcastle docker build-image`)
     3. **run** `pnpm sandcastle:implement`, which inside the sandbox:
        - **implements** the issue (opus, up to 100 iterations, RED→GREEN→
          REFACTOR, running relay's gate `eslint . / pnpm run typecheck /
          vitest run / pnpm -r run build`),
        - **reviews** the branch (opus, 1 iteration),
        - **opens a PR** against `main` (`Part of #<issue>`, NOT `Closes`).

4. **Expected result:** a new PR from branch `sandcastle/issue-<n>`, with
   `ci.yml` running on it. **The issue is NOT auto-closed and nothing is merged.**

5. **Review and merge the PR yourself.** Merging it (and thereby closing the
   issue) is what satisfies toon-meta#184's "a real agent PR merges" acceptance
   criterion. This is the human half of the checkpoint.

### Reading the logs

- The `implement` job's step **"Run sandcastle implement runner"** streams the
  engine output: the branch name, `implementer` / `reviewer` / `open-pr` phase
  banners, and the agents' tool calls.
- "Implementer produced no commits" in the log → the agent did nothing. No PR is
  opened. Inspect why, then **remove and re-apply** the label to retry.
- A non-zero exit (red job) = the engine or an agent errored. The PR, if any,
  is whatever was pushed before the failure.

### Rollback / abort

- **Before or during a run:** remove the `agent:implement` label and, in the
  Actions tab, **Cancel** the in-progress run.
- **After a bad PR:** just close the PR (and delete the `sandcastle/issue-<n>`
  branch). Nothing was merged, so there is nothing to revert on `main`.
- Re-running is safe: the branch name is deterministic (`sandcastle/issue-<n>`),
  so a retry reuses/updates the same branch rather than spawning duplicates.

---

## Trigger a review pass

1. **Apply `agent:review` to a PULL REQUEST** (not an issue). The
   `agent-review.yml` runner fires on PR label events.
2. The runner (checked out on `main`; it fetches the PR head into a local
   branch itself) runs the reviewer (opus, 1 iteration) along two axes —
   clarity/standards refinements plus a Spec review against the PR's target
   issue (resolved from the body's `Closes #n`) — and **pushes any commits
   back onto the PR**. It never merges or closes anything.
3. The reviewer must end with a structured `<review>` verdict
   (`clean`/`blocking` — toon-meta#275). A malformed verdict **fails the job**;
   a `blocking` verdict posts the findings as a PR review and applies the
   `needs:human` label. See `.sandcastle/review-verdict.ts`.
4. Rollback: remove the label / cancel the run. Any pushed review commits live
   on the PR branch and can be dropped like any other commit.

---

## The formal review verdict (factory-ops)

Once the reviewer emits its structured `<review>` verdict, the runner submits
that verdict as a **real GitHub review**, not just a comment — under a second
identity, **factory-ops**, authenticating via the `FACTORY_OPS_TOKEN` org
secret (toon-meta#282). A second identity is required because
GitHub forbids a PR's author (the factory App) from approving its own PR.

- **`clean` verdict** → factory-ops submits a real **`APPROVED`** review. This
  is what the green tick on an agent PR means.
- **`blocking` verdict** → factory-ops submits a **`CHANGES_REQUESTED`**
  review carrying the reviewer's findings, plus the `needs:human` label.

**A factory-ops approval is a human judgement call by the maintainer, not human judgement.** It
attests only that the gate passed and the sandcastle reviewer found nothing
blocking — it is not a substitute for a maintainer reading the PR.

The approver must never be the PR author. The review runner resolves the
`FACTORY_OPS_TOKEN` identity and compares it against the PR author *before*
the reviewer runs, so a bad credential fails in seconds instead of after a
full opus pass; the implement runner has no PR at that point, so it preflights
the credential alone and the author comparison happens at submission. Either
way, the comparison runs again inside the submission itself.

A `FACTORY_OPS_TOKEN` that is missing, expired, or authenticates as the wrong
identity **fails the review job loudly** — it never degrades to a `COMMENTED`
review. See `.sandcastle/review-verdict.ts`.

---

## The auto-merge toggle (leave OFF for the pilot)

The implement runner ships in **PR mode**: agent opens a PR, human merges. This
is the safe default and there is no merge code in the default path.

**To re-enable auto-merge later**, once the pilot is trusted:

- In `.github/workflows/agent-implement.yml`, in the **"Run sandcastle implement
  runner"** step's `env:`, set:

  ```yaml
  SANDCASTLE_AUTO_MERGE: "true"
  ```

  With that set, `agent-implement-issue.ts` runs the stock merge phase
  (`merge-prompt.md`) instead of opening a PR: it merges the branch into the
  checked-out base and closes the issue.

- **Before trusting it**, prove the merge path on a throwaway issue: the stock
  merge prompt's exact push-to-`main` semantics are inherited from the engine
  and are themselves verify-on-first-run. Branch protection on `main` is the
  recommended backstop even after enabling auto-merge.

The toggle lives in exactly one place — that env var — and is read once in
`agent-implement-issue.ts`.

---

