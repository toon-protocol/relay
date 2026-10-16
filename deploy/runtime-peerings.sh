#!/usr/bin/env bash
# The relay's forwarded legs, replayed over the operator surface.
#
# A forwarded leg (a peering and the route over it) is runtime state by design:
# it lives in runtime-peers.json in the connector's state volume and in no
# committed config, and writing it into connector.toml would take it away from
# the operator surface for good (connector ADR 0034, docs/devnet-pricing.md).
# So this file does not hold the legs as config. It is the way to put a leg on
# this hub, or take one off, by the same signed writes an operator makes by hand:
#
#   POST /peers         { id, url, fee, max_packet_amount, chain, deposit }
#   POST /routes/peers  { prefix, peer_id, price }
#
# Both are idempotent. A repeated POST /peers finds the channel it already
# opened and deposits nothing; a repeated route write rewrites the same row.
#
# Usage, on the box, from deploy/:
#
#   ./runtime-peerings.sh node <id> <url> <prefix> <deposit> <price>
#       a peering toward an agent node that has joined the devnet, and a
#       route over it, so a payer anywhere can reach that node's app through
#       this hub. <url> is the node's /ilp URL: http://<56 chars>.anyone/ilp
#       for a hidden service, which needs this bundle's `anon` sidecar.
#       <prefix> is the app's address, g.toon.<segment>.<app>, from the
#       node's `toon route list`. <price> is what a payer pays this hub for
#       the route: the app's price plus this hop's fee of 1.
#
#   ./runtime-peerings.sh remove <id> <prefix>
#       take a node's route and peering away again
#
#   ./runtime-peerings.sh devnet
#       opens nothing any more. It used to open the gas, store and echo legs;
#       the devnet now runs one connector, this one, which terminates those
#       apps' routes in connector.toml. Prints the cutover that takes the old
#       legs off a box that still has them (deploy/README.md).
#
# Needs the operator write key (operator-write.key, the PRIVATE half, which is
# kept wherever writes are signed from) and the connector repository's
# docs/operators/sign-write.sh, named by SIGN_WRITE. Each opening deposit is
# paid from this node's settlement key in USDC, with its gas in ETH: keep the
# key funded, or the sweep that lands what the node is paid stops too.
set -euo pipefail

cd "$(dirname "$0")"
URL=${CONNECTOR_URL:-http://127.0.0.1:3000}
KEY=${OPERATOR_WRITE_KEY:-./operator-write.key}
SIGN_WRITE=${SIGN_WRITE:-./sign-write.sh}

if [ "${1:-}" = devnet ]; then
    # A config [[routes]] prefix and a runtime route at the same prefix
    # cannot both be live: the connector refuses a runtime write to a
    # prefix connector.toml owns, and a runtime row it loads under such a
    # prefix is shadowed, inert, and can no longer be deleted over the
    # operator surface (connector ADR 0034). So these legs come off BEFORE
    # the connector restarts on the connector.toml that owns them.
    cat >&2 <<'CUTOVER'
runtime-peerings: `devnet` opens no legs any more. connector.toml terminates
g.toon.store, g.toon.relay.store, g.toon.gas, g.toon.relay.gas and g.toon.echo
itself. On a box that still has the old legs, BEFORE the connector restarts on
that connector.toml:

  ./runtime-peerings.sh remove gas   g.toon.relay.gas
  ./runtime-peerings.sh remove store g.toon.relay.store
  ./runtime-peerings.sh remove echo  g.toon.echo

then withdraw each leg's outbound channel deposit (deploy/README.md).
CUTOVER
    exit 2
fi

[ -x "$SIGN_WRITE" ] || { echo "runtime-peerings: needs SIGN_WRITE, the connector's docs/operators/sign-write.sh" >&2; exit 2; }
[ -r "$KEY" ] || { echo "runtime-peerings: needs the operator write key at $KEY (OPERATOR_WRITE_KEY)" >&2; exit 2; }

write() { # METHOD PATH [BODY]
    # A DELETE answers with an empty body, and grep -v over no lines exits 1,
    # which pipefail turns into this script's exit: `remove` used to stop after
    # its first write, leaving the peering in place.
    "$SIGN_WRITE" -k "$KEY" -X "$1" -p "$2" ${3:+-b "$3"} -u "$URL" | { grep -v -E '^(Signature|Signature-Input|Content-Digest):' || true; }
    echo
}

peer() { # ID URL CHAIN DEPOSIT [FEE] [MAX_PACKET]
    write POST /peers "{\"id\":\"$1\",\"url\":\"$2\",\"fee\":${5:-1},\"max_packet_amount\":${6:-100000},\"chain\":\"$3\",\"deposit\":$4}"
}

route() { # PREFIX PEER PRICE_JSON
    write POST /routes/peers "{\"prefix\":\"$1\",\"peer_id\":\"$2\",\"price\":$3}"
}

case ${1:-} in
    node)
        [ $# -eq 6 ] || { sed -n '19,26p' "$0"; exit 2; }
        peer "$2" "$3" evm "$5"
        route "$4" "$2" "$6"
        ;;
    remove)
        [ $# -eq 3 ] || { sed -n '28,29p' "$0"; exit 2; }
        write DELETE "/routes/peers/$3"
        write DELETE "/peers/$2"
        ;;
    *)
        sed -n '2,41p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
