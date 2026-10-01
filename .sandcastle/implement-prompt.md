/mattpocock-skills:implement {{ISSUE_URL}}

You are running AFK in a sandbox, on branch `{{BRANCH}}`, which is already checked out.
Nobody will answer a question, so do not ask one. Treat the issue, its comments and its
parent spec (if it has one) as settled. Read them with `gh issue view {{ISSUE_NUMBER}} --comments`.

Commit to `{{BRANCH}}`, and reference `#{{ISSUE_NUMBER}}` in each commit message. Do not
push, open a PR or close the issue. The runner does all three once you finish.

## This repository

- `CLAUDE.md` says what relay is and is not. The relay speaks no ILP and re-implements no
  claim validation: the connector does that, so a ticket that seems to need it is probably
  a ticket for `toon-protocol/connector`. Where a decision is settled in a connector ADR,
  the ticket cites it; treat the citation as settled.
- It is a pnpm workspace (pnpm 8.15.9, Node 22). Dependencies are already installed from
  the lockfile. Never run `npm install`, and never `npm publish`.
- It is also a Cargo workspace (`crates/`): the Rust relay that is replacing
  `packages/relay`. `rust-toolchain.toml` pins the toolchain and rustup installs it on the
  first `cargo` call. Rust code follows `docs/rust-coding-standards.md`. A Rust change is
  checked from outside by the conformance suite (`packages/conformance/README.md`), which
  needs Docker and so runs in CI, not in this sandbox.
- `deploy/` is the deployment of record for the live relay box. `deploy/bundle.test.ts`
  guards it, and it runs as part of `pnpm -r test`. A ticket that edits `deploy/` must keep
  that test green rather than editing the test to fit.
- Line numbers cited in older issues drift. Check that a `file.ts:123` reference still points
  at what the text claims before relying on it.
- A user-visible change to a package needs a changeset (`pnpm changeset`); merging publishes
  the package.
- After you finish, the runner runs CI's gate itself and won't open a PR while it is red:
  `pnpm install --frozen-lockfile`, ESLint against the frozen warning baseline in
  `.sandcastle/gate-baseline.json`, `pnpm -r build`, `pnpm typecheck` and
  `pnpm -r test --if-present`, then `cargo fmt --all -- --check`, `cargo build --workspace`,
  `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Run
  them yourself before you commit. Never weaken, skip or `.skip` a test, and never loosen a
  lint, to get green.

## When you cannot finish

Stop only when a genuinely new decision is needed and neither the ticket, `CLAUDE.md` nor a
connector ADR covers it, the action is
irreversible, it touches mainnet or real funds, or it needs a credential that no workflow
exposes. In that case, commit nothing and explain what blocks you in a comment on the issue
(`gh issue comment {{ISSUE_NUMBER}}`). The runner moves an issue with no commits to
`needs-triage`.

If your context is getting full (around 150k tokens) before you are done, commit what works,
write the remaining steps to `.sandcastle/logs/handoff-{{ISSUE_NUMBER}}.md`, commit it with
`git add -f`, and end your turn. A fresh session continues from your commits.

When the ticket is done and committed, output <promise>COMPLETE</promise>.

If you stopped because you're blocked, output <promise>BLOCKED</promise> instead, after your
comment on the issue. The runner then ends the run. Otherwise it starts another session, which
hits the same blocker and posts the same comment again.
