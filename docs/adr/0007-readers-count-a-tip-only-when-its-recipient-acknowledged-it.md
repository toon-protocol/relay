# Readers count a tip only when its recipient acknowledged it

**Status:** Accepted (owner decision, 2026-10-26, triage of #311). Applies to `/social` and every reader of the tip NIP in `docs/nips/`. Research: [`docs/tips-on-toon.md`](../tips-on-toon.md).

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/social` page that counts toward a total, a goal's progress, a bounty's payout or a count of payers any tip that has no tip acknowledgement signed by the tip's recipient.

**A reader counts a tip only from a tip acknowledgement signed by the recipient, and counts the amount that arrived.** This is a reader policy over the tip NIP's events; it adds no wire shapes of its own.

## Context

A tip is an ILP payment from a payer's node to a recipient's tip route, forwarded by a hub over peerings. The payer's tip receipt travels only sealed inside the packet, to the recipient's connector. Nothing a payer can publish shows that a tip happened: the fulfilment is derived from a secret the payer seals, so the payer can make one up; the connector states a payment only to the app it terminates at, unsigned, and states nothing on a forward (connector ADR 0040, 0042). The relay validates no payment (CLAUDE.md). Only the recipient's node knows the tip arrived and how much of it did.

## Decision

1. **Only a tip acknowledgement counts.** It is a public event signed by the recipient's key, naming what was tipped and the amount that arrived. A tip without one is not shown or counted.
2. **The amount is what arrived**, after hop fees and any conversion, in the acknowledgement's stated unit. Goal progress (NIP-75 kind 9041) and bounty payouts (30402) sum arrived amounts in the goal's or bounty's unit; amounts in another unit are not converted or summed.
3. **Distinct payers are counted from the payers the acknowledgements name.** A payer who asked not to be named adds to the totals and not to the count of payers.

## Considered options

- **Count the payer's receipts as claims.** Rejected: anyone can publish a receipt for a tip they never sent, so totals and payer counts would be whatever the most generous liar wanted.
- **Count NIP-57 zap receipts (9735).** Rejected: they are written by a recipient's Lightning server, and nothing on TOON carries Lightning.

## Consequences

- A recipient can still inflate their own totals by acknowledging tips that never arrived. That is self-dealing, visible as the recipient's own signature, and bounded by what readers choose to trust about that recipient.
- A tip whose recipient's node never publishes an acknowledgement is invisible, even though the money moved.
