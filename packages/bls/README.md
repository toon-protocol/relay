# @agent-society/bls

Standalone Business Logic Server (BLS) for ILP-gated Nostr event storage.

## Quick Start

```bash
docker pull di3twater/agent-society-bls:latest
```

```bash
docker run -d \
  -e NODE_ID=my-node \
  -e NOSTR_SECRET_KEY=<64-char-hex-secret-key> \
  -e ILP_ADDRESS=g.agent-society.my-node \
  -p 3100:3100 \
  -v bls-data:/data \
  di3twater/agent-society-bls
```

## Docker Hub

Published at [`di3twater/agent-society-bls`](https://hub.docker.com/r/di3twater/agent-society-bls).

### Available Tags

| Tag | Description |
|-----|-------------|
| `latest` | Most recent build from `main` branch |
| `X.Y.Z` | Semantic version (from `bls-vX.Y.Z` git tags) |
| `sha-<short>` | Git commit SHA for traceability |

## Docker Compose

An example `docker-compose.yml` is provided in [`packages/bls/examples/`](examples/).

### Quickstart

```bash
cd packages/bls/examples
cp .env.example .env
# Edit .env with your values (NOSTR_SECRET_KEY is required)
docker compose up
```

### Integrating into an Existing Compose File

To add the BLS to your own `docker-compose.yml`:

1. Copy the `bls` service definition from [`examples/docker-compose.yml`](examples/docker-compose.yml)
2. Add the `bls-data` volume to your top-level `volumes:` section
3. Set the required environment variables (`NODE_ID`, `NOSTR_SECRET_KEY`, `ILP_ADDRESS`)
4. Connect the BLS to the same Docker network as your ILP connector

Your connector can reach the BLS at `http://bls:3100/handle-packet` when both services share a network.

## Kubernetes

Example Kubernetes manifests are provided in [`packages/bls/examples/k8s/`](examples/k8s/).

### Quickstart

1. Edit `secret.yaml` with a real `NOSTR_SECRET_KEY` (generate one with `openssl rand -hex 32`)
2. Edit `configmap.yaml` with your desired `NODE_ID` and `ILP_ADDRESS`
3. Apply all manifests:

```bash
kubectl apply -f packages/bls/examples/k8s/
```

Or use Kustomize:

```bash
kubectl apply -k packages/bls/examples/k8s/
```

### Customizing with Kustomize Overlays

The manifests include a `kustomization.yaml` base. Create overlay directories for different environments:

```
overlays/
  production/
    kustomization.yaml   # references the base, patches as needed
```

Example overlay `kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - ../../packages/bls/examples/k8s
patches:
  - target:
      kind: Deployment
      name: bls
    patch: |
      - op: replace
        path: /spec/template/spec/containers/0/resources/limits/memory
        value: 512Mi
```

### What's Included

| Manifest | Purpose |
|----------|---------|
| `deployment.yaml` | BLS Deployment with resource limits, probes, and volume mount |
| `service.yaml` | ClusterIP Service exposing port 3100 |
| `configmap.yaml` | Non-secret configuration (NODE_ID, ILP_ADDRESS, etc.) |
| `secret.yaml` | Template for secrets (NOSTR_SECRET_KEY) |
| `pvc.yaml` | PersistentVolumeClaim for SQLite event storage |
| `kustomization.yaml` | Kustomize base for easy customization |

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `NODE_ID` | Yes | — | Unique node identifier |
| `NOSTR_SECRET_KEY` | Yes | — | 64-character hex Nostr secret key |
| `ILP_ADDRESS` | Yes | — | Node's ILP address (e.g., `g.agent-society.node1`) |
| `BLS_PORT` | No | `3100` | HTTP port to listen on (1–65535) |
| `BLS_BASE_PRICE_PER_BYTE` | No | `10` | Base price per byte for event storage |
| `OWNER_PUBKEY` | No | — | 64-char hex pubkey for self-write payment bypass |
| `DATA_DIR` | No | `/data` | Directory for SQLite database storage |
| `BLS_KIND_OVERRIDES` | No | — | JSON object mapping event kinds to prices |

### Pricing Variable Aliases

The pricing variables support multiple naming conventions for backwards compatibility:

- **`BLS_BASE_PRICE_PER_BYTE`** (canonical) — takes precedence
- `RELAY_BASE_PRICE_PER_BYTE` — fallback
- `BASE_PRICE_PER_BYTE` — final fallback (unprefixed)

- **`BLS_KIND_OVERRIDES`** (canonical) — takes precedence
- `RELAY_KIND_OVERRIDES` — fallback
- `KIND_OVERRIDES` — final fallback (unprefixed)

### Configuration Details

**NODE_ID** — A unique string identifying this node. Used in health check responses and logging.

**NOSTR_SECRET_KEY** — A 64-character lowercase hex string representing your Nostr private key. The corresponding public key is derived automatically and displayed in logs and health responses. The secret key is never logged.

**ILP_ADDRESS** — Must start with `g.` and contain only alphanumeric characters, dots, and hyphens. Example: `g.agent-society.node1`.

**BLS_PORT** — Integer between 1 and 65535. The container exposes this port for HTTP traffic.

**OWNER_PUBKEY** — When set, events signed by this pubkey bypass payment requirements. Must be a valid 64-character hex Nostr public key.

**DATA_DIR** — Directory where the SQLite database (`events.db`) is stored. Mount a Docker volume here for persistence.

**BLS_KIND_OVERRIDES** — JSON object mapping Nostr event kinds to custom prices. Keys are event kind numbers, values are price strings.

```bash
# Example: kind 0 (metadata) free, kind 30023 (long-form) costs 100 per byte
BLS_KIND_OVERRIDES='{"0":"0","30023":"100"}'
```

## Endpoints

### GET /health

Health check endpoint for container orchestration.

**Response (200 OK):**
```json
{
  "status": "healthy",
  "nodeId": "my-node",
  "pubkey": "6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3",
  "ilpAddress": "g.agent-society.my-node",
  "timestamp": 1234567890
}
```

### POST /handle-packet

Verify an ILP payment and process event storage.

**Request Body:**
```json
{
  "amount": "1000",
  "destination": "g.agent-society.my-node",
  "data": "<base64-encoded TOON event>"
}
```

**Success Response (200 OK):**
```json
{
  "accept": true,
  "fulfillment": "<base64, SHA-256 of event ID>",
  "metadata": {
    "eventId": "<nostr-event-id>",
    "storedAt": 1234567890
  }
}
```

**Rejection Response (400/500):**
```json
{
  "accept": false,
  "code": "F06",
  "message": "Insufficient payment",
  "metadata": {
    "required": "1000",
    "received": "500"
  }
}
```

### ILP Error Codes

| Code | Name | Description |
|------|------|-------------|
| `F00` | BAD_REQUEST | Malformed request or invalid TOON data |
| `F06` | INSUFFICIENT_AMOUNT | Payment amount below required price |
| `T00` | INTERNAL_ERROR | Server-side error during processing |
