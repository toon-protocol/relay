# Rust relay container (#192): the image the Rust rebuild (#185) is gated on.
#
# A drop-in for the TypeScript relay's image it replaced, as far as a stack can
# tell: the same ports, the same /data volume, the same healthcheck through
# busybox `wget`, and the same user id (1000, which was `node` in that image),
# so it can read and write the files already in a volume. It is one static
# binary on Alpine.
#
# Published as ghcr.io/toon-protocol/relay by publish-relay-image.yml, which
# moves the `:release` tag deploy/docker-compose.yml pulls by default (#205).
# CI builds it too and runs the conformance suite against it.
#
# Build from repo root:
#   docker build -f crates/relay/Dockerfile -t relay-rust:ci .
#
# The `rust:` tag below is rust-toolchain.toml's channel. A `FROM` cannot read
# that file, so the version is repeated here and deploy/rust-workspace.test.ts
# fails the build if the two drift.

# ── Stage 1a: chef ───────────────────────────────────────────
# cargo-chef, as the connector's image uses it: third-party dependencies are
# compiled in a layer keyed only on the manifests and the lockfile, so a
# source-only commit reuses it.
FROM rust:1.98.0-alpine3.22 AS chef

# musl-dev: libc headers for linking; the C sources of secp256k1 and of the
# bundled SQLite build with the gcc the base image already carries.
RUN apk add --no-cache musl-dev
RUN cargo install cargo-chef --locked --version 0.1.78

WORKDIR /workspace

# ── Stage 1b: planner ────────────────────────────────────────
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 1c: builder ────────────────────────────────────────
FROM chef AS builder

COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --release --locked -p relay --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# The release handle (YYYY.MM.DD.N), reported as the version by /health and the
# Relay Information Document. Declared here, after the dependency layer, so a
# new handle rebuilds only the relay crate. Empty means the crate version.
# (A build ARG is in the environment of every RUN after it, so cargo sees it.)
ARG TOON_RELEASE_HANDLE=

# The Alpine target is musl with a static C runtime, so this is one static
# binary with nothing to install beside it.
RUN cargo build --release --locked -p relay

# ── Stage 2: Production ──────────────────────────────────────
FROM alpine:3.22

COPY --from=builder /workspace/target/release/relay /usr/local/bin/relay

# The defaults the TypeScript image had, name for name.
ENV TOON_BLS_PORT=3100
ENV TOON_RELAY_PORT=7100
ENV TOON_DATA_DIR=/data

# 3100 = paid-write HTTP store (POST /write, GET /health); 7100 = free Nostr WS reads
EXPOSE 3100 7100

VOLUME /data

# Run unprivileged, as uid/gid 1000: the id the TypeScript image's `node` user
# had, and so the owner of every file already in a deployed /data volume.
RUN addgroup -g 1000 relay \
    && adduser -D -u 1000 -G relay relay \
    && mkdir -p /data && chown relay:relay /data
USER relay

# Compose healthcheck target — GET /health on the BLS port. 127.0.0.1, not
# localhost (relay#94): the write listener's host default is 0.0.0.0
# (IPv4-only), but "localhost" inside the container can resolve to ::1, which
# nothing is listening on. `wget` is busybox's, part of the Alpine base.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -q --spider "http://127.0.0.1:${TOON_BLS_PORT:-3100}/health" || exit 1

CMD ["relay"]
