#!/usr/bin/env bash
# The conformance suite against both relay images, inside a Docker-in-Docker
# daemon: for a host whose firewall drops traffic from the Docker bridge, where
# a relay never reaches the suite's stub connector (see ../README.md). Each
# image is run as the implementation under test, and the image-swap case gets
# both.
#
#   TYPESCRIPT_IMAGE=<image> RUST_IMAGE=<image> soak/dind-conformance.sh
#
# Both images must already be in the host's daemon, by tag. Arguments are
# passed to vitest. Exits non-zero if either run fails.
set -uo pipefail
: "${TYPESCRIPT_IMAGE:?the TypeScript relay image}" "${RUST_IMAGE:?the Rust relay image}"

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
dind="soak-dind-$$"
cli=$(mktemp -d)
trap 'docker rm -f -v "$dind" >/dev/null 2>&1; rm -rf "$cli"' EXIT

docker run -d --privileged --name "$dind" -e DOCKER_TLS_CERTDIR= docker:dind >/dev/null || exit 1
until docker exec "$dind" docker info >/dev/null 2>&1; do sleep 1; done
docker save "$TYPESCRIPT_IMAGE" "$RUST_IMAGE" | docker exec -i "$dind" docker load || exit 1
# The suite drives `docker`; the node image has none, and this one is static.
docker cp -q "$dind:/usr/local/bin/docker" "$cli/docker" || exit 1

suite() { # implementation, image, command, vitest arguments
  local implementation=$1 image=$2 command=$3
  shift 3
  echo "=== $implementation: $image"
  docker run --rm --network "container:$dind" -u "$(id -u):$(id -g)" \
    -e HOME=/tmp -e DOCKER_HOST=tcp://127.0.0.1:2375 \
    -e CONFORMANCE_IMPL="$implementation" -e CONFORMANCE_IMAGE="$image" \
    -e CONFORMANCE_COMMAND="$command" \
    -e CONFORMANCE_TYPESCRIPT_IMAGE="$TYPESCRIPT_IMAGE" \
    -e CONFORMANCE_RUST_IMAGE="$RUST_IMAGE" \
    -v "$repo:$repo" -v "$cli/docker:/usr/local/bin/docker:ro" \
    -w "$repo/packages/conformance" node:22-bookworm-slim \
    "$repo/node_modules/.bin/vitest" run --config vitest.config.ts "$@"
}

failed=0
suite rust "$RUST_IMAGE" relay "$@" || failed=1
suite typescript "$TYPESCRIPT_IMAGE" 'node dist/cli.js' "$@" || failed=1
exit $failed
