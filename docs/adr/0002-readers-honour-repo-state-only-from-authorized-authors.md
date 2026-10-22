# Readers honour repo state only from authorized authors

**Status:** Accepted (owner decision, 2026-10-22, #276). Recorded in #277. Carries over [`toon-protocol/rig` ADR 0005](https://github.com/toon-protocol/rig/blob/main/docs/adr/0005-readers-honor-repo-state-from-authorized-authors.md) (accepted 2026-10-16, rig#225, built in rig#226); the rule is unchanged. Not built here yet: the `/repo` UI is.

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a `/repo` page that resolves a branch from a kind:30618 signed by someone who is neither the repo owner nor in the `maintainers` tag of the owner's own latest kind:30617.

**A reader takes a repo's refs only from the newest kind:30618 signed by an authorized author.** This is a reader policy; it adds no wire shapes.

## Context

rig's readers once accepted a repo state only when the owner signed it, so a declared maintainer could not move a branch. rig#225 widened that to authorized authors. The relay's `/repo` UI ([0001](0001-app-uis-live-on-the-relay.md)) replaces rig-web, so it must resolve the same refs rig's CLI does.

## Decision

1. **Authorized authors** are the owner plus the pubkeys in the `maintainers` tag of the owner's own latest kind:30617. Maintainers listed in another maintainer's announcement carry no authority, so authority never spreads beyond what the owner granted. Removing a pubkey from the owner's tag revokes it for all earlier and later events alike.
2. **The honoured repo state** is the newest kind:30618 signed by an authorized author. A `created_at` tie goes to the lowest event id. A stranger's repo state is ignored, however new.
3. **Status events** (issues, patches) are honoured from the same set plus the target's own author.
4. The rule is a pure function with no I/O, so every reader resolves identical refs.

## Consequences

- A maintainer can move refs, not only set status.
- Two maintainers can publish states that drop each other's commits; the replaceable event has no merge. Guarding against that is a push-side concern.
- Revocation is by the owner's current announcement only; there is no history.
