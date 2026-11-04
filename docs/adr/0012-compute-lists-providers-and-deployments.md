# /compute lists providers and deployments from the directory's own events

**Status:** Accepted (owner decision, 2026-11-04, design session for `/compute`). Applies to `/compute` (`ui-compute`) and to the `/repo` and `/social` pages that link to it. Wire shapes: [`docs/nips/nip-deployment.md`](../nips/nip-deployment.md); guide: [`docs/apps/compute.md`](../apps/compute.md).

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/compute` page that shows a lease's state from anything other than the published events named below (a provider's `status` route, its operator status, an HTTP call to a provider or gateway), or a `/repo` Environments tab that lists a Deployment signed by someone who is neither the repository's owner nor one of its maintainers.

**`/compute` is a fourth read-only app UI ([0001](0001-app-uis-live-on-the-relay.md)): the TOON Compute Provider Directory, and the Deployments tenants choose to publish, drawn only from events in this relay's store.**

## Context

TOON Compute (the `TOON_Network` spec, v1) already has a public directory. A provider signs a Provider Profile (kind 10432), one Listing per tier (30432) and a Liveness (10433, NIP-40 expiry, with capacity per listing), and pays to write them to every relay in its Relay Set. The devnet relay holds them today. Nothing draws them: the archived `console` repo read them into a local daemon, and TOON_Network's ADR 0031 (on a branch) retires it.

What happens to a lease is private by design. `status` is a sealed route authorised by the tenant's token, the operator status is loopback-only (TOON_Network ADR 0029), and a tenant has no published identity (ADR 0016). The only public events about a workload are the provider's Eviction Notice (4433, `x` = workload id) and a standby's Takeover (30433, `d` = workload id). There is no workload log anywhere.

The spec reserves kind 30437, a **Deployment**, for a tenant to say that one environment of one repository is served by a lease. It leaves the shape to "tenant tooling", which was `rig`, now being sunset. The relay is where TOON's Nostr NIPs live, so the shape is fixed here.

The relay's store keeps only the newest replaceable or addressable event per identity, and serves no event past its NIP-40 expiration.

## Decision

1. **`/compute` is an app UI like `/repo` and `/social`.** Crate `ui-compute`, Cargo feature `ui-compute` (on by default), name `compute` in `TOON_UI_APPS`, server-rendered with `script-src 'none'`, a "Compute" link in the site bar, og cards for provider, listing and deployment pages. It reads the store through the read-only handle and fetches nothing.
2. **The directory is what this relay holds.** A provider appears only if it publishes to this relay; the page says so. A Listing whose Profile is not in the store is not shown, since the spec makes it unpurchasable. A provider is **live** while the store holds its unexpired Liveness.
3. **The words are TOON Compute's.** Provider, Listing, Liveness, Workload, Lease, Spawn, Extension, Eviction, Takeover, Deployment, as `TOON_Network/CONTEXT.md` defines them. Never VPS, plan, offer, heartbeat, or deploy for what is a Spawn.
4. **Deployments are read as [`nip-deployment`](../nips/nip-deployment.md) defines them**, and their state comes only from public events:
   - **Running on a live provider until `<expiration>`** while the Deployment is served and its provider is live. A page never claims more than that.
   - **Provider down** while the Deployment is served and its provider has no live Liveness.
   - **Evicted** when the store holds an Eviction Notice whose `x` is the workload id, signed by a provider the Deployment names.
   - **Taken over** when the store holds a Takeover whose `d` is the workload id, signed by a standby the Deployment names, whose `primary` is the Deployment's primary. The page names the winner.
   - An expired Deployment is not served, so it is simply gone.
5. **Whose Deployment counts on a repository is ADR 0002's rule.** The `/repo` Environments tab shows a Deployment only when its signer is the repository's owner or a maintainer the owner's latest 30617 names. Anywhere else a Deployment is shown as its signer's claim, under the signer's name.
6. **The activity log is the kept events.** A provider's page lists its Eviction Notices, the Takeovers naming it, and the Deployments that name it, newest first. There is no uptime or price history, because the store keeps only the newest Liveness, Profile and Listing.
7. **`/social` and `/repo` link in.** A provider page links to `/social/p/<npub>`; a `/social` profile whose key has a 10432 shows a "Compute provider" badge linking back; a repository has an Environments tab.

## Considered options

- **Workload logs.** Rejected for now: no log channel exists, and a public one would publish what the tenant's process prints under a tenant identity the protocol deliberately does not have. A log kind keyed by workload id and encrypted under a key derived from the root secret would need browser JavaScript to read. It is a TOON_Network design question, not a reader's.
- **Expose the operator status.** Rejected: TOON_Network ADR 0029 keeps it off the network on purpose.
- **Ask providers or gateways over HTTP.** Rejected by ADR 0001 (the relay fetches nothing to render a page), and connectors send no CORS headers for a browser to do it either.
- **Keep the Deployment shape in TOON_Network.** Rejected: its spec puts Deployment out of scope, and its readers are the relay's pages.

## Consequences

- An empty Environments tab until a client publishes Deployments. The relay ships the NIP and the reader, not the writer.
- A Deployment is opt-in publicity: publishing one ties the tenant's npub to a workload id, and so to the workload's gateway name. That is the tenant's choice, never needed to rent compute, and the NIP says so.
- A tenant who stops re-signing their Deployment on each Extension lets it expire while the lease still runs. The page errs toward showing nothing rather than a dead workload as running.
- A provider whose Relay Set leaves out this relay is invisible here, however live it is elsewhere.
