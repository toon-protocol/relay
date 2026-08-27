# =============================================================================
# relay-connector — the published TOON connector with THIS repo's
# connector.toml baked in.
# =============================================================================
# The payment proxy that runs in front of the relay. The relay app image is
# built and published separately (packages/relay/Dockerfile →
# ghcr.io/toon-protocol/relay); this image is ONLY the connector plus the
# route config it was validated against.
#
# Config is BAKED (COPYed), not bind-mounted, and that is the point: adopting
# a newer connector and changing the config it reads are the same reviewed
# commit, so a build can never reach a box ahead of the config it needs.
#
# ── Bumping the pin ──────────────────────────────────────────────────────────
# This ARG default is the pin of record. The publish workflow builds this file
# WITHOUT overriding it, so whatever ships to GHCR is whatever is written
# here — there is no second literal anywhere to drift out of sync with it.
#
# Pin an exact `rust-sha-` build, never the floating `rust-main` or the
# fleet's `rust-release` pointer. The config parser is `deny_unknown_fields`
# and startup is fail-closed, so a schema drift under you is a refuse-to-start
# rather than a degraded run — which is the behaviour you want, but it means
# an accidental bump is an outage.
#
# `rust-sha-6ea6009` is the first pin this bundle has shipped that speaks the
# `[node]` self-description (connector ADR 0050) and states a verified payment
# to the app on delivery (connector ADR 0040) — see connector.toml's `[node]`
# section and the relay's own write handler.
ARG CONNECTOR_TAG=rust-sha-6ea6009
FROM ghcr.io/toon-protocol/connector:${CONNECTOR_TAG}

# The image's own CMD already names this path (`CMD ["/app/config/connector.toml"]`),
# so the entrypoint needs no argument and there is no CONFIG_FILE to set.
COPY connector.toml /app/config/connector.toml

# Entrypoint and CMD are inherited from the connector image, running as uid
# 10001. The client edge is :3000 — POST /ilp, GET /ilp, GET /ilp/btp,
# GET /ilp/identity and /metrics all share it.
#
# Two things this image deliberately cannot bake, both supplied by the compose
# file next to it:
#   - the signer and settlement KEY FILES, mounted at /app/data/*.key
#   - the claim journal at /app/state, which must be a volume
