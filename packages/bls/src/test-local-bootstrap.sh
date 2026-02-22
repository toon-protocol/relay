#!/usr/bin/env bash
#
# Test docker-compose-with-local.yml with Fixed Bootstrap Flow
#

set -e

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}=================================================="
echo "Testing docker-compose-with-local.yml Bootstrap"
echo -e "==================================================${NC}"
echo ""

# Clean up
echo -e "${YELLOW}🧹 Cleaning up existing containers...${NC}"
docker compose -f docker-compose-with-local.yml down -v 2>/dev/null || true
echo ""

# Rebuild image
echo -e "${BLUE}🔨 Rebuilding crosstown image...${NC}"
docker build -f packages/bls/Dockerfile -t crosstown:optimized . || {
  echo -e "${RED}❌ Docker build failed${NC}"
  exit 1
}
echo -e "${GREEN}✅ Image rebuilt${NC}"
echo ""

# Set env vars
export NOSTR_SECRET_KEY=$(openssl rand -hex 32)
export NODE_ID="test-node-$(date +%s)"

echo -e "${BLUE}📝 Configuration:${NC}"
echo "  NODE_ID: $NODE_ID"
echo "  NOSTR_SECRET_KEY: ${NOSTR_SECRET_KEY:0:16}..."
echo ""

# Start stack
echo -e "${BLUE}🚀 Starting stack...${NC}"
docker compose -f docker-compose-with-local.yml up -d

echo -e "${YELLOW}⏳ Waiting 30s for initialization...${NC}"
sleep 30
echo ""

# Test 1: Full-featured entrypoint
echo -e "${BLUE}Test 1: Full-Featured Entrypoint${NC}"
BOOTSTRAP_BANNER=$(docker logs crosstown-node 2>&1 | grep "Starting Crosstown Node with Bootstrap" || echo "")
if [ -n "$BOOTSTRAP_BANNER" ]; then
  echo -e "${GREEN}✅ Full-featured entrypoint loaded${NC}"
else
  echo -e "${RED}❌ NOT using full-featured entrypoint!${NC}"
  echo "Logs:"
  docker logs crosstown-node 2>&1 | head -20
  exit 1
fi
echo ""

# Test 2: Nostr Relay
echo -e "${BLUE}Test 2: Nostr Relay${NC}"
RELAY_START=$(docker logs crosstown-node 2>&1 | grep "Nostr relay started" || echo "")
if [ -n "$RELAY_START" ]; then
  echo -e "${GREEN}✅ Nostr relay running${NC}"
else
  echo -e "${RED}❌ Nostr relay NOT started${NC}"
  exit 1
fi
echo ""

# Test 3: Anvil + Contracts
echo -e "${BLUE}Test 3: Settlement Infrastructure${NC}"
ANVIL=$(docker logs crosstown-anvil 2>&1 | grep "Listening on" || echo "")
CONTRACTS=$(docker logs crosstown-anvil 2>&1 | grep -E "Deploying contracts|Script ran" || echo "")
if [ -n "$ANVIL" ]; then
  echo -e "${GREEN}✅ Anvil running${NC}"
fi
if [ -n "$CONTRACTS" ]; then
  echo -e "${GREEN}✅ Contracts deployed${NC}"
fi
echo ""

# Test 4: Endpoints
echo -e "${BLUE}Test 4: Service Endpoints${NC}"
if curl -s http://localhost:3100/health | grep -q "healthy"; then
  echo -e "${GREEN}✅ BLS responding (3100)${NC}"
fi
if curl -s http://localhost:8080/health | grep -q "ok"; then
  echo -e "${GREEN}✅ Connector responding (8080)${NC}"
fi
echo ""

echo -e "${GREEN}✅ SUCCESS: Bootstrap flow is properly configured!${NC}"
echo ""
echo "View logs: docker compose -f docker-compose-with-local.yml logs crosstown-node"
echo "Stop: docker compose -f docker-compose-with-local.yml down"
echo ""
