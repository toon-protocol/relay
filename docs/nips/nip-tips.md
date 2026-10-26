NIP-XX
======

Tips on TOON: tip route, tip receipt and tip acknowledgement
------------------------------------------------------------

`draft` `optional`

This draft defines how one person tips another on TOON, and how that tip becomes something a
reader can show and count. It adds three events: a **tip route** the recipient signs, a **tip
receipt** the payer signs and seals into the payment, and a **tip acknowledgement** the
recipient signs once the tip has arrived.

It is a draft for the maintainer. Publishing it through the NIP process is out of scope here.
The words *Tip*, *Tip route*, *Tip receipt*, *Tip acknowledgement* and *Payment statement* are
used as [`GLOSSARY.md`](../../GLOSSARY.md) defines them. The decisions behind this draft are
relay#311, [ADR 0007](../adr/0007-readers-count-a-tip-only-when-its-recipient-acknowledged-it.md)
and the research note [`docs/tips-on-toon.md`](../tips-on-toon.md).

## Background

A tip is an ILP payment from the payer's node to a tip route on the recipient's connector. TOON
carries no Lightning and no Cashu, so [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)
zaps do not fit: a zap receipt (9735) is written by the recipient's Lightning server, and a
nutzap (9321, [NIP-61](https://github.com/nostr-protocol/nips/blob/master/61.md)) is a bearer
Cashu token. Neither is reused (see [Relation to other NIPs](#relation-to-other-nips)).

Nothing a payer can publish proves that a tip happened: the ILP fulfilment derives from a secret
the payer seals, so the payer can forge it, and the connector states a payment only to the app it
terminates at. Only the recipient's node knows that a tip arrived and how much of it did. So the
payer's event is only the payer's word, carried to the recipient, and the recipient's event is what readers
count.

## Kinds

These kinds were chosen because none of them appears in the upstream
[NIP index](https://github.com/nostr-protocol/nips/blob/master/README.md#event-kinds)
(checked 2026-10-26), and none is used by another draft in this directory.

| event | kind | NIP-01 class / replacement identity | signer | published to a relay |
| --- | ---: | --- | --- | --- |
| Tip route | 10850 | normal replaceable: `(kind, pubkey)` | recipient | yes |
| Tip receipt | 9850 | regular | payer | **never** |
| Tip acknowledgement | 9851 | regular | recipient | yes |

Do not reuse kind 10032. That was a connector announcing itself, and it was removed
(connector ADR 0046). A tip route is a person's own statement about where they can be tipped.

## Amounts

An amount is a tag of three values after the name: a non-negative integer in base units, the
asset code, and the scale (the number of decimal places the code's major unit has):

```
["amount", "2500000", "USDC", "6"]
```

The tag names an asset, never a chain and never a payee key. Two amounts are in the same unit
when their code and scale are both equal. Amounts in different units are never converted or
summed.

## Tip route (10850)

A replaceable event the recipient signs. It names a connector, the payment route on that
connector where they can be tipped, and the key that connector seals to. It uses the spelling of
the Provider Profile (TOON_Network ADR 0011 and ADR 0024, spec §4.1 and §13): `connector_url`,
`ilp_address` and `connector_seal_key`.

```json
{
  "kind": 10850,
  "pubkey": "<recipient pubkey>",
  "created_at": 1761480000,
  "tags": [
    ["connector_url", "https://node.example.com"],
    ["ilp_address", "g.toon.1a2b3c4d5e6f7a8b.tip"],
    ["connector_seal_key", "<hex sealing key of that connector>"]
  ],
  "content": ""
}
```

| tag | required | meaning |
| --- | --- | --- |
| `connector_url` | yes | Base URL of the recipient's connector. The payer reads `GET /ilp` there. |
| `ilp_address` | yes | The payment route on that connector that terminates at the recipient's tip app. |
| `connector_seal_key` | yes | The connector's sealing key. The payer pins it. |

No chain, payee key, price or carriage appears in the event. Carriage, minimum and terms change,
so a payer reads them from the connector's own `GET /ilp` at send time.

A payer:

- MUST refuse to send when `GET /ilp` at `connector_url` does not list `ilp_address` as a
  payment route.
- MUST refuse to send when the sealing key `GET /ilp` states disagrees with `connector_seal_key`.

This is the person's statement, not the node's. A reader or payer takes the newest event of the
recipient's, and treats it as a claim the connector then confirms or contradicts.

## Tip receipt (9850)

A regular event the payer signs. It says who is tipped, what for, and how much was **sent**.

```json
{
  "kind": 9850,
  "pubkey": "<payer pubkey>",
  "created_at": 1761480100,
  "tags": [
    ["p", "<recipient pubkey>"],
    ["a", "30617:<owner pubkey>:tidepool"],
    ["e", "<goal 9041 event id>"],
    ["amount", "2500000", "USDC", "6"]
  ],
  "content": "Thanks for the release."
}
```

| tag | required | meaning |
| --- | --- | --- |
| `p` | yes | The recipient. Exactly one. |
| `e` | no | What is tipped: a note, or a goal (9041). |
| `a` | no | What is tipped: a repo (`30617:…`), or a bounty (`30402:…`). |
| `amount` | yes | The amount **sent**, with its unit. |
| `anon` | no | A request not to be named. When present, the acknowledgement omits the payer. |

`content` is an optional message. A receipt with no `e` and no `a` tips the person.

**The receipt is never published to a relay.** It travels only as the sealed payload of the tip's
ILP packet to the recipient's tip route, and only the recipient's node sees it. Nothing here
asks a reader to look for one, and a reader MUST NOT count one it finds.

## Tip acknowledgement (9851)

A regular, public event the **recipient** signs after the tip arrives. It is the only public tip
event, and the only one a reader counts.

```json
{
  "kind": 9851,
  "pubkey": "<recipient pubkey>",
  "created_at": 1761480130,
  "tags": [
    ["a", "30617:<owner pubkey>:tidepool"],
    ["e", "<goal 9041 event id>"],
    ["amount", "2490000", "USDC", "6"],
    ["p", "<payer pubkey>"],
    ["receipt", "<id of the tip receipt>"]
  ],
  "content": ""
}
```

| tag | required | meaning |
| --- | --- | --- |
| `e` | no | What was tipped, copied from the receipt. |
| `a` | no | What was tipped, copied from the receipt. |
| `amount` | yes | The amount that **arrived**, after hop fees and any conversion, with its unit. |
| `p` | no | The payer. Omitted when the receipt carried `anon`. |
| `receipt` | yes | The id of the tip receipt, for deduplication. |

The author of the acknowledgement is the recipient; no tag repeats that. Two acknowledgements by
the same author with the same `receipt` value are one tip.

## Reader rule

This restates [ADR 0007](../adr/0007-readers-count-a-tip-only-when-its-recipient-acknowledged-it.md),
which is the authority.

1. **Only a tip acknowledgement counts**, signed by the recipient. A tip without one is not
   shown or counted.
2. **The amount is what arrived**, in the acknowledgement's unit. Arrived amounts are summed per
   unit. Goal progress (NIP-75 kind 9041, a TOON convention for it) and bounty payouts (30402) sum
   arrived amounts in the goal's or bounty's unit; amounts in another unit are neither converted
   nor summed.
3. **Distinct payers are counted from the payers the acknowledgements name.** A tip whose
   acknowledgement has no `p` adds to the totals and not to the count of payers.

A recipient can still acknowledge a tip that never arrived. That is self-dealing, visible as
their own signature, and bounded by what a reader chooses to trust about them.

## Flow (informative)

```
payer's node ──peering──▶ hub (the relay box's connector) ──peering──▶ recipient's tip route
   seals the receipt         forwards, keeps its peering fee           connector terminates at
   to the recipient's        and rolls back on a downstream reject     the tip app, which signs
   connector                                                            the acknowledgement
```

1. The payer reads the recipient's tip route, then `GET /ilp` at `connector_url`, and applies the
   two refusal rules above.
2. The payer's node sends one packet to `ilp_address`, sealed to `connector_seal_key`, whose
   payload is the tip receipt.
3. The hub forwards it over peerings less its peering fee. The route to a recipient exists only
   if the hub's operator peers toward them.
4. The recipient's connector terminates it at the tip route. Accepting a payment at a node's own
   address and stating the amount that arrived are connector work (connector#1543).
5. The recipient's tip app signs the acknowledgement and publishes it.

The relay validates no payment. It verifies an event's signature, stores it, and serves reads.
Payment checks live only in the connector.

## Relation to other NIPs

- **NIP-57 (zaps).** Not reused: 9734 and 9735 are bound to LNURL and a Lightning server that
  signs the receipt. The analogue of that server's receipt here is the acknowledgement, with the
  same trust: it is the recipient's word, not a proof of payment.
- **NIP-61 (nutzaps).** Not reused: 9321 carries a Cashu proof, and 10019 names mints. Borrowed:
  the roles (the sender writes the record, the recipient marks it received) and the replaceable
  recipient-signed discovery event.
- **NIP-75 (zap goals).** A goal (9041) is tipped with an `e` tag. Its `amount` is in millisats
  upstream; on TOON a goal states its target in the unit its readers sum (a TOON convention).
- **NIP-99 (classified listings).** A bounty (30402) is tipped with an `a` tag.
- **NIP-34 (repositories).** A repo (30617) is tipped with an `a` tag.
