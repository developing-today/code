#!/usr/bin/env bash
# N30 runner. One scenario, N runs. Each run starts a fresh server process and a fresh client
# process in one user+network namespace (unshare -Urn), so every side logs only what it saw.
#
#   lab/run.sh SCENARIO RUNS        (from anywhere; logs go to logs/SCENARIO/run-NN/)
#
# Scenarios:
#   loopback-refuse  server binds 127.0.0.1 only. Unlisted client (seed 3) binds 127.0.0.1.
#                    Refusal in after_handshake. Client opens no stream.
#   multi-refuse     server and unlisted client bind 0.0.0.0. lo also carries 10.200.0.1 (server)
#                    and 10.200.0.2 (client), so each side has candidates beyond the ticket hint.
#                    Refusal in after_handshake.
#   multi-allow      as multi-refuse, but the allowed client (seed 2) opens a stream. Control.
#   multi-app        after_handshake accepts everyone. Unlisted client opens a stream, which the
#                    server refuses after it opens.
#   filter-refuse    Incoming::refuse() before any handshake, unlisted client.
#
# The ticket the client gets is the server's EndpointId plus 127.0.0.1:PORT, nothing else.
set -euo pipefail

SELF=$(readlink -f "${BASH_SOURCE[0]}")
HERE=$(dirname "$(dirname "$SELF")")
BIN=${BIN:-/home/user/work/spikes/target/debug/iroh-allowlist-spike}

if [ "${1:-}" != "--inner" ]; then
	exec unshare -Urn bash "$SELF" --inner "$@"
fi
shift

scenario=${1:?usage: run.sh SCENARIO RUNS}
runs=${2:?usage: run.sh SCENARIO RUNS}

# Inside the namespace: bring lo up and give it two extra addresses. Anything bound to 0.0.0.0
# advertises these as direct addresses (netdev_impl.rs classes lo addresses as loopback).
ip link set lo up
ip addr add 10.200.0.1/32 dev lo
ip addr add 10.200.0.2/32 dev lo

case "$scenario" in
loopback-refuse) srv_bind=127.0.0.1:0 srv_mode=refuse seed=3 cli_bind=127.0.0.1:0 kind=connect ;;
multi-refuse) srv_bind=0.0.0.0:0 srv_mode=refuse seed=3 cli_bind=0.0.0.0:0 kind=connect ;;
multi-allow) srv_bind=0.0.0.0:0 srv_mode=refuse seed=2 cli_bind=0.0.0.0:0 kind=stream ;;
multi-app) srv_bind=0.0.0.0:0 srv_mode=app seed=3 cli_bind=0.0.0.0:0 kind=stream ;;
filter-refuse) srv_bind=0.0.0.0:0 srv_mode=filter seed=3 cli_bind=0.0.0.0:0 kind=connect ;;
*)
	echo "unknown scenario: $scenario" >&2
	exit 2
	;;
esac

out="${LOG_ROOT:-$HERE/logs}/$scenario"
rm -rf "$out"
mkdir -p "$out"
echo "scenario=$scenario runs=$runs server=$srv_bind/$srv_mode client=seed$seed/$cli_bind/$kind"

for i in $(seq -w 1 "$runs"); do
	d="$out/run-$i"
	mkdir -p "$d"
	"$BIN" server --bind "$srv_bind" --mode "$srv_mode" --ready "$d/ready.txt" >"$d/server.log" 2>&1 &
	spid=$!
	for _ in $(seq 1 200); do
		[ -s "$d/ready.txt" ] && break
		sleep 0.05
	done
	if [ ! -s "$d/ready.txt" ]; then
		echo "run $i: server never became ready (see $d/server.log)" >&2
		kill "$spid" 2>/dev/null || true
		wait "$spid" 2>/dev/null || true
		continue
	fi
	"$BIN" client --seed "$seed" --bind "$cli_bind" --kind "$kind" --ticket "$d/ready.txt" \
		>"$d/client.log" 2>&1 || true
	sleep 0.3
	kill "$spid" 2>/dev/null || true
	wait "$spid" 2>/dev/null || true
done

echo "done: $out"
