# Tip peering policy

How the devnet relay box at `toonprotocol.dev` peers toward people who want to
be tipped through it. Owner decision, 2026-10-26 (#318, part of #311).

A tip travels payer's node → this box's connector → recipient's node. Your
node joining the box (`toon join devnet`) makes a peering one way only, from
you to the box. The box reaches you only once its operator peers back toward
your node. That opens a channel the box funds from its own money (connector
ADR 0075: each side of a peering funds its own outbound channel). So the box
does it on request, on the terms below.

**Until connector#1543 ships, no node can be paid at a tip route,** so there is
nothing yet to peer toward. This page is published ahead of it.

## Who may ask, and how

- **You must have published a tip route**: the kind 10850 event, signed by
  your Nostr key, that names your `connector_url`, `ilp_address` and
  `connector_seal_key` ([the tip NIP](../docs/nips/nip-tips.md)). Publish it
  to this relay.
- **Ask with an issue** on
  [toon-protocol/relay](https://github.com/toon-protocol/relay/issues/new),
  titled `Tip peering: <your npub>`. A tip route is replaceable, so your npub
  is enough: the box reads your latest one.
- **The box peers toward what the signed event says**, never toward a URL or
  address written in the issue. Whoever opens the issue, the peering goes to
  the node your key named.
- Your node must settle on the box's EVM chain (Base Sepolia USDC, the
  `[settlement.evm]` table in [`connector.toml`](connector.toml)).

Only payers whose own node has joined the box can tip you through it. Their
tips arrive at the box as a peer, and a peer's packet is not capped by the
route's price. A payer attached to the box as a client (`toon-client`) is
capped at the route's price, which is the fee alone (connector ADR 0028), so
nothing would arrive.

## What the box puts in

| Term | Value |
| --- | --- |
| Deposit per recipient | **1 USDC**, into the box's outbound channel toward your node |
| Top-up | another **1 USDC** once less than 20% of the channel is left |
| Largest packet | **0.1 USDC**. The box refuses a larger packet (`T04`, connector ADR 0049), so a larger tip must be sent as several packets |
| Removal | the route and peering come off, and the box withdraws what is left in the channel |

## The fee

**1 micro-USDC per tip packet**, flat, whatever the packet carries (connector
ADR 0061). The box takes it from each packet it forwards to you, so what
arrives is what was sent less 1 per packet: a 1 USDC tip sent as ten 0.1
USDC packets arrives as 0.99999 USDC. This page is where payers read it; the arrived amount in your tip
acknowledgement shows it too ([ADR 0007](../docs/adr/0007-readers-count-a-tip-only-when-its-recipient-acknowledged-it.md)).

## Idle peerings

A peering that carried **no tip in 30 days** is dropped: the route and peering
come off, and the deposit is withdrawn. Ask again at any time with a new issue.
You can also ask for removal earlier, by commenting on your issue.

## For the operator

On the box, from `deploy/`, with the operator write key and the connector's
`docs/operators/sign-write.sh` (see [README](README.md#forwarded-legs-and-peering-toward-a-node-behind-nat)):

```sh
# Peer toward a recipient, on the terms above. <id> is short and yours to
# pick, <url> and <prefix> are the connector_url and ilp_address of the
# recipient's latest tip route (kind 10850).
SIGN_WRITE=./sign-write.sh ./runtime-peerings.sh tip <id> <url> <prefix>

# Take it off again.
SIGN_WRITE=./sign-write.sh ./runtime-peerings.sh remove <id> <prefix>
```

`runtime-peerings.sh tip` writes the deposit, fee, packet cap and route price
on this page, and `bundle.test.ts` fails if the script stops writing these
values. Change this page and that test together. The route's price is
the fee alone: a tip arrives from a joined node as a peer, and a route's price
caps only a client-edge packet (connector ADR 0029).

`GET /channels` (bearer token) lists each channel with its `id`,
`direction`, `counterparty`, `collateral` (what was deposited) and `watermark`
(what has been paid on it so far). It names the counterparty's settlement
address, not the peer id, so read it just before and just after `tip`. The new
outbound row is the recipient's, and its `id` goes in a comment on their issue.

Once a week, read `GET /channels` and note each tip channel's `watermark` on
its issue:

- **`collateral − watermark` below 20% of `collateral`:** top up with a signed
  `POST /channels/<id>/fund`, body `{ "amount": 1000000 }`.
- **`watermark` unchanged for 30 days:** `remove` the peering, then withdraw
  the channel with a signed `POST /channels/<id>/withdraw`, once to start and
  again after the channel's delay (a day) to finish, as in the README's cutover
  section. Close the issue.

Keep the EVM settlement key holding Base Sepolia ETH and USDC: every deposit
and top-up is paid from it.
