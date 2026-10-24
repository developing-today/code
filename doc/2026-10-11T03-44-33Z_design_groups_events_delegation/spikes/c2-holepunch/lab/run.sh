#!/usr/bin/env bash
# One holepunch run in the lab topology (see topo.sh).
#
#   [NAT_B=cone|restricted|symmetric] lab/run.sh NAT [TAG] [WINDOW]
#     NAT     NAT of gateway a (host-a, the echo server)
#     NAT_B   NAT of gateway b (host-b, the dialer); default: the same as NAT
#     TAG     name for the logs (default: 1)
#     WINDOW  seconds to hold the connection after the first reply (default: 60)
#
# Options, as environment variables (defaults are the design's setup):
#     HOME_QAD=on|off   QAD on the home relay (off = Tunnel-style control, no UDP)
#     N0=on|off         n0's relays in the peers' maps (they are unreachable from the lab)
#     STANDIN=on|off    a second relay that runs QAD but serves no relay traffic, in the maps
#
# Writes logs/NAT-TAG-{nft,relay,standin,serve,dial}.txt and prints the dial summary. Everything
# runs in one `unshare -Urn` namespace, so the host's network configuration is not touched. All
# namespaces and processes are removed on exit.
#
# Environment: NFT (nft binary), RELAY_BIN (iroh-relay 1.3.0), C2_BIN (this crate's binary).
set -euo pipefail

LAB=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(dirname "$LAB")
export NFT=${NFT:-/nix/store/66dl0246qv0avvzid7xkj85xqdlrx78a-nftables-1.1.7/bin/nft}
export RELAY_BIN=${RELAY_BIN:-/home/user/work/spikes/tools/bin/iroh-relay}
export C2_BIN=${C2_BIN:-/home/user/work/spikes/target/debug/c2-holepunch}

if [ "${1:-}" != "--inner" ]; then
	nat=${1:?usage: run.sh cone|restricted|symmetric [TAG] [WINDOW]}
	exec unshare -Urn bash "$LAB/run.sh" --inner "$nat" "${2:-1}" "${3:-60}"
fi
shift

NAT=$1
TAG=$2
WINDOW=$3
NAT_B=${NAT_B:-$NAT}
source "$LAB/topo.sh"

PORT=4444
RELAY_URL=http://10.0.0.1:3340
HOME_QAD=${HOME_QAD:-on}
N0=${N0:-on}
STANDIN=${STANDIN:-off}
NAME="$NAT-$TAG"
[ "$NAT_B" = "$NAT" ] || NAME="$NAT-vs-$NAT_B-$TAG"
LOGS="$ROOT/logs"
mkdir -p "$LOGS"
STATE=$(mktemp -d /tmp/c2-run.XXXXXX)
export STATE PORT NAT NAT_B
relay_pid=""
standin_pid=""
serve_pid=""

cleanup() {
	[ -n "$serve_pid" ] && kill "$serve_pid" 2>/dev/null || true
	[ -n "$standin_pid" ] && kill "$standin_pid" 2>/dev/null || true
	[ -n "$relay_pid" ] && kill "$relay_pid" 2>/dev/null || true
	topo_down
	rm -rf "$STATE"
}
trap cleanup EXIT

# Lab certificates: a CA, and a relay certificate for the IP 10.0.0.1 signed by it. The peers trust
# only the CA. Two days is enough for one run.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 \
	-keyout "$STATE/ca.key" -out "$STATE/ca.pem" -subj "/CN=c2 lab CA" \
	-addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
	-keyout "$STATE/relay.key" -out "$STATE/relay.csr" -subj "/CN=10.0.0.1" 2>/dev/null
printf 'subjectAltName=IP:10.0.0.1\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n' \
	>"$STATE/leaf.ext"
openssl x509 -req -in "$STATE/relay.csr" -CA "$STATE/ca.pem" -CAkey "$STATE/ca.key" -CAcreateserial \
	-days 2 -out "$STATE/relay.pem" -extfile "$STATE/leaf.ext" 2>/dev/null
openssl x509 -in "$STATE/ca.pem" -outform DER -out "$STATE/ca.der"

# The home relay in the root namespace: plain HTTP for relay traffic, and QAD on UDP 7842 unless
# HOME_QAD=off. With QAD on this is setup B (vps.toml) without the public TLS listener.
if [ "$HOME_QAD" = on ]; then
	cat >"$STATE/relay.toml" <<EOF
http_bind_addr = "10.0.0.1:3340"
enable_quic_addr_discovery = true
metrics_bind_addr = "127.0.0.1:9090"

[tls]
quic_bind_addr = "10.0.0.1:7842"
cert_mode = "Manual"
manual_cert_path = "$STATE/relay.pem"
manual_key_path = "$STATE/relay.key"
dangerous_http_only = true
EOF
else
	cat >"$STATE/relay.toml" <<EOF
http_bind_addr = "10.0.0.1:3340"
enable_quic_addr_discovery = false
metrics_bind_addr = "127.0.0.1:9090"
EOF
fi

topo_up "$STATE" "$PORT" "$NAT" "$NAT_B"
ns_exec gw-a "$NFT" list ruleset >"$LOGS/$NAME-nft.txt"
ns_exec gw-b "$NFT" list ruleset >>"$LOGS/$NAME-nft.txt"

RUST_LOG=info "$RELAY_BIN" --config-path "$STATE/relay.toml" >"$LOGS/$NAME-relay.txt" 2>&1 &
relay_pid=$!

# The stand-in is a second relay that runs QAD and serves no relay traffic (enable_relay = false).
# It stands in for a QAD-enabled relay in the map that is not the home relay. Its HTTP port is
# never dialled, so it cannot become home.
STANDIN_ARGS=()
if [ "$STANDIN" = on ]; then
	cat >"$STATE/standin.toml" <<EOF
http_bind_addr = "10.0.0.1:3341"
enable_relay = false
enable_metrics = false
enable_quic_addr_discovery = true

[tls]
quic_bind_addr = "10.0.0.1:7843"
cert_mode = "Manual"
manual_cert_path = "$STATE/relay.pem"
manual_key_path = "$STATE/relay.key"
dangerous_http_only = true
EOF
	RUST_LOG=info "$RELAY_BIN" --config-path "$STATE/standin.toml" >"$LOGS/$NAME-standin.txt" 2>&1 &
	standin_pid=$!
	STANDIN_ARGS=(--standin-url http://10.0.0.1:3341)
fi

# The relay is ready when /ping answers 200. Checked with bash's /dev/tcp, so no HTTP client is needed.
ready=no
for _ in $(seq 100); do
	if { exec 3<>/dev/tcp/10.0.0.1/3340; } 2>/dev/null; then
		printf 'GET /ping HTTP/1.1\r\nHost: 10.0.0.1\r\nConnection: close\r\n\r\n' >&3
		status=$(head -n1 <&3 | tr -d '\r')
		exec 3>&- 3<&-
		case "$status" in
		*" 200 "*)
			ready=yes
			break
			;;
		esac
	fi
	sleep 0.1
done
[ "$ready" = yes ] || {
	echo "relay did not answer /ping; see $LOGS/$NAME-relay.txt" >&2
	exit 1
}

# host-a runs the echo endpoint. nsenter without --fork execs the program, so $! is its PID.
nsenter --net="/proc/$(ns_pid host-a)/ns/net" -- "$C2_BIN" serve \
	--port "$PORT" --relay "$RELAY_URL" --ca "$STATE/ca.der" --ticket "$STATE/ticket.txt" \
	--home-qad "$HOME_QAD" --n0 "$N0" "${STANDIN_ARGS[@]}" >"$LOGS/$NAME-serve.txt" 2>&1 &
serve_pid=$!
for _ in $(seq 300); do
	[ -s "$STATE/ticket.txt" ] && break
	kill -0 "$serve_pid" 2>/dev/null || break
	sleep 0.2
done
[ -s "$STATE/ticket.txt" ] || {
	echo "server wrote no ticket; see $LOGS/$NAME-serve.txt" >&2
	exit 1
}

# host-b dials from its own network and holds the connection for WINDOW seconds.
set +e
timeout "$((WINDOW + 180))" nsenter --net="/proc/$(ns_pid host-b)/ns/net" -- "$C2_BIN" dial \
	--port "$PORT" --relay "$RELAY_URL" --ca "$STATE/ca.der" --ticket "$STATE/ticket.txt" \
	--window "$WINDOW" --home-qad "$HOME_QAD" --n0 "$N0" "${STANDIN_ARGS[@]}" \
	>"$LOGS/$NAME-dial.txt" 2>&1
dial_status=$?
set -e

# Strip the colour codes from the relay logs so they read as plain text.
sed -i 's/\x1b\[[0-9;]*m//g' "$LOGS/$NAME-relay.txt" "$LOGS/$NAME-standin.txt" 2>/dev/null || true

echo "== $NAME (dial exit $dial_status)"
cat "$LOGS/$NAME-dial.txt"
