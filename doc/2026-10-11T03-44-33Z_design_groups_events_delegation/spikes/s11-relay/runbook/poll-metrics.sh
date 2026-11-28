#!/usr/bin/env bash
# Samples the relay counters every 5 s from the given metrics ports.
# Usage: poll-metrics.sh <seconds> <port>...
secs=$1; shift
end=$((SECONDS + secs))
while [ $SECONDS -lt $end ]; do
  ts=$(date -u +%H:%M:%S)
  for port in "$@"; do
    curl -s "http://127.0.0.1:$port/metrics" \
      | grep -E '^relayserver_(accepts|disconnects|got_ping|sent_pong|send_packets_sent|send_packets_recv)_total ' \
      | sed "s/^/$ts :$port /"
  done
  sleep 5
done
