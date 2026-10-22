<!-- Moved from toon-protocol/rig docs/adr/0002-live-log-tail-as-a-nip-c1-extension-kind.md (https://github.com/toon-protocol/rig/blob/main/docs/adr/0002-live-log-tail-as-a-nip-c1-extension-kind.md), where it is ADR-0002. Content unchanged apart from the number and links. -->

# ADR-0003: The live log tail is one addressable NIP-C1 extension kind per run

## Status

Accepted (2026-09-17).

## Context

rig-web is a browser-only SPA: one relay WebSocket and Arweave gateway
HTTP, "no backend, no accounts, no servers". `rig ci serve` is the mirror
image — an outbound relay subscriber with no inbound surface, typically a
laptop behind NAT. Between those two facts, the relay is the only channel
that can carry anything from a running coordinator to a viewer.

Today nothing about a job reaches a viewer until the run concludes: the
Job Result (kind 9841) with its 4 KiB tail and its `logs` URL is published
after the runner returns. The `Runner.onLog(jobId, chunk)` seam exists and
`ActRunner` faithfully calls it, but no coordinator passes a sink — the
streaming seam was built and wired to nothing.

NIP-C1 defines no live-log event. It is also not rig's specification: it
is published by the ngit-ci project and was vendored into `docs/nips/` by
#125. Every relay event costs a flat `feePerEvent` regardless of event
size, and the coordinator's affordability check exists so that it "must
never start a run it may not be able to finish publishing".

## Decision

1. **One addressable event per run, not per job.** Its `d` is the workflow
   run id; its content carries a tail for each unfinished job plus the
   runner channel. Per-job events would multiply the update count by the
   job count, and since the event fee is flat, folding several tails into
   one event costs nothing extra.

2. **A rolling tail, not a transcript.** Each publication replaces the
   last. A viewer arriving mid-run sees the current tail immediately; no
   one can scroll back past it. Cost is therefore a function of run
   duration alone, never of how much output a repo's build produces.

3. **Fixed cadence, budgeted before the run starts.** The tail is
   republished on a fixed interval, and `canAfford` counts the worst case
   — run timeout divided by that interval — into the run estimate. A
   coordinator that cannot afford the whole stream does not start the run.

4. **Generous bytes.** Because the fee ignores size, each job gets a tail
   far larger than the 4 KiB that rides in a Job Result, divided down when
   enough jobs are live to threaten a relay-safe event ceiling.

5. **One final replacement, then expiry.** When the run concludes the
   event is replaced once more with the closing tail, then left to expire
   under NIP-40 like every other addressable in NIP-C1.

6. **The tail is sliced from redacted bytes.** The coordinator redacts the
   accumulated log and then takes the tail, never the reverse. This is the
   same whole-buffer redaction the durable path already performs, so the
   existing guarantee — an injected value never leaves the process in the
   clear — holds unchanged, with no chunk-boundary case to reason about.

7. **Ship under a rig-chosen kind, propose upstream in parallel.** The
   extension is documented in `docs/nips/` as rig's, and offered to
   ngit-ci.

## Considered options

- **Browser to coordinator directly** (HTTP or SSE from the job page).
  Cheap and unbounded, but it requires `rig ci serve` to become reachable,
  advertise a URL, and serve CORS to an Arweave-hosted origin. A laptop
  coordinator cannot, so live logs would silently become a feature only
  hosted coordinators have — retracting the premise that a coordinator is
  just a process someone runs.
- **Incremental store uploads.** Permanent and paid per chunk, with
  minutes of latency. Not live in any useful sense.
- **A relay-side streaming extension.** Fastest and cheapest, and it ends
  rig-web's ability to work against any NIP-01 relay.
- **Folding the tails into the existing Workflow Progress event (39842).**
  No new kind, and the event is already replaced on a timer and already
  subscribed to. Rejected because NIP-C1 specifies that event's content as
  empty, and because every progress consumer would then pay to receive log
  bytes it did not ask for.
- **Append-only chunk events.** Gives scrollback, at a cost proportional
  to the repo's own build output — the wrong shape for a coordinator
  paying its own way — and, on an ephemeral kind, shows a late viewer
  nothing at all.

## Consequences

- **Squatting a kind number is recoverable here, and only here.** This
  event carries no durable data: it is replaced on a timer and expires
  within the hour. If ngit-ci assigns the number elsewhere, renumbering is
  a deploy, not a data migration, and the worst case is that old
  coordinators and new clients cannot see each other for one expiry
  window. The same shortcut would not be defensible for any kind whose
  history is retained.
- **Live is a strictly worse view than the record, by design.** Output
  that scrolls past between two publications is never visible live. It is
  in the job log a minute later, which is where anyone reading carefully
  should be.
- **A viewer cannot distinguish a stalled job from a stalled
  coordinator** beyond the expiry of the event. That is the price of
  replacing rather than appending.
- **The runner channel becomes load-bearing.** `ActRunner` drops any line
  that does not parse as act JSON before `onLog` sees it, so an image pull
  failure or an act crash would leave the pane empty through the exact
  failure a viewer is watching for. Unparseable output is therefore routed
  to the runner channel, which mixes cleanup notes with raw backend
  output. The final log path is untouched.
