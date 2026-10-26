# Tips on TOON: what is native, and what is buildable

Research note. Accessed 2026-10-23. It decides nothing; it is input to
relay#311 and connector#1543, not a design. Every substantive claim carries a
label:

- **(fact)**: read in the owning spec, ADR, source file or issue, cited inline.
- **(inference)**: my reasoning from facts. All of §9 is inference.
- **unverified**: I could not confirm it from a primary source.

**The question.** A tip is one person paying another, named by a Nostr pubkey.
`/social` should show tips on notes, repos and profiles, progress toward NIP-75
goals (kind 9041) and bounty payouts (kind 30402), counting distinct payers.
What is the most TOON-native way to do that, given the decisions already
taken on relay#311 (§2)?

**Citation shorthand.**

- `CTX:L<n>` is the connector's `CONTEXT.md`, and `C/<path>:L<n>` is any other
  file in `toon-protocol/connector`, both at `origin/main` `d3e2bd51`
  (2026-10-23). `C-ADR <nnnn>:L<n>` is `C/docs/adr/<nnnn>-*.md`.
- `T/<path>:L<n>` is `toon-protocol/toon_cli` at `origin/main` `2b47fc9`.
  `T/skills/...` is short for `T/vendor/toon-plugins/plugins/...`.
- `N/<path>:L<n>` is `toon-protocol/TOON_Network` at `origin/main` `d72801d`.
- `NIPS/<file>:L<n>` is `nostr-protocol/nips` at `d47a2ec8` (the same pin as
  `social-for-builders-ux.md`).
- `RFCS/<path>:L<n>` is `interledger/rfcs` at `62abd3cc`. `WM:L<n>` is
  `WICG/webmonetization` `specification/index.html` at `3c710521`.
- A bare `path:L<n>` is this repo at `7f52c76`.
- `connector#1543` and `relay#311` are GitHub issues, read on 2026-10-23.

---

## 1. Summary

1. **The send half already exists. The receive half and the hub half do not.**
   A node can originate a packet of any amount to any address
   (`T/src/cli.rs:L501-531`, `C/crates/connector-operator/src/lib.rs:L267-271`)
   **(fact)**. A node cannot be paid at its own address without an HTTP app
   behind a priced route (connector#1543) **(fact)**. And a hub reaches a
   recipient only if the hub's operator peers toward that recipient, funds a
   channel to them, and routes their prefix (§3.7) **(fact)**. So "the hub
   connects everyone" is not automatic: it costs one operator act and some
   liquidity per recipient **(inference)**.
2. **ADR 0028's `F03` cap does not stop a tip.** It applies only when a client
   pays a priced *forwarded* route at the client edge. A payer that is a peer
   of the hub (which is what `toon join` makes it) is forwarded like any
   multi-hop packet: the hub takes its peering fee, and the per-peering cap
   (ADR 0049) is the only bound (§3.3) **(fact)**.
3. **A rejected tip is not charged; a fulfilled one always is.** A forward the
   next hop rejects is rolled back (ADR 0028 #1012; ADR 0075 #1446) **(fact)**.
   A termination's own reject rolls back nothing (ADR 0064), and value moves
   whatever the app answered (ADR 0020) **(fact)**.
4. **"Gift wrap" already means something in the connector, and it isn't
   NIP-59.** It means the packet's payload sealed to the *terminating
   connector's* identity key (`CTX:L190-200`, ADR 0018) **(fact)**. The native
   reading of "gift-wrap the tip to the recipient" is a tip packet whose sealed
   envelope carries the payer's signed Nostr event, the same way a relay write
   carries its event today (§3.8) **(inference)**. A NIP-59 kind 1059 holding a
   voucher would be NIP-61-shaped, but it needs a direct payer→recipient
   channel, which is not "just the peering relationships" **(inference)**.
5. **Nothing in the connector can prove a tip to a third party.** The
   fulfilment derives from a secret the *payer* minted, so the payer can forge
   it (ADR 0018 Update, `N/docs/adr/0016-*.md:L9`) **(fact)**. The payment
   statement is absent on any forwarded or peer arrival, and when it is
   present it names the route's charge, not what arrived (ADR 0040) **(fact)**.
   So the recipient's own node does not learn how much a tip delivered
   through the hub was worth unless the connector changes (§5) **(inference)**.
6. **A reader holding only Nostr events can check signatures, and only
   signatures.** A payer-written tip receipt is a claim. A
   **recipient-written acknowledgement** of that receipt is the strongest
   thing a reader can check: only the recipient's key can forge it, which
   matches what NIP-57 (a receipt signed by the recipient's server) and NIP-61
   (a `redeemed` 7376 from the recipient) offer (§5, §8) **(inference)**.
7. **Discovery should be a recipient-signed replaceable event naming a node
   URL and an address, defined as the person's claim, not the node's.** ADR
   0046 removed the connector's own announce so that no connector needs a
   relay, and it says a third party's announce "is a different object". A
   person's tip address is that different object, so it belongs in a NIP, not
   in the connector (§4) **(inference)**. NIP-A3's `payto` (kind 10133) is the
   closest existing shape, and it has no `ilp` type yet (§4) **(fact)**.
8. **Amounts need a unit even if no chain is named.** The wire never names a
   unit (`CTX:L548-559`), but a goal sums tips from many payers. Fees and
   conversion make "sent" and "arrived" differ (ADR 0061, ADR 0071), and NIP-75
   counts millisats (`NIPS/75.md:L13-20`) **(fact)**. Goal progress should
   count what arrived, as stated in the recipient's acknowledgement (§6)
   **(inference)**.
9. **Upstream moved away from receipts.** STREAM receipts (RFC 0039) are
   deprecated, and Web Monetization now sends no receipts to the browser
   (§8) **(fact)**. That supports a receipt that is a signed Nostr event
   rather than a cryptographic proof **(inference)**.

---

## 2. The decisions under test

As stated on relay#311's triage (the maintainer's words in quotes):

- A tip is an ILP payment from the payer's node to the recipient's node, with
  no chain or payee address stated: "tipping happens through ilp so no chain
  is needed to be stated its just the peering relationships."
- The relay box's connector (Caddy → connector → relay, `deploy/`) is the hop.
  The payer needs a peering into it and the recipient a peering out of it.
  "The relay can just use the event to giftwrap the claim aka tip to the
  recipient". The working reading: the packet's payload is the tip receipt,
  gift-wrapped to the recipient.
- The **tip receipt** is a new regular kind written by the sender, with `p`,
  `e`/`a` and `amount` tags, specified in `docs/nips/`. `/social` counts tip
  receipts toward 9041 goal progress. `GLOSSARY.md:L15-23` already defines
  **Tip** and **Tip receipt** **(fact)**.
- The receiving side doesn't work today. connector#1543 tracks it.

---

## 3. Is the flow buildable on connector law?

### 3.1 The path

The intended path is payer's node → hub (the relay box's connector) →
recipient's node.

- **How a node originates the tip.** `toon send <address> --amount <n>`
  builds a `Prepare{amount, expires_at: now+30s, destination, data}`, seals
  the envelope, and posts it to its own node's `POST /packets`
  (`T/src/operator.rs:L1923-1961`) **(fact)**. The amount is chosen by the
  payer and "a send always states it" (`T/src/cli.rs:L501-531`) **(fact)**.
- **The payer's own node charges no fee.** A packet the operator originates
  pays its own node no fee (ADR 0061 Update #1466, `C-ADR 0061:L146-153`)
  **(fact)**.
- **The address.** A toon node's apps answer at `g.toon.<segment>.<app>`,
  where the segment is the first 16 hex characters of that connector's
  identity key (`T/docs/adr/0006-a-toon-apps-addresses-*.md:L3`) **(fact)**.
  `toon join` forwards `g.toon` and the hub's published prefixes to the hub
  over a new peering at price 0 (`:L17-18`) **(fact)**. So the payer's node
  already sends `g.toon.<anyone's segment>` to the hub **(inference)**.
- **The relay itself stays out of the money.** The relay box's connector
  today has only terminated routes (`deploy/connector.toml`, no `[[peers]]`
  rows) **(fact)**. The relay app speaks no ILP (`CLAUDE.md:L9-10`), so in
  every reading below the *connector* is the hop and the relay only stores
  events **(fact; inference for "every reading")**.

### 3.2 Sealing to the recipient's node

- **What gets sealed.** Every packet's `data` is a gift wrap addressed to the
  identity key of the connector that terminates its route. Inside are the
  envelope (method, target, headers, body) and the shared secret the
  fulfilment derives from (`C-ADR 0018:L7-8`, `:L29-37`) **(fact)**.
- **Sending through a hop.** `toon send --seal-to <url>` seals to another
  connector's key, read from `GET <url>/identity`
  (`T/src/operator.rs:L1809-1833`). Its guide documents sending "to an app
  behind a connector that is not yours" (`T/docs/guide/peering.md:L52-61`)
  **(fact)**. So sealing to the recipient's node across the hub is supported
  today **(fact)**.
- **Hops in between can't read it.** A hop carries bytes it cannot open, and
  only a reject raised at the termination can be sealed
  (`C-ADR 0018:L45-52`) **(fact)**.
- **The wrap says nothing about who sent it.** "A terminating connector MUST
  NOT treat 'this wrap opened' as evidence of who sent it"
  (`C-ADR 0018:L134-138`) **(fact)**. The receiving node therefore learns *who
  paid* only from whatever the payer signs inside the envelope, never from
  the packet **(inference)**.
- **Client destinations (ADR 0032).** If the recipient is a BTP client of the
  hub rather than a peer, the packet is sealed to the recipient's own
  `seal_pubkey`, "a key carried on a signed Nostr event, never a connector's
  `GET /identity`". The client derives its own fulfilment
  (`C-ADR 0032:L48-52`, `:L66-72`) **(fact)**. The machinery is live
  (`C/crates/connector-client-edge/src/session_registry.rs:L1-8`) **(fact)**,
  but connector#1543 notes that "no shipped client (T included) uses" it
  **(fact)**.

### 3.3 Does the hub forward a payer-chosen amount?

- **The `F03` cap is a client-edge rule.** ADR 0028 caps a *client-edge*
  PREPARE to a priced forwarded route at that route's `price`
  (`C-ADR 0028:L65-71`) **(fact)**.
- **A peer arrival is not capped by price.** ADR 0029: "A `Forwarded`
  (`peer_id`) route reached over the peer wire is ordinary multi-hop
  forwarding, fee-metered exactly as it always was; `price` on such a route
  is a client-edge fact" (`C-ADR 0029:L76-79`) **(fact)**.
- **So a node that joined the hub can tip any amount.** `toon join` makes the
  payer's node a *peer* of the hub (§3.1), so the amount it chooses is
  forwarded less the hub's fee **(inference)**.
- **The only bound is the peering's cap.** It is "the largest amount a
  connector will forward to one peer in a single packet". Anything larger is
  refused `T04`, "never carried and never split" (`CTX:L457-464`,
  `C-ADR 0049:L13`) **(fact)**. A tip larger than the cap must be sent as
  several packets **(inference)**.
- **A client payer is a different case.** A payer that attaches to the hub as
  a *client* (for example `toon-client`) does meet ADR 0028's cap on any
  priced forwarded route. What an *unpriced* forwarded route does with a
  client's amount is **unverified**: ADR 0028 says only that it "keeps its
  pre-existing behavior" (`C-ADR 0028:L74-78`).
- **Enforcement on forwards is opt-in.** A covering claim on a *forwarded*
  arrival is enforced per peering behind `forwarded_claim_enforcement`, which
  "still defaults to observing" (`CTX:L424-432`) **(fact)**. A hub carrying
  tips would want `enforce` on every payer peering **(inference)**.

### 3.4 The fee on the hop, and conversion

- **The fee is flat and set per peering.** It is "flat per packet, not
  proportional to the amount carried", and set once per peering (ADR 0061,
  `C-ADR 0061:L7-9`, `:L84-95`; `CTX:L494-506`) **(fact)**.
- **The devnet charges no fee today.** "No devnet box sets a fee at all"
  (`C-ADR 0061:L46`) **(fact)**. So today a tip through the hub arrives whole
  unless the recipient's route has a price, or a rate applies **(inference)**.
- **Conversion.** Where the incoming and outgoing tokens differ, the forwarded
  amount is `floor(amount × rate) − fee`, with the fee in the outgoing unit
  (`C-ADR 0071:L81-86`) **(fact)**. A same-asset pair (USDC on Base to USDC on
  Solana) is forwarded unconverted (`:L88-93`) **(fact)**.
- **Cost is learned by probing.** A probe's reject states the whole path's
  cost in the asker's own unit, as a total with no per-hop breakdown
  (`CTX:L548-559`, `C-ADR 0011:L7-9`). `toon probe` reads it
  (`T/src/cli.rs:L66-68`) **(fact)**.

### 3.5 Does a reject mean the payer isn't charged?

- **Rejected downstream of a forward: not charged.** A forward the next hop
  terminally rejects is rolled back at the client edge (ADR 0028 #1012,
  `C-ADR 0028:L88-100`) **(fact)**. On a peering, since #1446, "a packet that
  was never carried is not paid for": the payer signs its next voucher from
  the receiver's reported watermark (`C-ADR 0075:L852-869`) **(fact)**.
- **Underpaid at a priced termination: not charged.** The packet is refused
  `F03` before the wrap is opened, and "the sending peer is not charged"
  (`C-ADR 0029:L82-85`) **(fact)**.
- **Delivered: always charged.** Value moves "whenever the app answered,
  whatever it answered" (`C-ADR 0020:L82`), and "a termination's reject rolls
  back nothing" (`C-ADR 0064:L48`) **(fact)**. A tip app therefore cannot
  refuse a tip by answering an error **(inference)**.
- **Hostile hops.** "A hop can take a claim and refuse to carry" still
  describes what a hostile hop can do, bounded by the per-peer cap
  (`C-ADR 0075:L855-858`) **(fact)**.

### 3.6 The receiving end: the gap connector#1543 names

- **A route must name a target.** It sets exactly one of `handler_url` or
  `peer_id` (`C/crates/connector-config/src/error.rs:L208-216`) **(fact)**.
  Every route a toon node terminates is an app: `toon add` needs `--image` or
  `--url` (`T/src/cli.rs:L262-296`) **(fact)**.
- **Overpayment is kept, silently.** A priced termination has "No upper
  bound … A peer that overpays a termination is not refused; the money is
  simply not returned either" (`C-ADR 0029:L112-116`) **(fact)**. So sending
  a tip to the recipient's *relay write route*, with the tip receipt as the
  written event and an amount above the price, works today. connector#1543
  lists it as the workaround **(fact)**. The cost: the recipient's app is
  told nothing about the excess (next point), and the receipt lands on the
  recipient's relay, not the hub's relay that `/social` reads **(inference)**.
- **The recipient's app is told nothing on a hub-delivered tip.**
  `X-TOON-Payer`/`-Amount`/`-Chain` are stated "only when a covering client
  claim it verified itself admitted the packet". A peer-wire arrival or a
  forwarded packet states nothing (`C-ADR 0040:L75-77`, `:L103-106`)
  **(fact)**. And `X-TOON-Amount` is "the price this connector charged, never
  the amount field of the arriving packet" (`:L90-92`) **(fact)**.
- **Consequence.** As the connector stands, a tip app behind the recipient's
  node can't learn how much arrived, even though its own connector holds the
  arriving PREPARE's `amount` and the covering voucher **(inference)**.
  connector#1543's open questions ask exactly this ("Should the payee get a
  memo or payer statement?", "What does the fulfilment prove when no app
  answered?") **(fact)**.

### 3.7 The hub's half: "a peering out of it"

- **Only operators create peerings.** A peering is "Created by an operator …
  and by nothing else. It cannot be bought, learned, earned or announced into
  existence" (`CTX:L254-262`) **(fact)**.
- **A peering goes one way.** "They send to you only if their operator peers
  toward you as well", and "a peering alone carries nothing" without a route
  (`T/docs/guide/peering.md:L8-13`) **(fact)**.
- **The hub must pay into a channel per recipient.** A peering is two x402
  channels, one each way (`CTX:L350-355`, ADR 0075 title). For the hub to pay
  a recipient it opens *its own* channel toward them, with a deposit
  (ADR 0075 decision 3) **(fact)**.
- **So "connects everyone" means, per recipient:**
  - the hub operator (or a controller holding a scoped write key with the
    `peers` and `routes-peers` scopes and a deposit limit, ADR 0078 decisions
    1-2, `C-ADR 0078:L9-11`, `:L31-35`) adds a peering toward the recipient's
    node;
  - funds a channel to it;
  - routes `g.toon.<segment>` over it.

  None of this happens today on the relay box **(inference from the facts
  above and `deploy/connector.toml`)**.
- **The client-destination alternative.** If the recipient is a BTP client of
  the hub instead, the hub pays it on a connector→client payout channel that
  the hub also opens and funds (ADR 0075 decision 7, `C-ADR 0075:L236-248`)
  **(fact)**. Either way the hub fronts the liquidity a recipient is paid
  from **(inference)**.

### 3.8 What "gift-wrap the claim" can mean

There are three readings, tested against how TOON payloads already work.

| Reading | What it is | Consistent with connector law? |
| --- | --- | --- |
| **R1: the tip packet carries the receipt** | The payer's node sends one packet to the recipient's tip address. Its sealed envelope is a request whose body is the payer-signed tip receipt event, the same shape as a relay write, `{"event": …}` through the write route (`T/src/event.rs:L1-5`). "Gift wrap" is the connector's own word for that seal (`CTX:L190-200`). | **Yes** (inference). It is ordinary envelope delivery (ADR 0018), it moves value over peerings only, and it names no chain. The receipt's Schnorr signature names the payer, which the wrap deliberately does not (§3.2). |
| **R2: a NIP-59 kind 1059 wrapping a voucher** | The payer signs a voucher on a payer→recipient channel, NIP-59-wraps it to the recipient's pubkey, and publishes it to the relay. The recipient lands it on chain. The connector already has a NIP-59-style *claim* wrap, but between a client and its connector, and "its receiver does learn who sent it" (`C-ADR 0018:L154-157`, `C/crates/connector-signer/src/nip59.rs:L1-23`). | **No, for this goal** (inference). A voucher pays only its channel's receiver, so it can't be routed through the hub. It names a channel and so a chain, and it needs "one funded channel per payee" (connector#1543's workaround cost). It is NIP-61 with x402 channels in place of Cashu. |
| **R3: the relay wraps or forwards the tip** | The relay app relays value or claims. | **No** (fact). The relay "speaks **no** ILP and contains no connector, settlement, or pricing logic" (`CLAUDE.md:L9-10`), and "All payment-claim validation lives ONLY in the connector" (`:L90-91`). |

**Two naming hazards** **(fact; the conclusion is inference)**:

- The connector's vocabulary lists *receipt* as an _Avoid_ term for both
  **fulfilment** and **claim** (`CTX:L74-79`, `:L357-365`).
- The toon CLI's NIP-44 module says "This is not the connector's 'giftwrap',
  which seals packets by a different construction" (`T/src/nip44.rs:L1-8`).

"Tip receipt" is fine as a Nostr kind name, since the relay `GLOSSARY.md`
already scopes it. But a spec should say which "gift wrap" it means every
time.

---

## 4. How does a payer find a pubkey's ILP address?

A payer needs three things: an **address**, a way to get the **sealing key**
for the node that terminates it, and the **unit** the recipient is paid in.

### 4.1 What the connector law says about discovery

- **A connector answers; it never announces.** ADR 0022: it "tells whoever
  asks what its own configuration already says … A sender asks directly and
  pays through the network" (`C-ADR 0022:L7-9`). `GET` on its URL returns the
  self-description, including the sealing key (`C-ADR 0050:L12-14`,
  `CTX:L144-155`) **(fact)**.
- **ADR 0046 removed kind 10032**, because "An announce assumes a relay", and
  a network of pure connectors has none. Whether a node's facts are copied
  into a discovery network "is the controller's business"
  (`C-ADR 0046:L7-9`, `:L13-24`, `:L32-36`) **(fact)**. It also warns: "If a
  controller publishes facts about a node it did not sign, the event is that
  controller's claim about the node … Anyone building that is defining a new
  thing and should say so" (`:L51-54`) **(fact)**.
- **What follows for a tip address** **(inference)**:
  - It must never be something the connector produces or needs.
  - It should be **the person's** claim, signed by their Nostr key, about
    where they accept tips. That is "a new thing", and the NIP should say so.
  - It should not reuse 10032, which old readers still parse
    (`C-ADR 0046:L56-59`).
- **Pinning the sealing key has precedent.** ADR 0032's client destination
  takes `seal_pubkey` "off a signed Nostr event" (`C-ADR 0032:L23-24`).
  TOON_Network ADR 0011 pins a provider's sealing key in a signed profile,
  because "Pinning the key also covers the case where a request reaches the
  provider through hops" (`N/docs/adr/0011-*.md:L3-5`) **(fact)**.
- **But the connector prefers asking directly.** ADR 0022 makes asking the
  primary path, with a signed announce as the *fallback* for a termination
  that can't be reached directly (`C-ADR 0018:L188-195`) **(fact)**. A
  recipient's node behind a hidden service (toon CLI default, its ADR 0003)
  is reachable only through an onion circuit **(inference)**.

### 4.2 Options

| Option | Shape | For | Against |
| --- | --- | --- | --- |
| **A. New replaceable event (10xxx), signed by the recipient** | e.g. `["ilp", "g.toon.<segment>.tip", "<connector url>"]`, optional `["seal", "<key>"]`, `["unit", …]`, `relay` | One self-contained thing; NIP-61's 10019 pattern (`NIPS/61.md:L24-40`); can pin the key (ADR 0032, TN 0011) | A new kind; it is the person's claim, not the node's (ADR 0046 L51-54 must be cited) |
| **B. NIP-A3 `payto` (kind 10133) with a new `ilp` type** | `["payto", "ilp", "g.toon.<segment>.tip"]` | Existing NIP; types are extensible, and none is `ilp` today (`NIPS/A3.md:L16-22`, `:L51-68`) | `payto` holds a type and an address only, so the connector URL or key must ride elsewhere |
| **C. A kind 0 field** | like `lud16` | No new kind | Unstructured; tied to profile edits; it is how LNURL-for-zaps works (`NIPS/57.md:L15`), which is the coupling the skill rejects |
| **D. Reuse NIP-61's 10019** | `mint` → address | Readers exist | 10019's `mint` and `pubkey` mean Cashu, and Cashu wallets would misread it (`NIPS/61.md:L24-40`) |
| **E. Payment pointer / SPSP** | `$host/path` → `GET` → `destination_account` + `shared_secret` (`RFCS/0009-*.md:L84-88`; `RFCS/0026-*.md:L57-72`) | ILP's own answer | TOON already answers with `GET /ilp`, and its secret rides *inside* the wrap (ADR 0018), not in an SPSP reply. TOON speaks ILPv4 semantics with its own encoding (`CTX:L57-65`), and STREAM is not spoken (inference). RFC 0026 itself is deprecated |

**Privacy cost** **(fact, then inference)**:

- The toon CLI keeps the Nostr identity apart from paying keys because "a
  shared key would let anyone link every post to the address the agent pays
  from" (`T/docs/adr/0004-*.md:L3`). An address names its connector's
  identity key (`T/docs/adr/0006-a-toon-apps-*.md:L14`).
- A published pubkey→address event links the person to one connector
  segment. That is opt-in. Segments are per connector "to prevent" linking
  (`:L9`), so a dedicated tip app on its own connector limits what is
  exposed.

---

## 5. Confirmed tip or forgeable claim?

What each candidate source of confirmation actually proves:

| Candidate | Who produces it | What it proves to a third party |
| --- | --- | --- |
| **Payer's tip receipt alone** | payer | Only that the payer signed a claim **(inference)**. Anyone can write one for the price of a write |
| **The fulfilment** | the termination derives it from the sender's secret (`C-ADR 0019`, own L43-44) | **Nothing.** The sender minted the secret; "a fulfilment proves opening, never identity" (`C-ADR 0018:L125-128`); "the Fulfilment derives from a secret the sender minted" (`N/docs/adr/0016-*.md:L9`). STREAM has the same property (`RFCS/0029-stream/0029-stream.md:L484-489`). **(fact)** |
| **The payment statement** | the terminating connector, to its own app | Absent on every forwarded or peer arrival, and it names the route's charge and a channel key, not a Nostr pubkey (`C-ADR 0040:L85-92`, `:L103-106`) **(fact)**. It never reaches a reader: the relay logs it and does not store it with the event (`crates/relay/src/write.rs:L215-223`) **(fact)** |
| **The hub's view** | the hub connector | It sees its two vouchers but not the original payer, because the wrap withholds the sender from "everyone including the termination" (`C-ADR 0018:L159-164`) **(fact)**. A hub-signed statement would be a third-party announce in ADR 0046's sense **(inference)** |
| **STREAM-receipt-like HMAC** | recipient, keyed by a verifier's secret | Deprecated upstream (`RFCS/deprecated/0039-*.md:L4`). Verification is symmetric, so it convinces only the secret holder (`:L53-61`) **(fact)**. `/social` would become a payment verifier, which the relay must not be (`CLAUDE.md:L90-91`) **(inference)** |
| **Recipient-signed acknowledgement event** | recipient's tip app, with the recipient's key (or a key the recipient names) | That the recipient says it received the receipt's tip, and how much arrived. Only the recipient can forge it **(inference)**. This is the same trust as NIP-57 (receipt signed by the recipient's LNURL `nostrPubkey`; "not a proof of payment", `NIPS/57.md:L102`, `:L140`) and NIP-61's `redeemed` 7376 (`NIPS/61.md:L90-110`) **(fact for the NIPs)** |

**What a reader holding only Nostr events can verify** **(inference)**:

- A receipt's signature (who claims to have paid).
- An acknowledgement's signature, and that its author is the receipt's `p`
  (or the goal's beneficiary).
- That the two agree on target and amount.

It can't verify that value moved; nothing in Nostr can. NIP-61 is the only
Nostr scheme where an observer checks value (DLEQ against the mint,
`NIPS/61.md:L113-120`), because there the event *is* bearer value. That is
reading R2, which the hub path rules out.

**Self-dealing remains** **(inference)**. A recipient can tip themselves
through the hub for the cost of the fees, or acknowledge a fake receipt. Zap
goals have the same weakness, which is why "distinct payers" is a weak signal
whichever proof is chosen.

**How to make the acknowledgement honest by construction** **(inference)**:

- The recipient's connector states the arrived amount to its own tip app on
  a *terminated* delivery, even when it arrived over a peering.
- The connector knows that figure: it is the PREPARE's `amount`, covered by
  the hub's voucher.
- That needs a connector ADR. It would amend 0040's "stated only when a
  client claim was admitted" for **amount only**, leaving the payer absent,
  just as ADR 0080 already states the amount without the payer at a zk edge
  (`C-ADR 0080:L3`, `C-ADR 0040:L3`) **(fact for 0080)**.
- It must be a new header, or a new meaning, distinct from `X-TOON-Amount`
  (the charge).

---

## 6. Amounts: sent or arrived?

- **What the payer knows: sent.** The payer chooses `amount` and can learn the
  path's cost by probing (§3.4) **(fact)**.
- **What the recipient knows: arrived.** Arrived = sent − the hub's fee,
  converted at any rate boundary (`C-ADR 0071:L81-86`) **(fact)**. Only the
  recipient's node can know it, and today it isn't told (§3.6) **(fact)**.
- **NIP-75 counts value to the beneficiary.** It tallies zap receipts, whose
  amount is the invoice the recipient was paid (`NIPS/57.md:L169-171`), in
  millisats (`NIPS/75.md:L13-20`). The tally rule itself is only implied by
  the NIP **(fact)**.
- **So goal progress should add up arrived amounts from acknowledgements.**
  An unacknowledged receipt can be shown as "claimed" with its sent amount
  but not counted **(inference)**.
- **A unit is unavoidable** **(fact, then inference)**:
  - The wire never names one; the cost "arrives in the asker's own unit"
    (`CTX:L548-559`).
  - But a node that declares tokens treats "USDC on two chains" as two
    denominations (`CTX:L565-580`), while a same-asset pair is forwarded at
    par today (`C-ADR 0071:L88-93`).
  - A goal needs a unit to sum in. `/social` already shows a 9041's `amount`
    "in millisats" and a bounty's `price` as `50 USDC`
    (`docs/apps/social.md:L640-668`).
  - The cleanest unit is the recipient's: the acknowledgement states arrived
    base units plus the asset it was paid in, and a TOON goal states its
    target in that asset. That names an asset, not a chain or a payee
    address, so it is compatible with the decision in §2.
- **Bounty payouts.** A payout is a tip from the bounty's author, `e`/`a`
  tagging the 30402 **(inference)**. "Paid" is then the acknowledged sum
  against the listing's `price` tag (`NIPS/99.md:L38-42`) **(inference)**.

---

## 7. Where each piece lives

The rule that holds this together: the relay stores events, and its UIs
interpret Nostr. Payment checks live only in the connector (`CLAUDE.md:L90-91`;
`docs/adr/0001-*.md:L18-19`) **(fact)**. Counting signed events is Nostr
interpretation, the same as reading 9735s, so it isn't payment validation
**(inference)**.

| Piece | Home | Why |
| --- | --- | --- |
| Tip address event, tip receipt kind, acknowledgement kind | relay `docs/nips/` | NIPs here "describe wire shapes" (`docs/nips/README.md`) (fact) |
| How `/social` counts (acknowledged vs claimed, distinct payers, `closed_at`, unit) | relay `docs/adr/` | "How the relay reads them … is a decision and belongs in `../adr/`" (`docs/nips/README.md`) (fact) |
| Accepting value at a node's own address; stating the arrived amount on a peer arrival | connector ADR (connector#1543) | Route targets and statements are connector law (`C-ADR 0040`, `C/…/error.rs:L208-216`) (fact) |
| A hub peering toward recipients, and its liquidity | operator, or a controller with a scoped key | Only an operator creates a peering (`CTX:L254-262`); controller keys are scoped (ADR 0078) (fact) |
| `toon tip`, a tip-receiving app, and publishing the address event | toon CLI | It originates packets and writes events (`T/src/operator.rs`, `T/src/event.rs`) (fact) |
| Lifting the social skill's zap ban into a "tip" section | toon-plugins skill | It forbids 9734/9735 today (`T/skills/toon-nostr/skills/social/SKILL.md:L184-186`) (fact) |

---

## 8. Prior art: what maps onto TOON

| Problem | NIP-57 zaps | NIP-61 nutzaps | NIP-75 goals | STREAM receipts / Web Monetization | TOON mapping (inference) |
| --- | --- | --- | --- | --- | --- |
| Find where to pay | LNURL from kind 0 `lud16` → `callback`, `nostrPubkey` (`NIPS/57.md:L15`) | kind 10019: relays, mints, P2PK key (`NIPS/61.md:L24-40`) | the goal's `relays`, and `zap` splits for beneficiaries (`NIPS/75.md:L20`, `:L59`) | payment pointer → SPSP (`RFCS/0009-*.md:L84-88`) | Recipient-signed address event (§4); the node's `GET /ilp` for the key |
| Who writes the record | recipient's LNURL server (`NIPS/57.md:L22`, `:L128`) | sender (`NIPS/61.md:L43`) | n/a (tallies 9735) | receiver issues, verifier checks (`RFCS/deprecated/0039-*.md:L53-61`) | Payer writes the receipt (decided); recipient writes the acknowledgement |
| What an observer can check | server signature, description hash, amount; "not a proof of payment" (`NIPS/57.md:L140`, `:L169-171`) | DLEQ, P2PK lock and mint listed, offline (`NIPS/61.md:L113-120`) | inherits 57 | only the secret-holding verifier (`RFCS/deprecated/0039-*.md:L34-38`); WM now asks the receiver's Open Payments server (`WM:L195-215`, `:L749-757`) | Signatures only; the acknowledgement is the 57-equivalent |
| Redeemed or arrived | implied by the receipt | `redeemed` 7376 (`NIPS/61.md:L90-110`) | n/a | `Total Received` per stream (`RFCS/deprecated/0039-*.md:L91-95`) | Acknowledgement states the arrived amount |
| Splits | `zap` tags with weights (`NIPS/57.md:L175-185`) | none | `zap` tags (`NIPS/75.md:L59`) | n/a | Reuse the `zap` tag *shape* for split weights; the payer sends one packet per beneficiary (inference) |
| Deadline | none | none | `closed_at` (`NIPS/75.md:L38`) | n/a | Count acknowledgements created before `closed_at` |

**What does not map** **(inference)**:

- Zaps need the recipient's server to pay for a write and speak Lightning
  (relay#311).
- Nutzaps need bearer tokens and mints. TOON has no bearer instrument;
  vouchers are cumulative and per channel.
- STREAM receipts are deprecated, and they move trust to a verifier the
  relay must not be.

**What maps** **(inference)**:

- NIP-61's *roles*: the sender writes the record, and the recipient marks it
  received.
- NIP-75's `closed_at` and `zap` split shape.
- Web Monetization's current lesson: ask the receiver. On Nostr, the
  receiver's answer is a signed event.

---

## 9. Most native design (inference)

**End to end.**

1. **The recipient publishes a tip address.** A replaceable event signed by
   their Nostr key names a tip address `g.toon.<segment>.tip`, the node's
   connector URL, optionally a pinned sealing key, and the asset they are
   paid in. Its NIP says it is the person's claim, not the node's
   (§4; ADR 0046 L51-54; ADR 0032 L23-24; TN ADR 0011).
2. **The payer sends one packet.** `toon tip <npub> --amount n [--for <e|a>]`
   reads the address event, probes the path, and sends one packet per
   beneficiary over its peering into the hub. The packet is sealed to the
   recipient's node (ADR 0018), and the envelope body is the payer-signed
   **tip receipt** (`p`, `e`/`a`, `amount` sent). This is reading R1. Amounts
   above the peering cap go as several packets (ADR 0049).
3. **The hub forwards it** as an ordinary peer forward, less its peering fee,
   and rolls back on a downstream reject (ADR 0029 L76-79; ADR 0061;
   ADR 0075 #1446).
4. **The recipient's node terminates it** at its tip address without
   needing a full app (connector#1543), and is told the arrived amount
   (a 0040 amendment for amount only, like 0080). The tip app then
   publishes, to the relay the receipt names:
   - the payer's tip receipt, as received;
   - its own **acknowledgement**, signed by the recipient, which `e`-tags
     the receipt and states the arrived amount and asset. It is deduplicated
     by receipt id.
5. **`/social` shows both, and counts only acknowledged tips.** It counts
   acknowledged tips by distinct payer pubkey, adds arrived amounts toward
   a 9041 (respecting `closed_at`) or a 30402 payout, and shows
   unacknowledged receipts as claims. The relay validates nothing (§5, §7).

**Changes it would need**

| # | Repo | Change | Evidence |
| --- | --- | --- | --- |
| 1 | connector | A way to accept value at a node's own address with no HTTP app, or a documented minimal tip app; settle whether it conflicts with ADR 0032 | connector#1543; `C/…/error.rs:L208-216`; `C-ADR 0032:L48-60` |
| 2 | connector | State the **arrived amount** (not the charge, not the payer) to the terminating app on a peer arrival. An ADR amending 0040, modelled on 0080 | `C-ADR 0040:L90-92`, `:L103-106`; `C-ADR 0080:L3` |
| 3 | connector / ops | Document that a hub reaching recipients is one peering, route and funded channel per recipient, possibly automated by a scoped controller key; set `forwarded_claim_enforcement = "enforce"` on payer peerings | `CTX:L254-262`, `:L424-432`; ADR 0075 decisions 3, 7; ADR 0078 |
| 4 | relay `docs/nips/` | One NIP with three shapes: the tip address event (or an `ilp` type for NIP-A3 `payto`), the tip receipt (payer), and the acknowledgement (recipient); `zap`-shaped split weights; the unit rule | §4, §5, §6; `GLOSSARY.md:L15-23` |
| 5 | relay `docs/adr/` | How `/social` reads tips: acknowledged vs claimed, distinct payers, `closed_at`, summing only one asset, with nothing validated | `CLAUDE.md:L90-91`; `docs/apps/social.md:L665-668` |
| 6 | relay `crates/` (`/social`) | Tips on notes, repos and profiles; goal progress; bounty paid totals | relay#311 |
| 7 | toon CLI | `toon tip` (resolve, probe, send with `--seal-to`, write the receipt); `toon add tip` (receive, acknowledge, publish); publishing the address event | `T/src/cli.rs:L501-531`; `T/src/operator.rs:L1809-1833`; `T/src/event.rs:L1-5` |
| 8 | toon-plugins | Replace the social skill's "no zaps" line with the tip NIP | `T/skills/toon-nostr/skills/social/SKILL.md:L184-186` |

**Findings that cut against the decisions in §2**

- "The payer only needs a peering into it and the recipient only a peering
  out of it": the peering *out* is the hub operator's act, not the
  recipient's, and it costs the hub a funded channel per recipient (§3.7).
- "The relay can just use the event to giftwrap the claim": the relay can't
  touch value. Gift wrap is the connector's seal to the recipient's node, and
  a NIP-59-wrapped voucher can't cross the hub (§3.8).
- "`/social` counts tip receipts toward 9041 goal progress": a payer's
  receipt alone is a forgeable claim, and neither the fulfilment nor the
  payment statement can confirm it. Counting needs a recipient-signed
  acknowledgement, and that acknowledgement needs connector change 2 to know
  the amount (§5, §6).
- "No chain is stated anywhere": that holds for chains, but amounts that are
  summed need an asset unit (§6).

---

## Sources

### Pins

- **Connector**: `https://github.com/toon-protocol/connector` at `d3e2bd51`
  (`origin/main`, 2026-10-23): `CONTEXT.md`; `docs/adr/` 0011, 0018, 0019,
  0020, 0022, 0028, 0029, 0032, 0040, 0042, 0046, 0049, 0050, 0061, 0064,
  0069, 0071, 0075, 0078, 0080; `crates/connector-config/src/error.rs`;
  `crates/connector-operator/src/lib.rs`;
  `crates/connector-client-edge/src/session_registry.rs`, `lib.rs`;
  `crates/connector-signer/src/nip59.rs`.
- **connector#1543**: "A node cannot be paid at its own ILP address without
  an HTTP app behind a route", open, no comments, read 2026-10-23.
- **toon CLI**: `https://github.com/toon-protocol/toon_cli` at `2b47fc9`:
  `src/cli.rs`, `src/operator.rs`, `src/event.rs`, `src/nip44.rs`,
  `docs/adr/0004-*`, `docs/adr/0006-a-toon-apps-addresses-*`,
  `docs/guide/peering.md`, `vendor/toon-plugins/.../social/SKILL.md` (skills
  pinned at toon-plugins `247d1f3`).
- **TOON_Network**: `https://github.com/toon-protocol/TOON_Network` at
  `d72801d`: `docs/adr/0011-a-profile-pins-its-connectors-sealing-key.md`,
  `0016-a-provider-authenticates-continuity-not-identity.md`,
  `0024-a-relay-pins-its-own-paid-write-edge.md`;
  `docs/research/paygress-on-toon-architecture.md` (marked superseded for
  design decisions; it lists a client destination as the variable-amount
  option, L136, and leaves overpayment surfaced in `X-TOON-Amount` as
  UNVERIFIED, L180-189. ADR 0040 L90-92 rules that reading out).
- **This repo**: `toon-protocol/relay` at `7f52c76`: `CLAUDE.md`,
  `GLOSSARY.md`, `crates/relay/src/write/payment.rs`,
  `crates/relay/src/edge.rs`, `crates/relay/src/write.rs`,
  `deploy/connector.toml`, `docs/adr/0001-*`, `docs/nips/README.md`,
  `docs/apps/social.md`, `docs/social-for-builders-ux.md` §7.2 #15-16 and §8;
  relay#311.
- **NIPS**: `https://github.com/nostr-protocol/nips` at
  `d47a2ec8aacb9d01ac00e6294ae77582f483ff68`: 47, 57, 59, 61, 75, 99, A3.
- **RFCS**: `https://github.com/interledger/rfcs` at
  `62abd3cc732fbb7e148f6c21c146ba9630c2edb1`: 0009 (SPSP), 0015 (ILP
  addresses), 0026 (payment pointers, deprecated), 0027 (ILPv4), 0029
  (STREAM; no receipt frame at this SHA), `deprecated/0039` (STREAM receipts;
  deprecated by interledger/rfcs#601, merged 2026-03-11),
  `deprecated/0028` (Web Monetization).
- **WM**: `https://github.com/WICG/webmonetization` at `3c710521`,
  `specification/index.html` (the `receipt` attribute is deprecated in favour
  of an Open Payments `incomingPayment` URL, L749-764).

### Not verified

- What a client-edge PREPARE to an **unpriced** (`price = 0`) forwarded route
  does with a payer-chosen amount (§3.3).
- Whether a packet that arrives at the hub *as a peer* can reach a client
  destination bound there (`session_route::route_prepare` wraps the
  client-edge ingresses; ADR 0026/0027 put both roles on one listener). This
  wasn't traced.
- NIP-75's tally rule is implied rather than stated in the NIP.
- The Open Payments `incomingPayment` authentication model was not read.
