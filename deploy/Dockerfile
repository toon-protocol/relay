# =============================================================================
# relay-connector — the published TOON connector with THIS repo's connector.toml
# baked in. It is the payment proxy ("nginx for payments") that runs in front of
# the oblivious relay. The relay app image is built/published separately
# (packages/relay/Dockerfile → ghcr.io/toon-protocol/relay); this image is ONLY
# the connector + the relay route config.
#
# Config is BAKED (COPYed), not bind-mounted, so the published image is a
# self-contained, pinned deployable. TLS is terminated by the deployment
# environment (no Caddy here).
#
# ── This is now the RUST connector (connector#755) ───────────────────────────
# The base was `ghcr.io/toon-protocol/connector:3.28.0`, the TypeScript node,
# reading a `connector.yaml`. That node is RETIRED: the TOON devnet cut over to
# the Rust connector on 2026-08-04 and both TypeScript containers were stopped,
# so `3.28.0` is a tag with nothing behind it that anyone still runs. This image
# now bakes `connector.toml` — a different config LANGUAGE, not just a different
# file name — into the same path the Rust image already reads by default.
#
# Two things to know before bumping this:
#
#   * The `connector` package carries BOTH lineages under one name. TypeScript
#     releases are plain semver (`3.28.0`); Rust builds are `rust-sha-<short>`
#     and `rust-main`. A tag without the `rust-` prefix is the retired node, and
#     it will not read this file.
#   * Pinned to an exact `rust-sha-`, never the floating `rust-main`, so the
#     config schema is frozen against a known connector. The parser is
#     `deny_unknown_fields` and startup is fail-closed, so a schema drift is a
#     refuse-to-start rather than a degraded run — which is the behaviour you
#     want, but it means an undeliberate bump is an outage.
#
# This ARG default is the pin's single source of truth: the publish workflow
# (.github/workflows/publish-relay-connector-image.yml) builds this file
# without overriding CONNECTOR_TAG, so whatever ships to GHCR is whatever is
# written here. `rust-sha-440eab7` is the fleet's pin of record chosen in
# toon-protocol/connector#848 — the earliest Rust build at or after the
# announce-identity fixes (connector#833/#839) — and is also the first pin this
# bundle has shipped that understands `[announce]` (connector#784); see
# connector.toml's "self-announce" comment.
ARG CONNECTOR_TAG=rust-sha-440eab7
FROM ghcr.io/toon-protocol/connector:${CONNECTOR_TAG}

# Bake the relay route config at the path the image's own CMD already names
# (`CMD ["/app/config/connector.toml"]`), so the entrypoint needs no argument
# and there is no CONFIG_FILE environment variable to set. There is no env layer
# in the Rust connector at all: every value comes from this one file.
COPY connector.toml /app/config/connector.toml

# Entrypoint and CMD are inherited from the connector image
# (`/usr/local/bin/connector /app/config/connector.toml`), running as uid 10001.
#
# The client edge is :3000 — POST /ilp, GET /ilp/btp, GET /ilp/identity and
# /metrics all share it. There is no separate health (:8080) or admin (:8081)
# port; those were the TypeScript node's and do not exist here.
#
# Two things this image deliberately CANNOT bake, both supplied by the compose
# file next to it:
#   - the signer and settlement KEY FILES, mounted at /app/data/*.key
#   - the claim journal at /app/state, which must be a volume — without it a
#     restart resets every channel's replay watermark and re-accepts claims the
#     payer already spent (connector#605)
