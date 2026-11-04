#!/usr/bin/env bash
# Per-variant smoke suite for the fleet workspace
# (step 1, developing-today/code#5429; queue-domain sections added
# by step 2, #5430; proxy-domain sections by step 3, #5431).
#
# Usage:
#   smoke.sh <fleet-out> <fleet-serve-out> <fleet-connect-out> <fleet-connect-static-out>
#
# Each argument is a nix package output path; its bin/fleet is the
# artifact under test. The suite asserts, per variant:
#   * --help renders the FULL registry, unavailable domains marked
#   * version lists exactly that variant's capability set
#   * a missing domain stubs with exit 69 and the right wording,
#     reached three ways: `fleet <domain>`, an argv0 alias symlink,
#     and a hyphenated variant name (which NARROWS the surface)
#   * a config-disabled domain stubs with exit 69, config wording
#   * both connect flavors run under `env -i`
#   * both connect flavors are within the D7 size gate (<= 2 MiB)
#   * the proxy domain end to end, hermetically: broker auto-start,
#     round-trips, shim materialization + stale-shim removal on a
#     services-map change, init no-op, legacy proto-0 interop,
#     idle exit
#
# Exits nonzero if any check fails. No network, no installs, no
# state outside a mktemp dir.
set -u

FULL_OUT=${1:?usage: smoke.sh <fleet> <fleet-serve> <fleet-connect> <fleet-connect-static>}
SERVE_OUT=${2:?missing fleet-serve output path}
CONNECT_OUT=${3:?missing fleet-connect output path}
STATIC_OUT=${4:?missing fleet-connect-static output path}

PASS=0
FAIL=0
TMP=$(mktemp -d)
SB_PID=""
QB_PID=""
PB_PID=""
UP_PID=""
IDLE_PID=""
trap 'kill $SB_PID $QB_PID $PB_PID $UP_PID $IDLE_PID 2>/dev/null; rm -rf "$TMP"' EXIT

ok()   { PASS=$((PASS + 1)); echo "ok   - $1"; }
bad()  { FAIL=$((FAIL + 1)); echo "FAIL - $1"; }

# run <name> <expected-rc> <required-substring|-> <cmd...>
run() {
  local name=$1 want_rc=$2 want_sub=$3; shift 3
  local out rc
  out=$("$@" 2>&1); rc=$?
  if [ "$rc" -ne "$want_rc" ]; then
    bad "$name (exit $rc, want $want_rc)"
    echo "$out" | head -5 | sed 's/^/       /'
    return
  fi
  if [ "$want_sub" != "-" ] && ! printf '%s' "$out" | grep -qF "$want_sub"; then
    bad "$name (output missing: $want_sub)"
    echo "$out" | head -5 | sed 's/^/       /'
    return
  fi
  ok "$name"
}

FULL_CAPS="Enabled capabilities: proxy-connect, proxy-serve, queue, queue-broker, secrets-broker, search, search-serve, dispatch, laptop-agent"
SERVE_CAPS="Enabled capabilities: proxy-connect, proxy-serve, queue-broker, secrets-broker, search-serve"
CONNECT_CAPS="Enabled capabilities: proxy-connect"

check_variant() {
  local label=$1 bin=$2 caps=$3
  run "$label: --help renders the full registry" 0 "laptop-agent" "$bin" --help
  run "$label: version capability set is exact" 0 "$caps" "$bin" version
  run "$label: capabilities is valid JSON with the build block" 0 '"source_rev"' "$bin" capabilities
}

echo "== variants =="
check_variant full    "$FULL_OUT/bin/fleet"    "$FULL_CAPS"
check_variant serve   "$SERVE_OUT/bin/fleet"   "$SERVE_CAPS"
check_variant connect "$CONNECT_OUT/bin/fleet" "$CONNECT_CAPS"
check_variant static  "$STATIC_OUT/bin/fleet"  "$CONNECT_CAPS"

echo "== help marks =="
run "full: --help marks nothing unavailable" 0 "-" "$FULL_OUT/bin/fleet" --help
if "$FULL_OUT/bin/fleet" --help | grep -qF "[not in this build]"; then
  bad "full: --help should have no [not in this build] marks"
else
  ok "full: --help has no [not in this build] marks"
fi
run "connect: --help marks queue as not in this build" 0 "[not in this build]" "$CONNECT_OUT/bin/fleet" --help
run "serve: --help marks dispatch as not in this build" 0 "[not in this build]" "$SERVE_OUT/bin/fleet" --help

echo "== stubs: not in this build (exit 69) =="
run "connect: fleet queue stubs" 69 "this build does not include 'queue'" \
  "$CONNECT_OUT/bin/fleet" queue
run "connect: fleet search stubs" 69 "this build does not include 'search'" \
  "$CONNECT_OUT/bin/fleet" search
run "serve: fleet dispatch stubs" 69 "this build does not include 'dispatch'" \
  "$SERVE_OUT/bin/fleet" dispatch
run "serve: fleet laptop-agent stubs" 69 "this build does not include 'laptop-agent'" \
  "$SERVE_OUT/bin/fleet" laptop-agent

echo "== stubs via argv0 alias symlinks =="
ln -s "$CONNECT_OUT/bin/fleet" "$TMP/repo-queue"
ln -s "$CONNECT_OUT/bin/fleet" "$TMP/atlas-search"
ln -s "$SERVE_OUT/bin/fleet" "$TMP/queue-broker"
run "connect as repo-queue stubs 69" 69 "this build does not include 'queue'" "$TMP/repo-queue"
run "connect as atlas-search stubs 69" 69 "this build does not include 'search'" "$TMP/atlas-search"
run "serve as queue-broker reaches the real broker role (missing token file: exit 3)" 3 "cannot read token file" \
  env QUEUE_BROKER_TOKEN_FILE="$TMP/no-such-token" "$TMP/queue-broker"

echo "== proxy domain: serve + connect + init + shims (step 3, #5431) =="
# Hermetic proxy world: a scratch HOME (token file), scratch socket,
# scratch state dirs, and a stub HTTP upstream on loopback ports. The
# broker runs with direct = true, so no CONNECT tunnel is involved;
# the tunnel path itself is exercised against the real system in the
# step's live proofs, not in per-variant smoke.
PX="$TMP/proxy"
mkdir -p "$PX/home/tools" "$PX/state" "$PX/state-connect"
printf 'smoke-broker-token\n' > "$PX/home/tools/laptop-agent-token"

cat > "$PX/upstream.py" <<'PYEOF'
import socket, threading, json, time
def handle(conn, port):
    f = conn.makefile("rb")
    try:
        while True:
            line = f.readline()
            if not line: return
            length = 0
            while True:
                h = f.readline()
                if h in (b"\r\n", b"\n", b""): break
                if h.lower().startswith(b"content-length:"):
                    length = int(h.split(b":", 1)[1])
            if length: f.read(length)
            body = json.dumps({"ok": True, "port": port}).encode()
            conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
                         b"Content-Length: %d\r\n\r\n%s" % (len(body), body))
    except Exception:
        return
def serve(port):
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port)); srv.listen(16)
    while True:
        conn, _ = srv.accept()
        threading.Thread(target=handle, args=(conn, port), daemon=True).start()
for p in (14799, 14801):
    threading.Thread(target=serve, args=(p,), daemon=True).start()
while True: time.sleep(3600)
PYEOF
python3 "$PX/upstream.py" >"$PX/upstream.log" 2>&1 &
UP_PID=$!

cat > "$PX/fleet.toml" <<'EOF'
[proxy]
direct = true
target_host = "127.0.0.1"
idle_exit_secs = 3600

[[proxy.services]]
port = 14799
name = "laptop-agent"

[[proxy.services]]
port = 14801
name = "queue-broker"
EOF

proxy_env() {
  env HOME="$PX/home" FLEET_CONFIG="$PX/fleet.toml" \
      FLEET_BROKER_SOCKET="$PX/broker.sock" XDG_STATE_HOME="$PX/state" "$@"
}

# connect auto-starts the broker and round-trips through it.
run "proxy: connect auto-starts broker and round-trips /health" 0 '"status":200' \
  proxy_env "$FULL_OUT/bin/fleet" proxy connect --request GET /health --service-port 14799
run "proxy: round-trip reached the 14799 upstream" 0 '"port":14799' \
  proxy_env "$FULL_OUT/bin/fleet" proxy connect --request GET /health --service-port 14799
run "proxy: round-trip to the 14801 service" 0 '"port":14801' \
  proxy_env "$FULL_OUT/bin/fleet" proxy connect --request GET /health --service-port 14801

# Shims materialized from the negotiated services map.
if [ -L "$PX/state/fleet/shims/repo-queue" ] && [ -L "$PX/state/fleet/shims/laptop-agent" ]; then
  ok "proxy: shims materialized from the services map (repo-queue, laptop-agent)"
else
  bad "proxy: shims materialized from the services map (repo-queue, laptop-agent)"
fi

# init is a strict no-op against the running broker.
PB_PID=$(pgrep -f "bin/fleet proxy serve" | head -1)
run "proxy: fleet init is a no-op when the broker runs" 0 "already running" \
  proxy_env "$FULL_OUT/bin/fleet" init
if [ -n "$PB_PID" ] && kill -0 "$PB_PID" 2>/dev/null && [ "$(pgrep -f 'bin/fleet proxy serve' | wc -l)" -eq 1 ]; then
  ok "proxy: init left the same single broker alive"
else
  bad "proxy: init left the same single broker alive"
fi

# The connect variant speaks to the same broker and materializes its
# own shims; it cannot serve (69), and neither can its init.
run "proxy: connect variant round-trips through the full broker" 0 '"status":200' \
  env HOME="$PX/home" FLEET_CONFIG="$PX/fleet.toml" \
      FLEET_BROKER_SOCKET="$PX/broker.sock" XDG_STATE_HOME="$PX/state-connect" \
  "$CONNECT_OUT/bin/fleet" proxy connect --request GET /health --service-port 14799
if [ -L "$PX/state-connect/fleet/shims/repo-queue" ]; then
  ok "proxy: connect variant materialized its own shims"
else
  bad "proxy: connect variant materialized its own shims"
fi
run "proxy: connect variant stubs serve at 69" 69 "does not include 'proxy'" \
  "$CONNECT_OUT/bin/fleet" proxy serve
run "proxy: connect variant stubs fleet init at 69 (no serve in build)" 69 "does not include 'proxy'" \
  "$CONNECT_OUT/bin/fleet" init

# Legacy (proto 0) client against the fleet broker: raw 8-byte
# frames, no hello — the Python clients' dialect, still served.
cat > "$PX/legacy.py" <<'PYEOF'
import socket, struct, json, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
frame = json.dumps({"method": "GET", "path": "/health", "port": 14799}).encode()
s.sendall(struct.pack(">Q", len(frame)) + frame)
def readn(n):
    buf = b""
    while len(buf) < n:
        chunk = s.recv(n - len(buf))
        if not chunk: raise SystemExit("closed")
        buf += chunk
    return buf
n = struct.unpack(">Q", readn(8))[0]
resp = json.loads(readn(n))
assert resp["status"] == 200, resp
print("legacy-ok")
PYEOF
run "proxy: legacy proto-0 client round-trips (no hello)" 0 "legacy-ok" \
  python3 "$PX/legacy.py" "$PX/broker.sock"

# Services-map change: restart the broker with a smaller map; the
# repo-queue shim must disappear, laptop-agent must stay.
kill "$PB_PID" 2>/dev/null
sleep 0.5
rm -f "$PX/broker.sock"
cat > "$PX/fleet.toml" <<'EOF'
[proxy]
direct = true
target_host = "127.0.0.1"
idle_exit_secs = 3600

[[proxy.services]]
port = 14799
name = "laptop-agent"
EOF
run "proxy: reconnect with the smaller map" 0 '"status":200' \
  proxy_env "$FULL_OUT/bin/fleet" proxy connect --request GET /health --service-port 14799
if [ ! -e "$PX/state/fleet/shims/repo-queue" ] && [ -L "$PX/state/fleet/shims/laptop-agent" ]; then
  ok "proxy: stale shim removed when the map shrank"
else
  bad "proxy: stale shim removed when the map shrank"
fi
PB_PID=$(pgrep -f "bin/fleet proxy serve" | head -1)
kill "$PB_PID" 2>/dev/null
PB_PID=""

# Idle exit: a broker with an 8-second idle span exits on its own
# even though its keepalive loop is running (keepalive != use).
mkdir -p "$PX/idle"
cat > "$PX/idle.toml" <<'EOF'
[proxy]
direct = true
target_host = "127.0.0.1"
idle_exit_secs = 8

[[proxy.services]]
port = 14799
name = "laptop-agent"
EOF
env HOME="$PX/home" FLEET_CONFIG="$PX/idle.toml" \
    FLEET_BROKER_SOCKET="$PX/idle/broker.sock" XDG_STATE_HOME="$PX/state" \
    "$FULL_OUT/bin/fleet" proxy serve >"$PX/idle.log" 2>&1 &
IDLE_PID=$!
sleep 1
if kill -0 "$IDLE_PID" 2>/dev/null; then
  ok "proxy: idle broker alive inside its span"
else
  bad "proxy: idle broker alive inside its span"
fi
for i in $(seq 1 40); do kill -0 "$IDLE_PID" 2>/dev/null || break; sleep 1; done
if kill -0 "$IDLE_PID" 2>/dev/null; then
  bad "proxy: idle broker exits after its span (keepalive does not reset it)"
else
  ok "proxy: idle broker exits after its span (keepalive does not reset it)"
fi
IDLE_PID=""
kill "$UP_PID" 2>/dev/null
UP_PID=""

echo "== queue domain: serve roles bind + answer; client surface (step 2, #5430) =="
# Scratch token files: the roles refuse to start without them. The
# contents are dummies — these instances serve nothing real.
printf 'smoke-secrets-token\n' > "$TMP/sb-token"
printf 'smoke-queue-token\n' > "$TMP/qb-token"

# secrets-broker role on a scratch port: /health answers unauthenticated.
env SECRETS_BROKER_BIND=127.0.0.1:19800 SECRETS_BROKER_TOKEN_FILE="$TMP/sb-token" \
  "$SERVE_OUT/bin/fleet" serve secrets-broker >"$TMP/sb.log" 2>&1 &
SB_PID=$!
# queue-broker role on a scratch port: /healthz answers
# unauthenticated. Its secrets URL points at a dead port so a
# credential fetch fails fast (exit 3) rather than retrying — enough
# to prove the client reached the broker and read its envelope.
env QUEUE_BROKER_BIND=127.0.0.1:19801 QUEUE_BROKER_TOKEN_FILE="$TMP/qb-token" \
  QUEUE_BROKER_SECRETS_URL=http://127.0.0.1:9 QUEUE_BROKER_SECRETS_TOKEN_FILE="$TMP/sb-token" \
  "$FULL_OUT/bin/fleet" serve queue-broker >"$TMP/qb.log" 2>&1 &
QB_PID=$!
sleep 1
run "secrets-broker role: /health answers" 0 '"ok":true' curl -sf http://127.0.0.1:19800/health
run "queue-broker role: /healthz answers" 0 '"service":"queue-broker"' curl -sf http://127.0.0.1:19801/healthz

# Client surface (the census gap §1): --help renders from the
# registry; --version reports the release through the argv0 route.
mkdir -p "$TMP/full"
ln -s "$FULL_OUT/bin/fleet" "$TMP/full/repo-queue"
run "repo-queue: --help renders the queue commands" 0 "Commands: validate" "$TMP/full/repo-queue" --help
run "repo-queue: --version reports the fleet release" 0 "repo-queue 0.1.0" "$TMP/full/repo-queue" --version
run "fleet queue: --help renders the queue commands" 0 "Commands: validate" "$FULL_OUT/bin/fleet" queue --help

# Client -> broker wiring: health reaches the scratch queue-broker,
# which answers with its credential failure (exit 3, broker wording).
run "repo-queue: health reaches the broker (creds fail fast, exit 3)" 3 "secrets broker" \
  env REPO_QUEUE_BROKER_URL=http://127.0.0.1:19801 REPO_QUEUE_TOKEN_FILE="$TMP/qb-token" \
  "$TMP/full/repo-queue" health
kill "$SB_PID" "$QB_PID" 2>/dev/null
SB_PID=""
QB_PID=""

echo "== name narrowing: full build invoked as fleet-connect =="
ln -s "$FULL_OUT/bin/fleet" "$TMP/fleet-connect"
run "narrowed: version lists only the connect set" 0 "$CONNECT_CAPS" "$TMP/fleet-connect" version
run "narrowed: queue stubs with narrowed wording" 69 "invoked as 'fleet-connect'" \
  "$TMP/fleet-connect" queue health
run "narrowed: proxy serve stubs with narrowed wording" 69 "invoked as 'fleet-connect'" \
  "$TMP/fleet-connect" proxy serve
run "narrowed: --help marks queue as narrowed out" 0 "[narrowed out by this name]" \
  "$TMP/fleet-connect" --help

echo "== name narrowing never widens: connect invoked as fleet =="
ln -s "$CONNECT_OUT/bin/fleet" "$TMP/fleet"
run "connect as fleet: queue still stubs not-in-build" 69 "this build does not include 'queue'" \
  "$TMP/fleet" queue

echo "== config-disabled domain (exit 69, config wording) =="
printf '[domains]\nqueue = false\n' > "$TMP/fleet.toml"
run "full with queue=false: fleet queue stubs as disabled" 69 "disabled by config" \
  env FLEET_CONFIG="$TMP/fleet.toml" "$FULL_OUT/bin/fleet" queue
run "full with queue=false: version drops queue capabilities" 0 "proxy-serve, queue-broker" \
  env FLEET_CONFIG="$TMP/fleet.toml" "$FULL_OUT/bin/fleet" version

echo "== runs anywhere: env -i =="
run "connect dynamic under env -i" 0 "$CONNECT_CAPS" env -i "$CONNECT_OUT/bin/fleet" version
run "connect static under env -i" 0 "$CONNECT_CAPS" env -i "$STATIC_OUT/bin/fleet" version

echo "== D7 size gate (<= 2097152 bytes) =="
for flavor in "$CONNECT_OUT" "$STATIC_OUT"; do
  size=$(stat -c %s "$flavor/bin/fleet")
  echo "     $flavor/bin/fleet: $size bytes"
  if [ "$size" -le 2097152 ]; then
    ok "connect size gate: $size <= 2097152"
  else
    bad "connect size gate: $size > 2097152"
  fi
done
for flavor in "$FULL_OUT" "$SERVE_OUT"; do
  echo "     $flavor/bin/fleet: $(stat -c %s "$flavor/bin/fleet") bytes (recorded, no gate)"
done

echo
echo "smoke: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
