#!/usr/bin/env bash
# Checks what each NAT type does to plain UDP, before any iroh code runs.
#
#   lab/probe-nat.sh cone|restricted|symmetric
#
# Expected:
#   cone        root sees host-a's port 4444 at both destinations (9000, 9001), and host-a
#               receives the unsolicited datagram from root:9002 (full cone).
#   restricted  same mapping, but the unsolicited datagram is dropped.
#   symmetric   a different external port for each destination; the unsolicited one is dropped.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
NFT=${NFT:-/nix/store/66dl0246qv0avvzid7xkj85xqdlrx78a-nftables-1.1.7/bin/nft}
export NFT

if [ "${1:-}" != "--inner" ]; then
	nat=${1:?usage: probe-nat.sh cone|restricted|symmetric}
	exec unshare -Urn bash "$HERE/probe-nat.sh" --inner "$nat"
fi
shift

NAT=$1
source "$HERE/topo.sh"
PORT=4444
STATE=$(mktemp -d /tmp/c2-probe.XXXXXX)
export STATE

trap 'topo_down; rm -rf "$STATE"' EXIT
topo_up "$STATE" "$PORT" "$NAT"
probe() { python3 -I "$HERE/probe.py" "$@"; }

# Root (the relay's side) listens on two destinations, then sends one unsolicited datagram from a
# new source to gateway a's external port.
probe 9000 6 >"$STATE/root-9000.txt" &
p1=$!
probe 9001 6 >"$STATE/root-9001.txt" &
p2=$!
probe 9002 0 --delay 2 --to "10.0.0.2:$PORT" >"$STATE/root-9002.txt" &
p3=$!

# Host a sends to both destinations from port 4444, then keeps listening on 4444.
ns_exec host-a python3 -I "$HERE/probe.py" "$PORT" 5 --delay 1 \
	--to 10.0.0.1:9000 --to 10.0.0.1:9001 >"$STATE/host-a.txt" 2>&1 &
p4=$!
wait "$p1" "$p2" "$p3" "$p4"

echo "== NAT: $NAT"
for f in root-9000 root-9001 host-a root-9002; do
	echo "-- $f"
	cat "$STATE/$f.txt"
done
