# relay

The TOON Protocol Nostr relay: it stores signed events whose writes the connector has already been
paid for, and serves free reads and read-only app UIs.

## Language

### Payments

**Payment statement**:
What the terminating connector says it verified about the payment for one write: who paid, how
much, and on which chain. The relay records it and never checks it.
_Avoid_: Receipt, proof of payment, attestation

**Tip**:
An ILP payment from one person's node to another person's tip route, the recipient named by their
Nostr pubkey, as opposed to the price of a write.
_Avoid_: Zap, nutzap, donation, payout

**Tip receipt**:
The Nostr event a payer writes to say they tipped a pubkey, optionally naming what the tip is for
(a note, a repo, a goal or a bounty).
_Avoid_: Zap receipt, claim, proof

**Tip acknowledgement**:
The Nostr event a tip's recipient signs to say a tip arrived, and how much of it arrived. A tip
counts as confirmed only when it has one.
_Avoid_: Zap receipt, confirmation, proof of payment

**Tip route**:
The payment route on a person's connector where they can be tipped, named with that connector and
its sealing key in an event the person signs.
_Avoid_: Tip address, payment pointer, payee address, wallet
