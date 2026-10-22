<!-- Moved from toon-protocol/rig docs/specs/nip-c1.md (https://github.com/toon-protocol/rig/blob/main/docs/specs/nip-c1.md). Content unchanged apart from links. -->

NIP-C1
======

Nostr CI: CI Extension to NIP-34
--------------------------------

This NIP is published on Nostr at [nostrhub.io](https://nostrhub.io/naddr1qvzqqqrcvypzpgqgmmc409hm4xsdd74sf68a2uyf9pwel4g9mfdg8l5244t6x4jdqqrxu6ts943nzy6kdcz); this file is its source. The companion [NIP-guidance.md](https://github.com/toon-protocol/rig/blob/main/docs/specs/nip-c1-guidance.md) contains rationale, operational guidance, and query patterns.

`draft` `optional`

This NIP defines continuous-integration events for [NIP-34](https://nips.nostr.com/34) repositories.

| event | kind | NIP-01 class / replacement identity | publisher | lifetime |
| --- | ---: | --- | --- | --- |
| Coordinator Advertisement | 19843 | normal replaceable: `(kind, pubkey)` | coordinator | at most 30 minutes |
| Request-Readiness List | 19844 | normal replaceable: `(kind, pubkey)` | coordinator | at most 24 hours |
| Coordinator Repository Status | 39844 | addressable: `(kind, pubkey, d)` | coordinator | at most 24 hours |
| Service Request | 9843 | regular | requester | no protocol expiry |
| Service Stop | 9844 | regular | requester | no protocol expiry |
| Repository Secret Update | 29846 | ephemeral | maintainer | relay does not retain |
| Manual Trigger | 9840 | regular | maintainer | no protocol expiry |
| Job Result | 9841 | regular | compute provider | no protocol expiry |
| Workflow Result | 9842 | regular | coordinator | no protocol expiry |
| Workflow Progress | 39842 | addressable: `(kind, pubkey, d)` | coordinator | at most 30 minutes |

## Coordinator Advertisement

A coordinator advertises its current availability and capabilities with a normal replaceable `kind:19843` event:

```jsonc
{
  "kind": 19843,
  "content": "",
  "tags": [
    ["software", "ngit-ci", "<version>"],

    ["W", "act"],                  // supported runner family
    ["W", "nix"],
    ["R", "act:ubuntu-latest"],   // accepted selector for that family
    ["R", "act:ubuntu-24.04"],    // another selector in the same family
    ["R", "nix:x86_64-linux"],

    ["M", "<operator-selected|maintainer-request|open>"],
    ["X", "<automatic|request-required>"],
    ["B", "<not-required|out-of-band>"], // optional
    ["secrets-key", "nip44-v2", "<32-byte-xonly-secp256k1-pubkey>",
      "wss://secret-inbox-1.example", "wss://secret-inbox-2.example"], // optional
    ["expiration", "<unix-timestamp>"]
  ]
}
```

`content` MUST be empty and the event MUST omit `d`. It MUST contain exactly one `M`, one `X`, and one `expiration` tag, at most one `B` and one `secrets-key` tag, and at least one `W` and one `R` tag. Unknown policy values MUST NOT be interpreted as open, automatic, or free service.

Each `W` value names a supported runner family. Each `R` value is `<family>:<selector>` and is split at its first `:`; its family MUST appear in a `W` tag, and every advertised family MUST have at least one `R`. Repeating `W` and `R` is how one event advertises multiple families and selectors. Values are ASCII-case-folded and deduplicated. The result-only `runs_on` tag MUST NOT be used for advertised capabilities.

`M` describes admission: `operator-selected` is bounded by operator policy, `maintainer-request` permits a maintainer request to start admission, and `open` accepts repositories without either restriction. `X` is `automatic` when eligible triggers may run without a current request, or `request-required` otherwise. `B`, when present, is `not-required` or `out-of-band`.

When present, `secrets-key` advertises support for Repository Secret Updates. It is a NIP-44 recipient generated separately from the coordinator signing key. Every following value is a normalized `ws://` or `wss://` secret-inbox relay URL, and at least one MUST be present. A maintainer publishes the same signed Repository Secret Update to every listed relay; acceptance by any one is sufficient. Refreshing an Advertisement does not itself rotate this key, but a continuously running coordinator MUST advertise a new generation no later than 86,400 seconds after first advertising the current generation. It MAY rotate earlier and MUST rotate at startup. Recipient private keys exist only in the live coordinator process and MUST NOT be persisted. A retired generation MUST NOT become current again. Its private key MUST be destroyed no later than 60 seconds after every Advertisement naming it has expired; this bounded grace permits delivery of an ephemeral update created inside the advertised window. Absence means that the coordinator is not accepting secret updates.

The NIP-40 `expiration` MUST be later than `created_at` and no more than 30 minutes after it. A live coordinator replaces the Advertisement before expiry and whenever its advertised state or recipient generation changes.

## Request-Readiness List

A coordinator MAY advertise repositories for which it is ready to accept a Service Request with one public `kind:19844` standard list:

```jsonc
{
  "kind": 19844,
  "content": "",
  "tags": [
    ["a", "30617:<selected-maintainer-pubkey>:<repository-id>", "<relay-hint>"],
    ["p", "<selected-maintainer-pubkey>", "<relay-hint>"],
    ["expiration", "<unix-timestamp>"]
  ]
}
```

This event uses the public standard-list representation described by [NIP-51](https://nips.nostr.com/51). It is a normal replaceable event, not a generic custom list or deprecated kind `30001`. `content` MUST be empty, `d` MUST be absent, and private items are unsupported.

An `a` item covers exactly the referenced repository. A `p` item covers all repositories rooted at that pubkey; it does not propagate to other maintainers listed by that pubkey. Items have two fields or an optional relay-hint field, MUST NOT have a fourth marker, and MUST be deduplicated.

Inclusion means the coordinator is ready to act after an otherwise valid Service Request. The list is an optional, non-exhaustive discovery hint. It MUST contain exactly one `expiration`, later than `created_at` and no more than 86,400 seconds later. It is actionable only while the same pubkey has a live Coordinator Advertisement.

## Service Request and Stop

A requester asks a coordinator to provide or stop standing service for one repository perspective with an immutable event of the following shape:

```jsonc
{
  "kind": 9843, // 9843 requests service; 9844 stops service
  "content": "",
  "tags": [
    ["a", "30617:<selected-maintainer-pubkey>:<repository-id>", "<relay-hint>"],
    ["p", "<coordinator-pubkey>"]
  ]
}
```

For both kinds, `content` MUST be empty. The event MUST contain exactly one `a` tag and one `p` tag, and MUST NOT contain `d` or `expiration`. The `a` tag contains the exact repository address and MAY include one relay hint. Other tags MUST NOT change the event's meaning.

Neither event admits an otherwise unknown repository or grants repository authority to its author. The default policy accepts a Request signed by a current maintainer of the selected perspective; an operator MAY explicitly accept other requester pubkeys.

Controls are totally ordered: a greater `created_at` is later; at equal timestamps, the lexicographically lower event id is later. A Stop from a current maintainer closes every earlier Request for the perspective. A Stop from any other author closes only that author's earlier Requests, including when that author is an explicitly accepted requester or a removed maintainer. A coordinator selects the newest remaining Request accepted by its local policy. A later accepted Request makes service eligible again. Current authorization is evaluated when the coordinator decides whether to run work, so a removed maintainer's Stop loses repository-wide effect but retains its author-local effect.

A request-required coordinator MUST remain closed when no accepted, unstopped Service Request is present in the history it evaluates. A kind-9840 Manual Trigger is a separate one-shot request and does not require a standing Service Request.

## Repository Secret Update

A maintainer provisions or removes CI secrets with an ephemeral kind-29846 event:

```jsonc
{
  "kind": 29846,
  "content": "<nip44-v2-ciphertext>",
  "tags": [
    ["a", "30617:<maintainer-pubkey>:<repository-id>", "<relay-hint>"],
    ["p", "<coordinator-pubkey>"],
    ["e", "<coordinator-advertisement-id>", "<relay-hint>", "secrets-key"],
    ["sender", "<fresh-encryption-pubkey>"],
    ["recipient", "<advertised-secrets-key>"],
    ["encryption", "nip44-v2"]
  ]
}
```

The event is signed by the maintainer identity. NIP-44 encryption uses a fresh sender key generated only for this submission; its private key SHOULD be destroyed after publication. The maintainer signature authenticates the tags and ciphertext together without using the maintainer's long-term identity key for decryption. The encrypted payload binds the ciphertext to that signer and the event's ordering timestamp.

The event MUST contain exactly one tag of each shown type and no other tags. The `a` coordinate MUST be kind 30617 and its pubkey MUST equal the event signer. `p` names the coordinator. The `e` tag MUST use the `secrets-key` marker and reference an authentic Advertisement from that coordinator. `recipient` MUST equal the key advertised by that exact event and `sender` MUST be the key used for NIP-44 encryption. The update timestamp MUST satisfy `advertisement.created_at <= update.created_at < advertisement.expiration`. The referenced Advertisement need not still be the current NIP-01 replacement, but its recipient private key must still be within its retained expiry window.

The update MUST be submitted to at least one secret-inbox relay listed by the referenced Advertisement and SHOULD be submitted to all of them. The `a` relay hint locates the maintainer's repository announcement; the `e` relay hint locates the coordinator Advertisement. Neither is the secret-update submission destination.

The decrypted UTF-8 plaintext is one atomic update:

```jsonc
{
  "author": "<maintainer-pubkey>",
  "created_at": 1730000000,
  "set": {"DEPLOY_TOKEN": "value"},
  "remove": ["OLD_TOKEN"]
}
```

The plaintext MUST contain exactly `author`, `created_at`, `set`, and `remove`. `author` MUST equal the outer event pubkey and `created_at` MUST equal the outer event timestamp. A receiver MUST reject either mismatch before applying any operation. These bindings prevent another maintainer from re-signing observed ciphertext into their authority and prevent old ciphertext from being assigned a newer ordering position. Repository, coordinator, Advertisement, sender, and recipient remain authenticated by the outer signature and are not repeated in the plaintext.

At least one of `set` and `remove` MUST be non-empty. A name MUST NOT occur in both. Names MUST match `[A-Z_][A-Z0-9_]*`, MUST NOT claim a coordinator-reserved runtime variable, and an update MUST contain no more than 100 names. Each set value is non-empty UTF-8 and no larger than 16,384 bytes. Decrypted plaintext is limited to 65,535 bytes and ciphertext to 100 KiB.

For one `(author-rooted repository, name)`, updates are totally ordered: a greater `created_at` is later; at equal timestamps the lexicographically lower event id is later. A remove records an ordering tombstone until no older Advertisement-bound event can still be accepted, so an older set cannot restore a value. All changes from one event are persisted and exposed to jobs atomically.

The coordinator MUST NOT persist plaintext in its general Nostr event cache. It persists only the accepted operation frontier, encrypted with an authenticated local storage key separate from its signing key. Exact public Advertisement bindings are retained with that frontier, but recipient private keys live only in memory. A restart therefore publishes a fresh recipient and cannot decrypt updates addressed to an earlier process. Losing the encrypted frontier or its local key requires maintainers to submit the values again.

The name `WORKFLOW_SECRETS_DECRYPTION_BUNKER` is reserved and is never injected into jobs. Its submitted value binds the repository scope to a maintainer-controlled NIP-46 remote signer and MAY be either a fresh `bunker://` pairing URL or an established connection encoded as interoperable `nbunksec`. For a pairing URL the coordinator MUST generate a fresh dedicated client key and complete the initial `get_public_key` request. For `nbunksec` it MUST use the embedded client key. It MUST NOT reuse the coordinator identity key for either form.

Before accepting the update, the coordinator MUST convert the working connection to `nbunksec` and remove the one-time pairing secret. Only that sanitized `nbunksec` may enter the durable secret frontier, so the binding can be replayed after restart without attempting to consume the pairing URL again. The stored credential contains the remote-signer pubkey, dedicated client secret key, and one or more relays. Bech32 encoding does not encrypt those fields, so it MUST be handled as a secret. The coordinator MUST request only `get_public_key` and `nip44_decrypt`; maintainers SHOULD dedicate the connection to this coordinator and restrict it to those methods when their remote signer supports per-connection policy. Failed pairing or connection validation rejects the whole update and MUST NOT change the stored binding or its public inventory receipt.

While a scope carries this binding, the coordinator MUST store every other accepted value for the scope *sealed*: NIP-44-encrypted from a single-use ephemeral key to a per-scope sealing public key whose private half the coordinator retains only *wrapped* — NIP-44-encrypted to the bunker's user public key. The retained state therefore cannot be opened with any key the coordinator holds, while sealing new values needs no bunker interaction. The binding itself stays coordinator-readable. At job time the coordinator requests one NIP-46 `nip44_decrypt` of the wrapped scope key per run, decrypts the released values locally, keeps scope key and plaintext in memory only for that run, and MUST conclude the run `startup_failure` instead of executing when an authorized sealed value cannot be unlocked. Values accepted before the binding MUST be sealed once the bunker's user public key is known; an update for a bound scope MUST be rejected — not stored plaintext — while that key cannot be obtained. A binding whose bunker presents a different user public key MUST receive a fresh scope sealing keypair.

Receipt is not permission to use a value. The author-rooted coordinate is the secret owner used by job-time authorization. Every release MUST re-evaluate the trigger author, signed repository reference, selected coordinate, current acting maintainer, maintainership excluding invited maintainers, and repository-relay confidence. A selected former-maintainer coordinate MAY route through a valid active `M` path, but its signer is not thereby current and cannot contribute a secret scope. Removing the provisioning maintainer therefore withholds its stored values without changing the durable operation frontier.

Maintainers publish updates to the union of relays announced by their repository perspective and its confirmed maintainership, and SHOULD also use an index relay carrying the referenced Advertisement. A relay acknowledgement proves only relay receipt; this version does not define a separate per-update coordinator receipt. Coordinator Repository Status provides the durable, value-free inventory described below so maintainers can audit the effective names, sources, and update times. An acting coordinator MUST attempt to publish a replacement status immediately after its effective secret inventory changes; the matching source and update time are the maintainer's public confirmation that the update was accepted.

## Coordinator Repository Status

A coordinator reports the effective service it is providing for one selected repository root with `kind:39844`:

```jsonc
{
  "kind": 39844,
  "content": "",
  "tags": [
    ["d", "30617:<selected-maintainer-pubkey>:<repository-id>"],
    ["a", "30617:<selected-maintainer-pubkey>:<repository-id>", "<relay-hint>"],
    ["a", "30617:<maintainer-pubkey>:<repository-id>", "<relay-hint>"],
    ["s", "acting"],

    ["W", "act"],                         // effective runner family
    ["workflow-path", ".ngit/act/workflows/*.yml"],
    ["R", "act:ubuntu-latest"],           // effective runner selector
    ["secret", "<name>", "<origin-maintainer-pubkey>", "<created-at>"],
    ["secret", "<locally-provisioned-name>"],
    ["expiration", "<unix-timestamp>"]
  ]
}
```

`content` MUST be empty. There MUST be exactly one `d`, one `s`, and one `expiration`. The `d` value is the selected root's exact repository address. The first `a` MUST repeat that address; remaining `a` tags list every other reference in its current maintainer closure once, sorted and deduplicated. Relay hints SHOULD be included when known.

`s` is `acting` while the coordinator accepts eligible triggers for this repository. Before making that claim, the coordinator SHOULD complete a filtered retained-event request against at least one relay currently announced by the repository's maintainer closure. `workflow-path`, `W`, and `R` describe its effective capabilities; `W` and `R` follow the Coordinator Advertisement rules. `runs_on` MUST NOT be used.

Each effective secret is disclosed by name, but never by value. A secret accepted from kind 29846 uses `["secret","<name>","<origin-maintainer-pubkey>","<created-at>"]`, where `<created-at>` is the accepted update's decimal `created_at`. A value held only as bunker-sealed ciphertext appends a literal `sealed` marker: `["secret","<name>","<origin-maintainer-pubkey>","<created-at>","sealed"]`. An operator-provided secret uses `["secret","<name>"]`. If an operator value shadows a Nostr value, only the operator form is published. No `secret` tag means the coordinator has no effective secrets for this selected repository perspective.

When the coordinator stops acting for the repository, it SHOULD publish a kind-5 NIP-09 deletion request for the kind-39844 coordinate to the repository relays. When the previous status event id is known, the deletion request SHOULD also include an `e` tag naming it.

The `expiration` MUST be later than `created_at` and no more than 86,400 seconds later. The event is actionable only while its signer has a live Coordinator Advertisement.

## Common tags

Every event in this NIP identifies the repository, commit, workflow file and normalized trigger:

```jsonc
[
  ["a", "30617:<repo-owner-pubkey>:<repo-id>"],
  ["a", "30617:<maintainer-pubkey>:<repo-id>"], // optional, for each known maintainer announcement for the same repository
  ["c", "<commit-id>"],
  ["c", "<annotated-tag-id>"], // when triggered by an annotated tag
  ["w", "<workflow-file-path>", "<sha256-of-workflow-file-content>"],
  ["o", "<push|pull_request|schedule|manual>"], // normalized reason this attempt was run
]
```

The first `c` tag is always the commit id the workflow ran against. When the trigger was an annotated tag, a second `c` tag with the tag object id SHOULD be included so a single `#c` filter finds the event whether the client holds the commit id or the tag id.

Plus the trigger context. For `push` triggers:

```jsonc
["r", "refs/<heads|tags>/<branch-or-tag-name>"]
```

For `pull_request` triggers, [NIP-22](https://nips.nostr.com/22) style tags pointing to the PR (kind `1618`) as root and the PR or PR Update (kind `1619`) event that supplied the commit as parent, and no Git-ref `r` tag:

```jsonc
[
  ["E", "<pull-request-event-id>"],
  ["K", "1618"],
  ["P", "<pull-request-author>"],

  ["e", "<pull-request-or-update-event-id>"],
  ["k", "<1618|1619>"],
  ["p", "<pull-request-or-update-author>"],
]
```

## Manual Trigger

A repository maintainer MAY request a manual workflow run by publishing a `kind:9840` CI Manual Trigger event. The event is signed by the requesting maintainer, has empty `content`, tags the intended coordinator with a `p` tag, and includes the common tags **except** `o`; a Manual Trigger is always normalized as `manual`.

```jsonc
{
  "kind": 9840,
  "content": "",
  "tags": [
    ["p", "<coordinator-pubkey>"],
    ["a", "30617:<repo-owner-pubkey>:<repo-id>"],
    ["a", "30617:<maintainer-pubkey>:<repo-id>"], // optional additional maintainer repo refs
    ["c", "<commit-id>"],
    ["c", "<annotated-tag-id>"], // optional
    ["w", "<workflow-file-path>", "<sha256-of-workflow-file-content>"],
    ["r", "refs/heads/<branch-name>"] // optional, as context for workflows that use it
  ]
}
```

The first `c` tag identifies the commit to run. A requester MAY add `c` tags for annotated tag objects that resolve to that commit. A coordinator MUST resolve every supplied `c` object and ignore the request unless all supplied object ids peel to the same commit id. It MUST run the workflow and publish the peeled commit as the first `c` tag in resulting CI events; supplied annotated tag object ids are retained as additional `c` tags.

A Manual Trigger is an authorized replay of the exact workflow identified by its `w` tag. The selected file MUST exist at the resolved commit and its content SHA-256 MUST equal the requested value, but it need not declare `manual` in its `on:` clause. This permits maintainers to retry a push or pull-request workflow when no earlier result exists, for example because a coordinator or runner was unavailable. The resulting CI events use `["o", "manual"]` to identify the attempt as a manual replay.

For a pull-request run, the Manual Trigger instead carries the NIP-22 PR context tags specified above, **except** the lowercase `p` tag: on a Manual Trigger the only `p` tag is the coordinator address. It MUST NOT include an `r` tag. The coordinator uses the referenced PR and PR Update to determine the commit and repository scope; the `c` tag remains the requested commit.

The `a` tags identify repository announcements, not an authorization grant. They MUST all describe the same `<repo-id>`. A coordinator MUST resolve every `a` tag independently and MUST NOT run a workflow for a repository merely because it was named in the request:

- For a non-PR trigger, a named repository is eligible only when its resolved repository state contains the requested `c` commit.
- For a PR trigger, the eligible repositories are only those referenced by the resolved PR/PR Update; the coordinator MUST NOT widen the scope using other `a` tags on the request.
- The event author MUST be an authorized maintainer of each repository for which the coordinator schedules a run. A coordinator MUST reject unauthorized repository scopes.

Coordinators MUST verify the event signature, require exactly one `p` tag — which MUST name themselves — and ignore requests addressed to another coordinator. They SHOULD reject malformed, ambiguous, stale, or unauthorized requests without starting any jobs. Results and progress produced from an accepted request use the usual common tags with `["o", "manual"]` and MUST quote the request using a [NIP-18](https://nips.nostr.com/18) `q` tag:

```jsonc
["q", "<9840-request-id>", "<relay-url>", "<requester-pubkey>", "manual-trigger"]
```

This request-provenance quote is present on each Workflow Progress and Workflow Result event (and MAY be included on Job Results). Its `manual-trigger` marker distinguishes it from a Workflow Result's Job Result quotes.

## Job Result

A Job Result records the outcome of a single job within a workflow. The `content` is a small tail excerpt from the job's log output; the full log SHOULD be uploaded to a [Blossom](https://github.com/hzrd149/blossom) server and referenced with a `logs` tag.

A Job Result is signed by the compute provider that executed the job. Its `pubkey` identifies the provider making the execution claim, which MAY be the same key as the coordinator or a separate runner/provider key.

A Job Result MUST NOT quote a standing Service Request. The coordinator made that authorization decision; the provider asserts only the job execution.

```jsonc
{
  "kind": 9841,
  "content": "[log-tail omitted=<bytes>]\n<small tail of job log>",
  "tags": [
    // common and trigger context tags

    ["q", "39842:<coordinator-pubkey>:<workflow-run-id>", "<relay-url>"],
    ["job", "<job-id>"], // as declared in the workflow file
    ["name", "<human-readable job name>"], // optional
    ["conclusion", "<conclusion>"], // see conclusion values below
    ["logs", "<blossom-url-of-full-log-file>"],
    ["artifact", "<blossom-url>", "<filename>", "<name>"], // optional, for each file produced by the job
    ["output", "<name>", "<value>"], // optional public scalar output
    ["output-omitted", "<name>", "<missing|oversized|unresolved>"], // optional declared output without a value
    ["queued_at", "<unix-timestamp>"], // optional, when the job entered the queue
    ["started_at", "<unix-timestamp>"], // optional, when execution began
    ["exit_code", "<code>"], // optional
    ["runs_on", "<runner-label>", ...], // optional, e.g. "ubuntu-latest"
  ]
}
```

The `q` tag references the addressable Workflow Progress event for the run. Its address identifies both the coordinator that requested the job and the workflow run to which the result belongs. The relay hint identifies a relay to which the Progress event was published. A Job Result MUST NOT use `d` for this association.

Files produced by the job are listed as `artifact` tags, one tag per individual file — never an archive bundling several files. `<filename>` is the file's path within the artifact and `<name>` is the artifact name grouping related files (mirroring the `name` of GitHub-style upload-artifact actions; publishers whose execution backend stores a named artifact as a single zip, as GitHub does, SHOULD unpack it and publish the contained files individually). The Blossom URL embeds the file's sha256, so the tag is the compute provider's commitment to the exact bytes produced; downstream consumers — such as a later job that packages build outputs into a NIP-82 Software Release (kind `30063` / `3063`) — can fetch the file from any Blossom server and verify it.

`output` carries one public scalar produced by the job. Output names MUST be non-empty and unique across both `output` and `output-omitted` tags. An empty `output` value is valid and is distinct from an omitted output. Each output value MUST be valid UTF-8 no larger than 8 KiB, and the sum of the UTF-8 byte lengths of all output values in one Job Result MUST NOT exceed 64 KiB. The workflow format or an execution request determines which outputs are declared; their representation in a Job Result is independent of runner family and does not require this NIP to define that request.

When a publisher intends to report a declared output but cannot supply its value, it uses `output-omitted`: `missing` means that execution produced no value, `oversized` means that publishing it would exceed either size limit, and `unresolved` means that the execution backend could not reduce it to a value. A Job Result MUST NOT carry both tags for the same name. A consumer which requires an omitted output MUST treat it as unavailable and MUST NOT substitute an empty string.

Output values are public Nostr tag values. Publishers MUST NOT put secrets or other private values in them. This NIP defines no URL fallback or private form for an oversized output: a large result intended for transfer between jobs is a file and can use the existing `artifact` tag instead. These scalar outputs are distinct from artifact files and from build-system concepts such as Nix flake outputs.

## Workflow Result

A Workflow Result records the combined outcome of a workflow run, quoting the Job Result for each job that ran using a [NIP-18](https://nips.nostr.com/18) `q` tag with the job id appended as a marker. A request-gated run additionally quotes the exact standing Service Request selected at final runner handoff:

```jsonc
["q", "<9843-request-id>", "<relay-url>", "<requester-pubkey>", "service-request"]
```

The relay URL identifies the relay from which the coordinator received the Request. The pubkey hint is required because kind `9843` is a regular event. An automatic run MUST omit this quote. A manual replay uses the `manual-trigger` quote defined above instead.

```jsonc
{
  "kind": 9842,
  "content": "",
  "tags": [
    // common and trigger context tags

    ["r", "<workflow-run-id>"],
    ["conclusion", "<conclusion>"], // combined outcome of the jobs
    ["queued_at", "<unix-timestamp>"], // optional
    ["started_at", "<unix-timestamp>"], // optional
    ["q", "<9843-request-id>", "<relay-url>", "<requester-pubkey>", "service-request"], // request-gated runs
    ["q", "<job-result-event-id>", "<relay-url>", "<publisher-pubkey>", "<job-id>"], // for each job ran
  ]
}
```

Workflow Result `content` is always empty. Clients obtain log tails from the Job Results referenced by the `q` tags rather than receiving duplicated output.

When a Workflow Result quotes a Job Result signed by a different pubkey, the `q` tag records both the Job Result event id and the compute-provider pubkey. This makes the trust relationship explicit: the coordinator signing the Workflow Result is vouching that it scheduled or accepted that provider's Job Result for the workflow run, while the provider's signature makes the direct execution claim. Clients may decide whether to trust the coordinator, the compute provider, or require both.

Publishers SHOULD pass through the conclusion reported by their execution backend rather than recomputing it.

A publisher MAY run the same workflow for the same commit more than once, e.g. a manual re-run. Each attempt produces its own Workflow Result; clients SHOULD order attempts by `created_at` (or `started_at` when present) and treat a publisher's latest as current.

The Workflow Result's workflow run ID matches the `d` value of its Workflow Progress event. On push-triggered results, `r` tags beginning with `refs/` are Git refs; the other `r` value is the workflow run ID.

## Workflow Progress

A Workflow Progress event (kind `39842`) indicates that a workflow run is queued, executing, or recently concluded. It is an addressable version of the Workflow Result: it carries every tag specified for kind `9842` unless its applicability is overridden below. Its `d` replaces the Workflow Result's workflow-run `r`, its `content` is empty, its `conclusion` tag is present only when status is `concluded`, and it adds these tags:

```jsonc
[
  ["d", "<workflow-run-id>"], // unique per run attempt
  ["status", "<queued|in_progress|concluded>"],
  ["queue", "<rounds>"], // optional while queued; capacity rounds before start
  ["in-progress", "<job-id>", ...], // jobs currently executing
  ["conclusion", "<conclusion>"], // only when status is concluded
  ["expiration", "<unix-timestamp>"], // NIP-40
]
```

`q` tags referencing Job Results are added as each job completes, so a Workflow Progress event shows completed, in-progress and (by omission from both) pending jobs. The workflow run ID is generated when the run is queued and kept for the lifetime of that run attempt, so re-runs get their own marker. The [NIP-40](https://nips.nostr.com/40) `expiration` MUST be no more than 30 minutes after `created_at`; the publisher replaces the event as jobs start and finish and SHOULD renew it before it expires, so a crashed publisher's stale markers clear themselves.

A queued standing-service Progress event MUST omit the `service-request` quote because final authorization has not yet been selected. Its first `in_progress` event, every renewal, any `concluded` replacement, and the Workflow Result MUST carry the same frozen quote. A later Stop does not invalidate this historical provenance.

The optional `queue` tag is the publisher's best current coordinator-relative estimate of how many rounds of that coordinator's concurrent job capacity must drain before the workflow run starts, rounded up. `1` means the run is waiting for the next available capacity round; with concurrency `3`, the 10th queued workflow run publishes `4`. Clients MUST NOT interpret the value as an exact count of jobs ahead. The tag SHOULD be omitted once any job in the workflow starts.

Clients MUST NOT require a quoted Workflow Progress event to remain available after its expiration in order to accept a Job Result or Workflow Result. Publishers MAY skip runs for which a trusted result or an unexpired trusted progress marker already exists.

## Conclusion values

Aligned with the GitHub API `conclusion` field:

- `success` - completed successfully
- `failure` - completed with a failing step or job
- `neutral` - completed without a blocking outcome, e.g. a failing job allowed to fail (`continue-on-error` / `allow_failure`)
- `cancelled` - cancelled before completion
- `skipped` - intentionally not run
- `timed_out` - exceeded the publisher's time limit
- `startup_failure` - failed before execution, e.g. clone or workflow parse failure
