# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the
codebase. This repo is **single-context**: the Rust relay under `crates/relay` (#185). The pnpm
workspace beside it holds the conformance suite (`packages/conformance`) and the
deploy guards (`deploy/*.test.ts`).

## Before exploring, read these

- **`CLAUDE.md`** at the repo root: what the relay is and is not, how it is deployed, and where
  payment validation lives (only in the connector).
- **`README.md`**: the operator's guide to running the relay.
- **`docs/`**: the ADRs (`docs/adr/`), the NIP drafts (`docs/nips/`), the app guides
  (`docs/apps/`), the retention policy (`docs/retention.md`), the Rust coding standards
  (`docs/rust-coding-standards.md`) and this directory.

This repo has no `CONTEXT.md` and no `CONTEXT-MAP.md`. Nostr decisions (what the relay serves,
how readers interpret events, app UIs) are recorded here in `docs/adr/`; the NIP drafts are in
`docs/nips/` and the per-app guides in `docs/apps/`. Decisions about ILP, payment, claims and
settlement are the connector's, recorded in
[`toon-protocol/connector`](https://github.com/toon-protocol/connector) (`CONTEXT.md` is its
glossary, `docs/adr/` its decisions). When a ticket cites a connector ADR by number, read it there.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest
creating them upfront. The `/domain-modeling` skill creates them lazily when terms or decisions
actually get resolved.

## Use the vocabulary the docs use

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a
test name), use the term as `CLAUDE.md`, `README.md` and the connector's `CONTEXT.md` define it.
Don't drift to synonyms they avoid. In particular the relay is an **app** behind a connector
route, not a connector, and it speaks no ILP.

## Flag ADR conflicts

If your output contradicts an ADR (this repo's or the connector's), surface it explicitly rather than silently overriding:

> _Contradicts connector ADR-0040 (the connector states the payment on the delivery) — but worth reopening because…_
