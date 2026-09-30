# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual
label strings used in this repo's issue tracker. This repo uses the canonical names unchanged.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label
string from this table.

These are the only labels the factory uses. The older `agent:implement`, `agent:review`,
`agent:fix`, `needs:human` and `tracking` labels are retired, and nothing here applies them.

## These labels drive the AFK factory

There is no separate trigger label. `ready-for-agent` is the queue, as `to-spec`, `to-tickets`
and `triage` assume: `.github/workflows/agent-implement.yml` picks up every open
`ready-for-agent` issue whose blockers are closed, and turns it into a PR. It runs when the label
is applied, when any issue closes (a blocker may have cleared), every 2 hours, and on dispatch
(`gh workflow run agent-implement.yml -f issue=<n>`, or with no `issue` to sweep).

For each issue the runner (`.sandcastle/agent-implement-issue.ts`) runs `/mattpocock-skills:implement`
in a sandbox, then `/mattpocock-skills:code-review` in a second, fresh session. It then runs this
repo's CI gate itself, with at most 2 fix passes, and never opens a PR while the gate is red. The gate
is the commands of `ci.yml`'s `build` job (`pnpm install --frozen-lockfile`, ESLint against the frozen
warning baseline, `pnpm -r build`, `pnpm typecheck`, `pnpm -r test --if-present`), in `.sandcastle/run-gate.ts`.

The factory moves labels like this:

- **`ready-for-agent`** on an issue: queued. Removed once the agent's PR is open. Put it back to
  retry.
- **`ready-for-human`** on a PR: the agent finished and the gate is green. A human merges. The PR
  body has `Closes #N`, so merging closes the issue.
- **`needs-triage`** on an issue: the AFK run failed. The issue has a comment linking the run, and
  any committed work is on `sandcastle/issue-<n>`.

A spec (an issue with sub-issues, or one written from the to-spec template) is never built
directly, even with `ready-for-agent` on it. Its tickets are.

## What the runner needs

Actions secrets `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`) and `APP_ID` +
`APP_PRIVATE_KEY` (the GitHub App whose token opens the PR, so that `ci.yml` runs on it; the runner
mints a fresh installation token before each push). The sandbox image is prebuilt and pulled from
`ghcr.io/toon-protocol/relay:sandcastle-agent`; when `.sandcastle/Dockerfile` changes, rebuild and
push it with the commands in the comment above the pull step in `agent-implement.yml`. `agent-image.yml`
builds the image on a PR that touches `.sandcastle/` and checks the skills plugin is installed.
