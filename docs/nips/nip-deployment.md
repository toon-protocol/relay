NIP-XX
======

Deployments: which lease serves a repository's environment
----------------------------------------------------------

`draft` `optional`

This draft fixes the shape of the TOON Compute **Deployment**, kind `30437`: a tenant's signed
statement that one environment of one [NIP-34](https://nips.nostr.com/34) repository, at one
commit, is served by one lease on one provider, until a stated time.

TOON Compute (the `TOON_Network` spec, v1, §3.1.2) reserves the kind and leaves its shape to
tenant tooling; this draft is that shape. Providers never publish, read or depend on a
Deployment. It is a draft for the maintainer; publishing it through the NIP process is out of
scope here. *Provider*, *Listing*, *Liveness*, *Lease*, *Workload*, *Spawn*, *Extension*,
*Eviction Notice*, *Takeover* and *Standby Set* are used as `TOON_Network/CONTEXT.md` defines
them. How the relay reads it is [ADR 0012](../adr/0012-compute-lists-providers-and-deployments.md).

## A Deployment is optional and public

Nothing a tenant does to rent compute is a Nostr event, by design (TOON_Network ADR 0016). A
Deployment is the one exception, and only by the tenant's choice: it ties the signing key to a
workload id, and so to the workload's gateway name and, when the tenant gives one, its public
URL. Renting, extending and ending a lease never need one. Publish it only for a workload you
want people to find, the way a repository shows its environments.

## Kind

| event | kind | NIP-01 class / replacement identity | signer | lifetime |
| --- | ---: | --- | --- | --- |
| Deployment | 30437 | addressable: `(kind, pubkey, d)` | tenant | until its `expiration`, at most the lease's `expires_at` |

The number is the one `TOON_Network` spec §3.1 allocates.

## Event

```jsonc
{
  "kind": 30437,
  "pubkey": "<tenant pubkey>",
  "created_at": 1762250000,
  "tags": [
    ["d", "<environment>"],                                   // production, preview-42
    ["a", "30617:<repo owner pubkey>:<repo d>"],              // the repository it deploys
    ["c", "<commit id>"],                                      // the commit the image was built from
    ["a", "30432:<provider pubkey>:<listing name>"],           // the Listing the lease was bought on
    ["p", "<provider pubkey>", "primary"],                     // the provider serving the lease
    ["p", "<standby provider pubkey>", "standby"],             // one per Warm Standby, if any
    ["x", "<workload id hex>", "workload"],
    ["x", "<image digest hex>", "image"],
    ["r", "https://<base32 workload id>.gw.example.com"],      // optional: where people reach it
    ["expiration", "<lease expires_at>"],
    ["L", "toon.network"]
  ],
  "content": ""
}
```

- **`d`** is the environment's name, chosen by the tenant. One environment of one repository
  is one Deployment, so `d` SHOULD be unique per repository among the tenant's Deployments.
- **`a` (repository)**: exactly one, the NIP-34 coordinate of the repository.
- **`c`**: exactly one, the full commit id the image was built from, as the `c` tag of a NIP-34
  repository state and of NIP-C1.
- **`a` (Listing)**: exactly one, the Listing the primary's lease was bought on.
- **`p`**: exactly one marked `primary`; one marked `standby` per other member of the Standby
  Set, in its order. A standalone lease has only the primary.
- **`x`**: exactly one marked `workload` (the 32-byte workload id, lowercase hex) and at most
  one marked `image` (the image digest, lowercase hex with no `sha256:` prefix, as TOON_Network
  §8.1 writes it). The marker is required: both values are 64 hex characters. One `#x` filter
  still finds a Deployment by either.
- **`r`**: optional, an `https` URL people open the workload at, usually its Workload Gateway
  name. Readers draw it as a link and fetch nothing from it.
- **`expiration`**: required, a NIP-40 expiration no later than the lease's `expires_at` as the
  Spawn or last Extension returned it.
- **`L`**: `toon.network`, as on every TOON Compute event.
- **`content`**: empty, or a short note in plain text for people.

## Keeping it true

- **Re-sign on every Extension** with the new `expires_at` as `expiration`. A lease that is not
  extended ends; its Deployment expires with it and relays stop serving it.
- **Re-sign after a Takeover** naming the winner as `primary`, the Listing its lease was bought
  on, and the rest of the set as `standby`.
- **Re-sign on a new image or commit**: same `d`, new `c` and `x` image.
- **To take it down early**, publish a NIP-09 deletion naming its address, or let it expire.

## Reading it

A reader states only what public events show.

- **Running on a live provider until `<expiration>`**: the Deployment is served and the
  `primary` provider has an unexpired Liveness (10433).
- **Provider down**: the Deployment is served and the primary has no unexpired Liveness.
- **Evicted**: an Eviction Notice (4433) with `x` = the workload id, signed by a provider the
  Deployment names in `p`.
- **Taken over**: a Takeover (30433) with `d` = the workload id, signed by a `standby` the
  Deployment names, whose content's `primary` is the Deployment's `primary`. A Takeover from any
  other key is ignored.
- **Whose word it is**: the signer's. A reader that shows a Deployment *on a repository* shows it
  only when the signer is the repository's owner or a maintainer named in the owner's latest
  30617 (relay ADR 0002); elsewhere it is the signer's claim.

## Relation to other events

- A Deployment carries no price, no capacity and nothing from the lease's `status`: those stay
  with the Listing, the Liveness and the sealed route.
- It is not a NIP-C1 Job Result. CI builds the image; a Deployment says where it runs.
- It is not a NIP-89 handler (31990) or a NIP-82 app (32267): it names a running workload, not
  software to install.
