# Relay container (issue #26)
#
# The relay is a plain HTTP/WebSocket app behind an external terminator — it
# contains no ILP/connector/settlement logic. Entrypoint is the `relay` CLI
# (dist/cli.js):
#   * POST /write on TOON_BLS_PORT (default 3100) — body { "event": <NostrEvent> }
#   * GET  /health on TOON_BLS_PORT — compose healthcheck
#   * free NIP-01 WS reads on TOON_RELAY_PORT (default 7100)
#
# Published as ghcr.io/toon-protocol/relay:latest (the connector's
# `app-behind-terminator` compose points RELAY_IMAGE here).
#
# Build from repo root:
#   docker build -f packages/relay/Dockerfile -t ghcr.io/toon-protocol/relay:latest .

# ── Stage 1: Builder ─────────────────────────────────────────
FROM node:20-alpine AS builder

# pnpm via corepack
RUN corepack enable && corepack prepare pnpm@8.15.0 --activate

# Native build deps for better-sqlite3
RUN apk add --no-cache python3 py3-setuptools make g++

WORKDIR /app

# Workspace root files for dependency resolution
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.json .npmrc ./

# Workspace package manifests (both packages — the workspace resolves as a unit)
COPY packages/relay/package.json packages/relay/
COPY packages/bls/package.json packages/bls/

# Install all deps (incl. dev) for the build. --no-frozen-lockfile mirrors CI
# (release.yml / ci.yml): an upstream tarball was re-published in place, so a
# strictly frozen lockfile install is brittle. See .changeset/fix-ci-tarball-integrity.md
RUN pnpm install --no-frozen-lockfile

# Source + build config. The relay has no runtime dependency on the other
# workspace package, so the relay alone is sufficient to build.
COPY packages/relay/src/ packages/relay/src/
COPY packages/relay/tsconfig.json packages/relay/tsup.config.ts packages/relay/

# Build the relay (produces dist/cli.js — the `relay` bin)
RUN pnpm --filter @toon-protocol/relay build

# Isolated production install. We deliberately do NOT use `pnpm deploy` here:
# it bundles the workspace-root dependencies (@ardrive/turbo-sdk, arweave — and
# their large web3 transitive tree) into the deployment even though the relay
# does not use them. Installing relay's own manifest in a clean dir pulls ONLY
# its real prod closure (hono, better-sqlite3, nostr-tools, @toon-format/toon, ws).
RUN mkdir /prod \
    && cp packages/relay/package.json /prod/package.json \
    && cp -r packages/relay/dist /prod/dist
WORKDIR /prod
RUN npm install --omit=dev --no-package-lock --no-audit --no-fund

# Strip better-sqlite3 build artifacts (source + gyp not needed at runtime)
RUN rm -rf node_modules/better-sqlite3/deps \
           node_modules/better-sqlite3/src \
           node_modules/better-sqlite3/binding.gyp

# ── Stage 2: Production ──────────────────────────────────────
FROM node:20-alpine

# Runtime dep for the better-sqlite3 native addon
RUN apk add --no-cache libstdc++

WORKDIR /app

COPY --from=builder /prod/dist ./dist
COPY --from=builder /prod/node_modules ./node_modules
COPY --from=builder /prod/package.json ./

# Defaults — the connector compose supplies identity (NOSTR_SECRET_KEY) and may
# override ports; the image runs as a standalone read/write relay out of the box.
ENV NODE_ENV=production
ENV TOON_BLS_PORT=3100
ENV TOON_RELAY_PORT=7100
ENV TOON_DATA_DIR=/data

# 3100 = paid-write HTTP store (POST /write, GET /health); 7100 = free Nostr WS reads
EXPOSE 3100 7100

VOLUME /data

# Run unprivileged; /data must be writable by the runtime user
RUN mkdir -p /data && chown node:node /data
USER node

# Compose healthcheck target — GET /health on the BLS port
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -q --spider "http://localhost:${TOON_BLS_PORT:-3100}/health" || exit 1

# Entrypoint = the relay CLI
CMD ["node", "dist/cli.js"]
