<!-- Moved from toon-protocol/rig docs/adr/0003-bounded-job-log-uploads.md (https://github.com/toon-protocol/rig/blob/main/docs/adr/0003-bounded-job-log-uploads.md), where it is ADR-0003. Content unchanged apart from the number and cross-references. -->

# ADR-0004: Job log uploads are bounded

## Status

Accepted (2026-09-17).

## Context

The coordinator uploads each job's complete log to the TOON store and puts
the resulting gateway URL in the Job Result's `logs` tag. Nothing caps it.
A job that leaves `set -x` on inside a loop writes an unbounded blob to
permanent, paid storage; the affordability estimate budgets a "generous
per-upload envelope" of 64 KiB per log and the actual upload can exceed
that by orders of magnitude, so the coordinator can be committed to a
write it did not price.

Nothing in the repo reads a log back. That changes with ADR-0003 (rig ADR-0002): the job
page in rig-web renders the job log once the run concludes, which turns an
unbounded blob into a browser that hangs on a URL a viewer clicked.

## Decision

The coordinator bounds what it uploads. A log over the cap is uploaded as
its head and its tail with an explicit marker naming the omitted byte
count in between; it never uploads more than the cap. rig-web additionally
stops reading at a client-side ceiling, so a blob written by some other
coordinator cannot hang the page.

## Consequences

- **The `logs` tag stops meaning "the complete log"** and starts meaning
  "the job log, bounded". This is why the decision is taken now rather
  than when it starts hurting: `logs` URLs are permanent, and once a
  population of them exists promising the whole log, narrowing what the
  tag means is a silent change to records already published.
- **The middle of a very long log is unrecoverable.** Head and tail keep
  the setup and the failure, which is what gets read; a job whose
  interesting output is in the middle of hundreds of megabytes has to
  publish it as an artifact instead, which is already the right tool.
- **The per-run cost estimate becomes honest.** With a cap, the per-upload
  envelope in `estimateRunCost` is an actual bound rather than a guess.
