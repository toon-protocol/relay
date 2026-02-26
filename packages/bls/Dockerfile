# BLS (Business Logic Server) Standalone Container
# Multi-stage build for minimal image size
#
# Build from repo root:
#   docker build -f packages/bls/Dockerfile -t crosstown-bls .

# ── Stage 1: Builder ─────────────────────────────────────────
FROM node:20-alpine AS builder

# Install pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

# Install native build dependencies for better-sqlite3 and WebSocket modules
RUN apk add --no-cache python3 py3-setuptools make g++

WORKDIR /app

# Copy workspace root files for dependency resolution
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.json .npmrc ./

# Copy BLS package.json
COPY packages/bls/package.json packages/bls/

# Copy placeholder package.json files for other workspace packages
# (required for pnpm install --frozen-lockfile to resolve the full workspace)
COPY packages/core/package.json packages/core/
COPY packages/relay/package.json packages/relay/
COPY docker/package.json docker/

# Install all dependencies (including devDependencies for building)
RUN pnpm install --frozen-lockfile

# Copy workspace dependency source code (must be built before BLS)
COPY packages/core/src/ packages/core/src/
COPY packages/core/tsconfig.json packages/core/tsup.config.ts packages/core/
COPY packages/relay/src/ packages/relay/src/
COPY packages/relay/tsconfig.json packages/relay/tsup.config.ts packages/relay/

# Copy BLS source code
COPY packages/bls/src/ packages/bls/src/
COPY packages/bls/tsconfig.json packages/bls/tsup.config.ts packages/bls/

# Build workspace dependencies then BLS
RUN pnpm --filter @crosstown/core build && \
    pnpm --filter @crosstown/relay build && \
    pnpm --filter @crosstown/bls build

# Create production deployment (production deps only, no symlinks)
RUN pnpm --filter @crosstown/bls deploy --prod /prod

# Strip better-sqlite3 build artifacts (deps/ contains SQLite source, src/ contains C++ source)
RUN rm -rf /prod/node_modules/.pnpm/better-sqlite3@*/node_modules/better-sqlite3/deps \
           /prod/node_modules/.pnpm/better-sqlite3@*/node_modules/better-sqlite3/src \
           /prod/node_modules/.pnpm/better-sqlite3@*/node_modules/better-sqlite3/binding.gyp

# ── Stage 2: Production ──────────────────────────────────────
FROM node:20-alpine

# Install runtime dependency for better-sqlite3 native addon
RUN apk add --no-cache libstdc++

WORKDIR /app

# Copy production deployment from builder
COPY --from=builder /prod/dist ./dist
COPY --from=builder /prod/node_modules ./node_modules
COPY --from=builder /prod/package.json ./

# Set production environment
ENV NODE_ENV=production
ENV DATA_DIR=/data
ENV BLS_PORT=3100

# Expose BLS HTTP port
EXPOSE 3100

# Declare volume for persistent storage
VOLUME /data

# Create data directory owned by non-root user, then switch
RUN mkdir -p /data && chown node:node /data
USER node

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -q --spider http://localhost:3100/health || exit 1

# Start Crosstown node (use ENTRYPOINT env var to choose)
# Default: entrypoint.js (full-featured node with Nostr relay, peer discovery, payment channels)
# Minimal: entrypoint-bls-only.js (BLS only, no networking)
CMD node dist/${ENTRYPOINT:-entrypoint.js}
