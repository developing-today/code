#!/usr/bin/env bash
# Runbook for the stock iroh-relay 1.3.0 binary. Every subcommand was run in this environment.
#   relay-ctl.sh start   <config> <pidfile> <log> <base-url>
#   relay-ctl.sh stop    <pidfile>                 SIGINT, the graceful path. SIGTERM is not handled.
#   relay-ctl.sh restart <config> <pidfile> <log> <base-url>
#   relay-ctl.sh serial  <https-base-url>          serial of the certificate the relay serves
#   relay-ctl.sh rotate-cert <new.pem> <new.key> <live.pem> <live.key> <config> <pidfile> <log> <base-url> [ca.pem]
#   relay-ctl.sh health  <base-url>                /ping and /healthz. TLS is verified if CA is set.
# RELAY_BIN overrides the binary. CA=<file> makes curl verify TLS, otherwise curl -k is used.
set -euo pipefail
RELAY_BIN=${RELAY_BIN:-/home/user/work/spikes/tools/bin/iroh-relay}

curl_tls() { if [ -n "${CA:-}" ]; then printf -- '--cacert %s' "$CA"; else printf -- '-k'; fi; }

wait_ready() {  # base-url
  local i code
  for i in $(seq 1 200); do
    code=$(curl -s $(curl_tls) -o /dev/null -w '%{http_code}' --max-time 1 "$1/ping" || true)
    [ "$code" = 200 ] && return 0
    sleep 0.05
  done
  echo "relay not ready at $1" >&2
  return 1
}

start() {  # config pidfile log base-url
  RUST_LOG=${RUST_LOG:-info} "$RELAY_BIN" --config-path "$1" >>"$3" 2>&1 &
  echo $! >"$2"
  wait_ready "$4"
  echo "started pid $(cat "$2") on $4"
}

stop() {  # pidfile
  local pid
  pid=$(cat "$1")
  kill -INT "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do sleep 0.05; done
  echo "stopped pid $pid"
}

restart() {  # config pidfile log base-url
  stop "$2"
  start "$1" "$2" "$3" "$4"
}

serial() {  # https-base-url
  echo | openssl s_client -connect "${1#https://}" -servername "${SERVERNAME:-localhost}" 2>/dev/null \
    | openssl x509 -noout -serial
}

rotate_cert() {  # new.pem new.key live.pem live.key config pidfile log base-url [ca.pem]
  local new_cert=$1 new_key=$2 live_cert=$3 live_key=$4 config=$5 pidfile=$6 log=$7 base=$8
  if [ "$(openssl x509 -in "$new_cert" -noout -pubkey | openssl sha256)" != \
       "$(openssl pkey -in "$new_key" -pubout | openssl sha256)" ]; then
    echo "certificate and key do not match; nothing changed" >&2
    return 1
  fi
  if [ -n "${9:-}" ] && ! openssl verify -CAfile "$9" "$new_cert" >/dev/null; then
    echo "certificate does not verify against $9; nothing changed" >&2
    return 1
  fi
  local stamp
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  cp -p "$live_cert" "$live_cert.bak-$stamp"
  cp -p "$live_key" "$live_key.bak-$stamp"
  install -m 0644 "$new_cert" "$live_cert.tmp" && mv "$live_cert.tmp" "$live_cert"
  install -m 0600 "$new_key" "$live_key.tmp" && mv "$live_key.tmp" "$live_key"
  echo "installed; previous files kept as *.bak-$stamp"
  restart "$config" "$pidfile" "$log" "$base"
  echo "served: $(serial "$base")"
}

health() {  # base-url
  local p
  for p in /ping /healthz; do
    echo "$p $(curl -s $(curl_tls) -o /dev/null -w '%{http_code}' "$1$p")"
  done
  echo "healthz body: $(curl -s $(curl_tls) "$1/healthz")"
}

case "${1:-}" in
  start) shift; start "$@" ;;
  stop) shift; stop "$@" ;;
  restart) shift; restart "$@" ;;
  serial) shift; serial "$@" ;;
  rotate-cert) shift; rotate_cert "$@" ;;
  health) shift; health "$@" ;;
  *) sed -n '2,11p' "$0"; exit 2 ;;
esac
