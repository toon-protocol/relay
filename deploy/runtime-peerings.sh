#!/usr/bin/env bash
# The relay's forwarded legs, replayed over the operator surface.
#
# A forwarded leg (a peering and the route over it) is runtime state by design:
# it lives in runtime-peers.json in the connector's state volume and in no
# committed config, and writing it into connector.toml would take it away from
# the operator surface for good (connector ADR 0034, docs/devnet-pricing.md).
# So this file does not hold the legs as config. It is the record of which legs
# the devnet relay runs, and the way to put them back on a relay whose state
# volume is new, by the same two signed writes an operator makes by hand:
#
#   POST /peers         { id, url, fee, max_packet_amount, chain, deposit }
#   POST /routes/peers  { prefix, peer_id, price }
#
# Both are idempotent. A repeated POST /peers finds the channel it already
# opened and deposits nothing; a repeated route write rewrites the same row.
#
# Usage, on the box, from deploy/:
#
#   ./runtime-peerings.sh devnet
#       every leg the devnet relay forwards: gas, store, echo
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

[ -x "$SIGN_WRITE" ] || { echo "runtime-peerings: needs SIGN_WRITE, the connector's docs/operators/sign-write.sh" >&2; exit 2; }
[ -r "$KEY" ] || { echo "runtime-peerings: needs the operator write key at $KEY (OPERATOR_WRITE_KEY)" >&2; exit 2; }

write() { # METHOD PATH [BODY]
    "$SIGN_WRITE" -k "$KEY" -X "$1" -p "$2" ${3:+-b "$3"} -u "$URL" | grep -v -E '^(Signature|Signature-Input|Content-Digest):'
    echo
}

peer() { # ID URL CHAIN DEPOSIT [FEE] [MAX_PACKET]
    write POST /peers "{\"id\":\"$1\",\"url\":\"$2\",\"fee\":${5:-1},\"max_packet_amount\":${6:-100000},\"chain\":\"$3\",\"deposit\":$4}"
}

route() { # PREFIX PEER PRICE_JSON
    write POST /routes/peers "{\"prefix\":\"$1\",\"peer_id\":\"$2\",\"price\":$3}"
}

case ${1:-} in
    devnet)
        # Each price is the far node's own price plus this hop's fee of 1.
        peer gas   https://proxy.gas.devnet.toonprotocol.dev/ilp  evm    10000000
        route g.toon.relay.gas   gas   1001
        peer store https://proxy.ario.devnet.toonprotocol.dev/ilp solana 10000000
        route g.toon.relay.store store '{"base":1001,"per_kib":10}'
        # The Echo Merchant (infra#55) terminates g.toon.echo itself, at 1 plus
        # 1 per KiB. This leg is what makes the Echo flow's "joining the devnet
        # reaches g.toon.echo" true.
        peer echo  https://proxy.echo.devnet.toonprotocol.dev/ilp evm    1000000
        route g.toon.echo        echo  '{"base":2,"per_kib":1}'
        ;;
    node)
        [ $# -eq 6 ] || { sed -n '23,30p' "$0"; exit 2; }
        peer "$2" "$3" evm "$5"
        route "$4" "$2" "$6"
        ;;
    remove)
        [ $# -eq 3 ] || { sed -n '32,33p' "$0"; exit 2; }
        write DELETE "/routes/peers/$3"
        write DELETE "/peers/$2"
        ;;
    *)
        sed -n '2,39p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
