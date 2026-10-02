# The paid live feed

The Rust relay can sell its live feed (#215). The contract is the draft NIP
[`paid-subscription`](https://github.com/toon-protocol/toon_cli/blob/docs/nip-paid-subscription/nips/paid-subscription.md)
(toon_cli#43) and toon_cli's ADR 0005: a balance per **subscriber key**, not per
payer. This page is what an operator needs to turn it on and what the relay
decides where the draft leaves it open.

A relay whose operator sets none of the settings below does not sell its feed:
nothing changes, a `REQ` stays open and live for everyone, and the database
file is left exactly as the TypeScript relay wrote it.

## Turn it on

All three, or none (and the connector settings the write edge already needs):

| Variable                     | Meaning                                                                                  |
| ---------------------------- | ---------------------------------------------------------------------------------------- |
| `TOON_SUBSCRIBE_ILP_ADDRESS` | the connector route whose handler is this relay's `POST /subscribe`                      |
| `TOON_BROADCAST_PRICE`       | what one broadcast event debits, in the unit of the write price (a positive integer)     |
| `TOON_RELAY_URL`             | the URL clients reach this relay at (`wss://…` or `https://…`); only its host is checked |

and, optionally, `TOON_OPERATOR_PUBKEYS` (below).

The **subscribe price is not a setting.** It is the price of the route the
connector publishes at `TOON_SUBSCRIBE_ILP_ADDRESS`, read from the connector's
`GET /ilp` like every other fact about where a payment goes. The relay will not
publish `toon_subscription` while the connector does not terminate that address,
prices it by the KiB, or charges nothing for it, and says why in its log.

In the connector's `connector.toml`, add a route beside the write route:

```toml
[[routes]]
prefix = "g.toon.relay.subscribe"
handler_url = "http://relay:3100/subscribe"
price = 1000     # a flat price above zero: one packet credits exactly this
```

`POST /subscribe` is on the write port for the reason `POST /write` is: nothing
but the relay's own connector can reach it. Do not publish that port.

`deploy/` does not turn this on. Its compose file and `connector.toml` stay the
deployment of record for a relay that sells nothing; an operator who sells the
feed adds the route above and the three variables to their own `.env`.

## What a subscriber does

1. Read the NIP-11 document: `toon_subscription` holds the route's address and
   price, and the broadcast price. `supported_nips` lists 42.
2. Pay the subscribe route with a packet carrying `POST /` with a NIP-98
   `Authorization` signed by the subscriber key and a body `{ "filter": … }`.
   Every packet credits the route's price to that key, whoever paid and whichever
   path it took. The first needs a `filter`; a later `{}` only tops up.
3. Connect, answer the relay's NIP-42 `AUTH` challenge with the same key, and
   send a `REQ`. Stored events come first and are free; live events matching the
   subscription's filter and an open `REQ` follow, each debited once, until the
   balance cannot pay for another and the `REQ` is `CLOSED` with
   `payment-required:`.
4. `GET` the relay's URL with `Accept: application/toon-subscription+json` and
   NIP-98 to read the balance.

A read without a subscription is stored events, `EOSE`, then `CLOSED` with
`auth-required:` (not authenticated) or `payment-required:` (authenticated, no
balance).

## The operator

**Following the feed without paying.** The draft leaves how an operator names
their key to the relay. Here, a connection that authenticates (NIP-42) with

- the relay's own identity key (`TOON_SECRET_KEY`), or
- a key listed in `TOON_OPERATOR_PUBKEYS` (comma-separated hex public keys)

reads the live feed with no subscription and no debit. The identity key is
implied because whoever holds it is the operator by definition; the list is for
a key you would rather not carry the identity key around as.

**Listing subscribers.** `GET /subscribers` on the write port returns every
subscription and its balance:

```json
{ "broadcast_price": 10, "subscribers": [{ "pubkey": "…", "balance": 990, "broadcast_price": 10, "filter": {} }] }
```

It is on the write port because it names keys and balances; like `/metrics` it
is for the operator and must not be published.

## How it is kept

- **One table, only added.** Balances are in `feed_subscriptions` in `events.db`.
  It is created only when the relay sells its feed; no TypeScript table is
  touched, so the last TypeScript image still opens the file.
- **Memory is the book, the table follows.** A debit happens when an event is
  accepted, on the write path, so it is made in memory and queued to one writer
  thread. A credit waits for its row to be on disk before the subscriber is
  answered; a credit whose row cannot be written is taken back and the request
  refused with `500`. A crash can lose the last few debits (a subscriber gets
  those events for free) and never a credit.
- **A balance stops at 2^53 − 1**, the largest a JSON number carries exactly.
  A packet's `credited` is what it added, which is less than its price only at
  that ceiling.
- **Charged once, in acceptance order.** The write path decides, per accepted
  event, which subscribers pay: those whose subscription filter matches the
  event and who hold an open `REQ` on any connection that matches it. Each is
  debited one broadcast price however many `REQ`s or connections it is sent on,
  and a balance left under one price closes its `REQ`s directly after the last
  event it paid for.

## Known edges

- An event saved after a `REQ` is registered and before the store is queried can
  be in the stored answer **and** charged; it is sent once, and the charge stands.
  The window is the length of one store query.
- A packet is credited once per delivery to `POST /subscribe`. The relay has no
  packet identity to tell a redelivery from a new packet; the connector delivers
  each fulfilled packet once.
- A debit is made for a broadcast, not a receipt (the draft says so): an event
  debited and lost to a dropped connection can be read for free as a stored event.
- Reads are not rate-limited yet, so a reader can poll instead of paying (the
  draft's own limit).
