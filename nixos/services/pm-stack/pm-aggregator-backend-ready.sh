#!/usr/bin/env bash
# Command contract: <backend-port> <timeout-seconds>.
# Wait for the private aggregator listener before systemd starts the socket
# proxy. This keeps the public socket's first cold connection queued until the
# Type=simple service has actually bound its backend port.
set -euo pipefail

usage() {
  echo "usage: pm-aggregator-backend-ready.sh <backend-port> <timeout-seconds>" >&2
}

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

port="$1"
timeout_seconds="$2"
if [[ ! $port =~ ^[0-9]{1,5}$ || ! $timeout_seconds =~ ^[0-9]{1,3}$ ]]; then
  usage
  exit 2
fi

port_num=$((10#$port))
timeout_num=$((10#$timeout_seconds))
if ((port_num < 1 || port_num > 65535 || timeout_num < 1 || timeout_num > 300)); then
  usage
  exit 2
fi

deadline=$((SECONDS + timeout_num))
while ((SECONDS < deadline)); do
  if exec 3<>"/dev/tcp/127.0.0.1/${port_num}" 2>/dev/null; then
    exec 3>&- 3<&-
    exit 0
  fi
  sleep 0.1
done

echo "pm-aggregator backend 127.0.0.1:${port_num} did not accept a connection within ${timeout_num}s" >&2
exit 1
