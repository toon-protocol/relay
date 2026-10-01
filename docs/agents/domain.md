# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the
codebase. This repo is **single-context**: one package, `@toon-protocol/relay`, under `packages/relay`,
and the Rust rebuild of the same relay under `crates/relay` (#185).

## Before exploring, read these

- **`CLAUDE.md`** at the repo root: what the relay is and is not, how it is deployed, and where
  payment validation lives (only in the connector).
- **`README.md`**: the operator's guide to running the relay.
- **`docs/`**: the retention policy (`docs/retention.md`), the Rust coding standards
  (`docs/rust-coding-standards.md`) and this directory.

This repo has no `CONTEXT.md`, no `docs/adr/` and no `CONTEXT-MAP.md`. The decisions that bind the
relay are recorded in [`toon-protocol/connector`](https://github.com/toon-protocol/connector)
(`CONTEXT.md` is its glossary, `docs/adr/` its decisions), because payment, claims and settlement
are the connector's. When a ticket cites a connector ADR by number, read it there.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest
creating them upfront. The `/domain-modeling` skill creates them lazily when terms or decisions
actually get resolved.

## Use the vocabulary the docs use

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a
test name), use the term as `CLAUDE.md`, `README.md` and the connector's `CONTEXT.md` define it.
Don't drift to synonyms they avoid. In particular the relay is an **app** behind a connector
route, not a connector, and it speaks no ILP.

## Flag ADR conflicts

If your output contradicts a connector ADR, surface it explicitly rather than silently overriding:

> _Contradicts connector ADR-0040 (the connector states the payment on the delivery) — but worth reopening because…_
