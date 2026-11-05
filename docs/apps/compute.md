# Compute on TOON: providers and deployments

A protocol guide for TOON Compute as the relay's `/compute` pages read it: the
events a provider publishes to be found, the Deployment a tenant may publish to
show where a repository runs, and where each one appears. It names no client,
CLI or library: anything that can sign Nostr events and pay for writes can
follow it.

Sources, in order of authority:

- The `TOON_Network` spec, v1 (`docs/spec/toon-network-v1.md`): §4 the Provider
  Directory, §6.7 the Eviction Notice, §7.1 the Takeover. Its `CONTEXT.md` is
  the glossary this guide's words come from.
- [`../nips/nip-deployment.md`](../nips/nip-deployment.md) — the Deployment
  (kind 30437).
- [ADR 0001](../adr/0001-app-uis-live-on-the-relay.md) — the relay's UIs read
  only this relay's store, write nothing and fetch nothing.
- [ADR 0002](../adr/0002-readers-honour-repo-state-only-from-authorized-authors.md)
  — authorized authors: the owner plus the owner's `maintainers`.
- [ADR 0012](../adr/0012-compute-lists-providers-and-deployments.md) — what
  `/compute` shows and why.

`P` means a provider's pubkey and `A` a repository's address,
`30617:<owner pubkey>:<d>`.

## How writes work

Every event below is written by sending it to the relay's `POST /write`
through the connector, which enforces payment; reads are free. A provider pays
for each Profile, Listing and Liveness it publishes, and a tenant pays for each
Deployment. `/compute` shows only what this relay holds: a provider whose Relay
Set leaves this relay out is not listed here.

## What `/compute` reads

| Event | Kind | Signer | What the page takes from it |
| --- | ---: | --- | --- |
| Provider Profile | 10432 | provider | isolation, hidden, host, settlement, Relay Set, Liveness cadence |
| Listing | 30432, `d` = listing | provider | resources, arch, `lease_interval_s`, `price`, `standby_price`, capabilities; tags `l` (isolation, arch, hidden, gpu), `t`, `g` |
| Liveness | 10433 | provider | live while unexpired; `available` per listing |
| Eviction Notice | 4433 | provider | `x` = workload id, `reason`, `message` |
| Takeover | 30433, `d` = workload id | standby | `primary` it took over from |
| Deployment | 30437, `d` = environment | tenant | see [`nip-deployment`](../nips/nip-deployment.md) |

Every one carries `["L","toon.network"]`, so `{"kinds":[10432,30432,10433],"#L":["toon.network"]}`
is the whole directory. A Listing joins its Profile through its `a` tag,
`10432:P:`. A Listing with no Profile in the store is not shown: the spec makes
it unpurchasable.

**Price.** `price` is µUSDC per Lease Interval. The page shows it exactly, per
interval, and per hour as `price × 3600 / lease_interval_s`. A Listing with
`standby_price` also shows the standby price; one without it sells no standbys.

**Live.** A provider is live while the store holds its unexpired Liveness. Its
`available` count is the capacity shown for each listing. The store keeps only
the newest Liveness, Profile and Listing, so there is no uptime or price
history to show.

## Being listed as a provider

Publish the three directory events as the spec's §4 says, to a Relay Set that
includes this relay, and keep Liveness inside its expiry. Nothing else is
needed to appear on `/compute`.

## Showing where a repository runs

Publish a Deployment (kind 30437) as [`nip-deployment`](../nips/nip-deployment.md)
says, and re-sign it on every Extension, Takeover and new image. It is
optional and public: it ties your key to the workload id and its gateway name.

To appear on the repository's own Environments tab, sign it with the
repository owner's key or a maintainer's (ADR 0002). A Deployment signed by
anyone else still has its own page, shown as that key's claim.

## Reading in a browser: the relay's `/compute` pages

Built so far: `/compute`, the Provider and Listing pages (`/compute/<naddr>` redirects
to the Listing's page), the Deployment page (its `naddr` redirects too), the provider
page's Serving section and the `/social` badge; the other rows are the agreed shape
(ADR 0012).

| Path | What it shows | What it reads |
| --- | --- | --- |
| `/compute` | The Provider Directory: live providers first, then by price; `?arch=`, `?isolation=`, `?gpu=`, `?cap=`, `?hidden=` filter | kinds 10432, 30432, 10433 by `#L` |
| `/compute/p/<npub>` | One provider: its Listings, live status, what it is serving, its activity log, a link to `/social/p/<npub>` | its 10432, 30432, 10433 and 4433; 30433 by its Deployments' workload ids; 30437 by `#p` |
| `/compute/<npub>/<listing>` | One Listing: resources, price, capacity, capabilities, the provider | the 30432 at that address, the provider's 10432 and 10433 |
| `/compute/d/<npub>/<environment>` | One Deployment: repository at its commit, provider and Listing, image, Visit link, state | the 30437 at that address; the 30617 it names; the provider's 10433; 4433 by `#x`; 30433 by `#d` |
| `/repo/<npub>/<d>/environments` | The repository's Environments tab: each environment's provider, commit, state and Visit link | 30437 by `#a` = `A`, signed by an authorized author |
| `/social/p/<npub>` | A "Compute provider" badge when the key has a 10432 | the key's 10432 |

**Link previews (ADR 0006).** The Provider, Listing and Deployment pages name a 1200×630 card
in `og:image`, an absolute URL under `TOON_UI_PUBLIC_URL`, drawn from the same events as the page:
`/compute/og/p/<npub>.png` (name, live status, number of Listings),
`/compute/og/l/<npub>/<listing>.png` (resources, price per hour) and
`/compute/og/d/<npub>/<environment>.png` (repository, environment, provider, state). Each answers
`ETag`/`304`; a card for something the store does not hold is `404`. The pages' policy is unchanged.

**A Deployment's state** is one of: running on a live provider until its
expiration; provider down; evicted (an Eviction Notice for its workload id from
a provider it names); taken over (a Takeover from a standby it names, naming
the winner). An expired Deployment is not served, so it is gone, not shown as
ended.

**A provider's activity log** is its Eviction Notices, the Takeovers naming it
and the Deployments naming it, newest first.

**Hidden Providers** carry `l` `hidden:true`; their pages say "Anyone-only"
and show no host.

**Out of v1:** Templates, Image Registry entries and Blob Records
(30434–30436), and Deployments in the `/social` feed.
