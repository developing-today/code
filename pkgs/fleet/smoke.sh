#!/usr/bin/env bash
# Per-variant smoke suite for the fleet workspace
# (step 1, developing-today/code#5429; queue-domain sections added
# by step 2, #5430; proxy-domain sections by step 3, #5431;
# dispatch-domain sections by step 4, #5432; search-domain
# sections by step 5, #5433).
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
#     idle exit, pool default rendering + a 32-concurrent burst
#     on the default pool config (pool fix, #5563)
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
DS_PID=""
SECB_PID=""
SRCH_PID=""
trap 'kill $SB_PID $QB_PID $PB_PID $UP_PID $IDLE_PID $DS_PID $SECB_PID $SRCH_PID 2>/dev/null; rm -rf "$TMP"' EXIT

ok() {
  PASS=$((PASS + 1))
  echo "ok   - $1"
}
bad() {
  FAIL=$((FAIL + 1))
  echo "FAIL - $1"
}

# run <name> <expected-rc> <required-substring|-> <cmd...>
run() {
  local name=$1 want_rc=$2 want_sub=$3
  shift 3
  local out rc
  out=$("$@" 2>&1)
  rc=$?
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

FULL_CAPS="Enabled capabilities: proxy-connect, proxy-serve, queue, queue-broker, secrets-broker, search, search-serve, dispatch, laptop-agent, secrets"
SERVE_CAPS="Enabled capabilities: proxy-connect, proxy-serve, queue-broker, secrets-broker, search-serve, secrets"
CONNECT_CAPS="Enabled capabilities: proxy-connect"

check_variant() {
  local label=$1 bin=$2 caps=$3
  run "$label: --help renders the full registry" 0 "laptop-agent" "$bin" --help
  run "$label: version capability set is exact" 0 "$caps" "$bin" version
  run "$label: capabilities is valid JSON with the build block" 0 '"source_rev"' "$bin" capabilities
}

echo "== variants =="
check_variant full "$FULL_OUT/bin/fleet" "$FULL_CAPS"
check_variant serve "$SERVE_OUT/bin/fleet" "$SERVE_CAPS"
check_variant connect "$CONNECT_OUT/bin/fleet" "$CONNECT_CAPS"
check_variant static "$STATIC_OUT/bin/fleet" "$CONNECT_CAPS"

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
run "serve: fleet search (client) stubs — serve carries search-serve only" 69 \
  "this build does not include 'search'" \
  "$SERVE_OUT/bin/fleet" search
run "connect: fleet secrets stubs" 69 "this build does not include 'secrets'" \
  "$CONNECT_OUT/bin/fleet" secrets get smoke.secret

echo "== stubs via argv0 alias symlinks =="
ln -s "$CONNECT_OUT/bin/fleet" "$TMP/repo-queue"
ln -s "$CONNECT_OUT/bin/fleet" "$TMP/atlas-search"
ln -s "$SERVE_OUT/bin/fleet" "$TMP/queue-broker"
run "connect as repo-queue stubs 69" 69 "this build does not include 'queue'" "$TMP/repo-queue"
run "connect as atlas-search stubs 69" 69 "this build does not include 'search'" "$TMP/atlas-search"
run "serve as queue-broker reaches the real broker role (missing token file: exit 3)" 3 "cannot read token file" \
  env QUEUE_BROKER_TOKEN_FILE="$TMP/no-such-token" "$TMP/queue-broker"

echo "== search serve role (#5604): reaches the real service role, no stub =="
# Hermetic: the service config path does not exist, so the role stops
# at config load (exit 2) before any Turso token, embedder or socket.
run "connect: fleet serve search stubs" 69 "this build does not include 'serve'" \
  "$CONNECT_OUT/bin/fleet" serve search
run "serve: fleet serve search reaches the real role (missing config: exit 2)" 2 "cannot read service config" \
  env ATLAS_SEARCH_SERVICE_CONFIG="$TMP/no-such-service.json" "$SERVE_OUT/bin/fleet" serve search
run "full: fleet serve search --help prints the role usage" 0 "usage: fleet serve search" \
  "$FULL_OUT/bin/fleet" serve search --help
mkdir -p "$TMP/asp" && ln -sf "$SERVE_OUT/bin/fleet" "$TMP/asp/atlas-search"
run "serve as atlas-search: 'serve' is the same role (the rollback spelling)" 2 "cannot read service config" \
  env ATLAS_SEARCH_SERVICE_CONFIG="$TMP/no-such-service.json" "$TMP/asp/atlas-search" serve

echo "== proxy domain: serve + connect + init + shims (step 3, #5431) =="
# Hermetic proxy world: a scratch HOME (token file), scratch socket,
# scratch state dirs, and a stub HTTP upstream on loopback ports. The
# broker runs with direct = true, so no CONNECT tunnel is involved;
# the tunnel path itself is exercised against the real system in the
# step's live proofs, not in per-variant smoke.
PX="$TMP/proxy"
mkdir -p "$PX/home/tools" "$PX/state" "$PX/state-connect"
printf 'smoke-broker-token\n' >"$PX/home/tools/laptop-agent-token"

cat >"$PX/upstream.py" <<'PYEOF'
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

cat >"$PX/fleet.toml" <<'EOF'
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
PB_PID=$(pgrep -f "bin/fleet (proxy serve|serve proxy)" | head -1)
# The manifest fixture is the hermetic vehicle for the init cases
# (design #5687 §6 step 5): FLEET_HOST_CONFIG carries the sandbox
# [host] section inline, newline-separated TOML.
HOSTCFG_SANDBOX=$(printf 'location="sandbox"\ninit_mode="spawn"\n[roles]\nproxy={}')
run "proxy: fleet init is a no-op when the broker runs" 0 "already running" \
  proxy_env env FLEET_HOST_CONFIG="$HOSTCFG_SANDBOX" "$FULL_OUT/bin/fleet" init
if [ -n "$PB_PID" ] && kill -0 "$PB_PID" 2>/dev/null && [ "$(pgrep -f 'bin/fleet (proxy serve|serve proxy)' | wc -l)" -eq 1 ]; then
  ok "proxy: init left the same single broker alive"
else
  bad "proxy: init left the same single broker alive"
fi

echo "== location-aware init (#6770, design #5687 §§4-6) =="
PXU="$TMP/init-location"
mkdir -p "$PXU/home" "$PXU/state"
printf '[proxy]\ndirect = true\n' >"$PXU/fleet.toml"
# Unknown location: no manifest from any source (empty env values
# count as unset) — report, exit 69, and no broker is spawned.
run "init: unknown location reports and starts nothing" 69 "starting nothing" \
  env HOME="$PXU/home" FLEET_CONFIG="$PXU/fleet.toml" \
  FLEET_BROKER_SOCKET="$PXU/broker.sock" XDG_STATE_HOME="$PXU/state" \
  FLEET_HOST_CONFIG= FLEET_LOCATION= \
  "$FULL_OUT/bin/fleet" init
if [ ! -e "$PXU/broker.sock" ]; then
  ok "init: unknown location spawned no broker"
else
  bad "init: unknown location spawned no broker"
fi
# A laptop manifest on this host: systemd mode reports each role
# and its unit state; nothing is detached-spawned, exit 0.
HOSTCFG_LAPTOP=$(printf 'location="laptop"\ninit_mode="systemd"\n[roles]\nqueue-broker={}\nsecrets-broker={}\nsearch={}\nlaptop-agent={}')
run "init: laptop manifest reports its location" 0 "location laptop" \
  env HOME="$PXU/home" FLEET_CONFIG="$PXU/fleet.toml" \
  FLEET_BROKER_SOCKET="$PXU/broker.sock" XDG_STATE_HOME="$PXU/state" \
  FLEET_HOST_CONFIG="$HOSTCFG_LAPTOP" \
  "$FULL_OUT/bin/fleet" init
run "init: laptop manifest reports role lines" 0 "role queue-broker:" \
  env HOME="$PXU/home" FLEET_CONFIG="$PXU/fleet.toml" \
  FLEET_BROKER_SOCKET="$PXU/broker.sock" XDG_STATE_HOME="$PXU/state" \
  FLEET_HOST_CONFIG="$HOSTCFG_LAPTOP" \
  "$FULL_OUT/bin/fleet" init
if [ ! -e "$PXU/broker.sock" ]; then
  ok "init: laptop manifest spawned no broker"
else
  bad "init: laptop manifest spawned no broker"
fi
# The serve spelling of the proxy role routes (auto-start uses it).
run "serve: fleet serve proxy --help prints the role usage" 0 "Usage: fleet serve proxy" \
  "$FULL_OUT/bin/fleet" serve proxy --help

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
cat >"$PX/legacy.py" <<'PYEOF'
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

# Pool sizing (#5563): this broker was auto-spawned by connect
# with no LAPTOP_AGENT_POOL anywhere in its environment, so
# its startup line must render the compiled default of 50.
run "proxy: broker startup renders the effective pool size (default 50)" 0 \
  "pool_size=50" grep "pool_size=50" "$PX/broker.log"
# Identity stamp (#5758): the same first startup line names the
# implementation and carries a UTC timestamp + pid, so broker
# ownership of an incident window is reconstructable from the log.
run "proxy: broker startup line is timestamped with implementation + pid" 0 \
  "implementation=fleet-proxy" grep -E "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]+Z fleet proxy serve start implementation=fleet-proxy version=[^ ]+ pool_size=50 pid=[0-9]+" "$PX/broker.log"

# 32-concurrent burst against that default-config broker,
# mirroring the #5412 benchmark method (hello-negotiated
# framed requests, one untimed warm-up per worker, barrier
# release, two timed requests each). Hermetically the
# connections are loopback, so this asserts the bounded
# wait's safety properties — 32 in flight at once, none hang,
# none fail — plus a sane p50; the production cliff it
# prevents was measured against real egress (see #5563).
cat >"$PX/burst.py" <<'PYEOF'
import json, socket, struct, sys, threading, time
sock_path, port = sys.argv[1], int(sys.argv[2])
lat, errors = [], []
lock = threading.Lock()
barrier = threading.Barrier(33)
def readn(s, n):
    buf = b""
    while len(buf) < n:
        chunk = s.recv(n - len(buf))
        if not chunk: raise ConnectionError("broker closed mid-frame")
        buf += chunk
    return buf
def send_frame(s, obj):
    data = json.dumps(obj).encode()
    s.sendall(struct.pack(">Q", len(data)) + data)
def read_frame(s):
    (n,) = struct.unpack(">Q", readn(s, 8))
    return json.loads(readn(s, n))
def rpc(s, method, path, sport):
    send_frame(s, {"method": method, "path": path, "port": sport})
    return read_frame(s)
def worker():
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(30)
        s.connect(sock_path)
        send_frame(s, {"hello": {"proto": 1, "client": "smoke-burst",
                                 "capabilities": ["proxy-connect"]}})
        read_frame(s)  # hello response
        r = rpc(s, "GET", "/health", port)  # untimed warm-up
        assert r["status"] == 200, r
        barrier.wait(timeout=30)
        for _ in range(2):
            t0 = time.perf_counter()
            r = rpc(s, "GET", "/health", port)
            dt = (time.perf_counter() - t0) * 1000
            assert r["status"] == 200, r
            with lock: lat.append(dt)
        s.close()
    except Exception as e:
        with lock: errors.append(repr(e))
threads = [threading.Thread(target=worker) for _ in range(32)]
for t in threads: t.start()
barrier.wait(timeout=30)
for t in threads: t.join()
assert not errors, f"burst errors: {errors[:3]}"
assert len(lat) == 64, len(lat)
lat.sort()
p50 = lat[len(lat) // 2]
assert p50 < 1000, f"p50 {p50:.1f}ms"
print(f"burst-ok n=64 errors=0 p50={p50:.1f}ms")
PYEOF
run "proxy: 32-concurrent burst on the default pool (no hang, no failure)" 0 "burst-ok" \
  python3 "$PX/burst.py" "$PX/broker.sock" 14801

# Services-map change: restart the broker with a smaller map; the
# repo-queue shim must disappear, laptop-agent must stay.
kill "$PB_PID" 2>/dev/null
sleep 0.5
rm -f "$PX/broker.sock"
cat >"$PX/fleet.toml" <<'EOF'
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
PB_PID=$(pgrep -f "bin/fleet (proxy serve|serve proxy)" | head -1)
kill "$PB_PID" 2>/dev/null
PB_PID=""

# Idle exit: a broker with an 8-second idle span exits on its own
# even though its keepalive loop is running (keepalive != use).
mkdir -p "$PX/idle"
cat >"$PX/idle.toml" <<'EOF'
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
for i in $(seq 1 40); do
  kill -0 "$IDLE_PID" 2>/dev/null || break
  sleep 1
done
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
printf 'smoke-secrets-token\n' >"$TMP/sb-token"
printf 'smoke-queue-token\n' >"$TMP/qb-token"

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

echo "== dispatch domain: thin client over a stub pm-dispatch (step 4, #5432) =="
# Hermetic dispatch world: a stub pm-dispatch on a scratch loopback
# port (health unauthenticated; POSTs bearer-checked, with canned
# answers and one deliberate 409) and a stub `bws` on PATH, so the
# client's by-name token resolution through fleet-secrets succeeds
# with no vault and no session broker (the scratch HOME has no
# broker socket, so the chain falls through to the bws source).
# The client must resolve the token by name, present it as
# Authorization: Bearer, and print the service's JSON verbatim.
DS="$TMP/dispatch"
mkdir -p "$DS/bin" "$DS/home"
cat >"$DS/stub.py" <<'PYEOF'
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
TOKEN = "smoke-dispatch-token"
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        if self.path == "/health":
            self._send(200, {"ok": True, "status": "ok", "version": "smoke"})
        else:
            self._send(404, {"error": "not found"})
    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b"{}"
        if self.headers.get("Authorization") != "Bearer " + TOKEN:
            self._send(401, {"error": "unauthorized"})
            return
        try:
            req = json.loads(raw)
        except Exception:
            req = {}
        if self.path == "/v1/sessions":
            self._send(201, {"task_id": 4242, "session_id": "ses_smoke",
                             "issue_number": None,
                             "queue": "repo-health-code-pilot",
                             "status": "claimed"})
        elif self.path == "/v1/tasks":
            if req.get("dedupe_key") == "smoke-dupe":
                self._send(409, {"error": "dedupe_key already has an open task"})
                return
            self._send(201, {"task_id": 4243, "issue_number": None,
                             "queue": "repo-health-code-pilot"})
        else:
            self._send(404, {"error": "not found"})
HTTPServer(("127.0.0.1", 14879), H).serve_forever()
PYEOF
python3 "$DS/stub.py" >"$DS/stub.log" 2>&1 &
DS_PID=$!
cat >"$DS/bin/bws" <<'EOF'
#!/usr/bin/env bash
# fleet-secrets' bws fallback, stubbed: one secret under a fixed id.
if [ "$1 $2" = "secret list" ]; then
  echo '[{"id":"smoke-id","key":"pm-dispatch.api-token"}]'
elif [ "$1 $2" = "secret get" ]; then
  echo '{"id":"smoke-id","key":"pm-dispatch.api-token","value":"smoke-dispatch-token"}'
else
  exit 1
fi
EOF
chmod +x "$DS/bin/bws"
printf '[dispatch]\nport = 14879\n' >"$DS/fleet.toml"
sleep 1
run "dispatch: status round-trips the stub /health" 0 '"version": "smoke"' \
  env HOME="$DS/home" PATH="$DS/bin:$PATH" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch status
run "dispatch: status names its transport" 0 '"transport": "direct http://127.0.0.1:14879"' \
  env HOME="$DS/home" PATH="$DS/bin:$PATH" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch status
run "dispatch: launch round-trips with the by-name bearer token" 0 '"task_id": 4242' \
  env HOME="$DS/home" PATH="$DS/bin:$PATH" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch launch --repo developing-today/code \
  --prompt "smoke launch" --no-wait
run "dispatch: enqueue round-trips" 0 '"task_id": 4243' \
  env HOME="$DS/home" PATH="$DS/bin:$PATH" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch enqueue --repo developing-today/code \
  --title "smoke" --prompt "smoke enqueue"
run "dispatch: a service HTTP error exits 4 with its message" 4 "dedupe_key already has an open task" \
  env HOME="$DS/home" PATH="$DS/bin:$PATH" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch enqueue --repo developing-today/code \
  --title "smoke" --prompt "x" --dedupe-key smoke-dupe
run "dispatch: an unknown task kind is a usage error" 2 "unknown task kind" \
  env HOME="$DS/home" PATH="$DS/bin:$PATH" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch enqueue --repo developing-today/code \
  --title "t" --prompt "p" --kind bogus
run "dispatch: an unresolved token name exits 3 before any request" 3 "unresolved" \
  env HOME="$DS/home" PATH="/nonexistent" FLEET_CONFIG="$DS/fleet.toml" \
  "$FULL_OUT/bin/fleet" dispatch launch --repo developing-today/code --prompt "x"
kill "$DS_PID" 2>/dev/null
DS_PID=""

echo "== secrets domain: broker-first chain, bws fallback (step 6, #5554) =="
# Hermetic secrets world: a stub session broker speaking the framed
# protocol on a scratch socket (it answers /secrets and /secret/<name>
# itself — the BrokerSource exchange in the legacy dialect, no hello),
# plus a stub `bws` on PATH whose value for the shared name DIFFERS
# from the broker's, so the source that answered is provable from the
# returned value alone.
SEC="$TMP/secrets"
mkdir -p "$SEC/bin" "$SEC/home"
cat >"$SEC/broker.py" <<'SECEOF'
import socket, struct, json, sys, threading, os
sock_path = sys.argv[1]
RESP = {
    "/secrets": (200, {"names": ["other.name", "smoke.secret"]}),
    "/secret/smoke.secret": (200, {"key": "smoke.secret", "value": "smoke-broker-value"}),
}
def frame(obj):
    payload = json.dumps(obj).encode()
    return struct.pack(">Q", len(payload)) + payload
def readn(conn, n):
    buf = b""
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk: raise EOFError
        buf += chunk
    return buf
def handle(conn):
    try:
        while True:
            n = struct.unpack(">Q", readn(conn, 8))[0]
            req = json.loads(readn(conn, n))
            status, body = RESP.get(req["path"], (404, {"error": "no such secret"}))
            conn.sendall(frame({"status": status, "body": body}))
    except Exception:
        return
srv = socket.socket(socket.AF_UNIX)
if os.path.exists(sock_path): os.unlink(sock_path)
srv.bind(sock_path); srv.listen(16)
while True:
    conn, _ = srv.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
SECEOF
python3 "$SEC/broker.py" "$SEC/broker.sock" >"$SEC/broker.log" 2>&1 &
SECB_PID=$!
cat >"$SEC/bin/bws" <<'EOF'
#!/usr/bin/env bash
# fleet-secrets' bws fallback, stubbed: two secrets under fixed ids.
# smoke.secret exists in BOTH sources with different values (the
# broker must win); fallback.name exists only here.
if [ "$1 $2" = "secret list" ]; then
  echo '[{"id":"id-1","key":"smoke.secret"},{"id":"id-2","key":"fallback.name"}]'
elif [ "$1 $2" = "secret get" ] && [ "$3" = "id-1" ]; then
  echo '{"id":"id-1","key":"smoke.secret","value":"smoke-bws-value"}'
elif [ "$1 $2" = "secret get" ] && [ "$3" = "id-2" ]; then
  echo '{"id":"id-2","key":"fallback.name","value":"smoke-bws-fallback-value"}'
else
  exit 1
fi
EOF
chmod +x "$SEC/bin/bws"
sleep 1
secrets_env() {
  env HOME="$SEC/home" PATH="$SEC/bin:$PATH" FLEET_BROKER_SOCKET="$SEC/broker.sock" "$@"
}
run "secrets: --help renders from the registry" 0 "Commands: get, list" \
  "$FULL_OUT/bin/fleet" secrets --help
run "secrets: get resolves broker-first (broker value, not bws')" 0 "smoke-broker-value" \
  secrets_env "$FULL_OUT/bin/fleet" secrets get smoke.secret
run "secrets: list returns the broker's names" 0 "smoke.secret" \
  secrets_env "$FULL_OUT/bin/fleet" secrets list
run "secrets: a broker 404 falls back to bws" 0 "smoke-bws-fallback-value" \
  secrets_env "$FULL_OUT/bin/fleet" secrets get fallback.name
kill "$SECB_PID" 2>/dev/null
SECB_PID=""
rm -f "$SEC/broker.sock"
run "secrets: broker down, get falls back to bws" 0 "smoke-bws-value" \
  secrets_env "$FULL_OUT/bin/fleet" secrets get smoke.secret
run "secrets: broker down, list fails loud (no bws substitution)" 3 "unavailable via secrets-broker" \
  secrets_env "$FULL_OUT/bin/fleet" secrets list
run "secrets: unknown name fails clean through both legs" 3 "unresolved" \
  secrets_env "$FULL_OUT/bin/fleet" secrets get absent.name

echo "== search domain: via-service client over a stub atlas service (step 5, #5433) =="
# Hermetic search world: a stub atlas service speaking the job
# protocol (atlas-proto shapes) on a scratch loopback port. The
# stub is the service-parity oracle for this suite: it owns a
# canonical ranking per effective ceiling, implements the
# server-side clamp (token cap ∧ requested ceiling), and plants
# the step-6 canary — chunk 537, secret-bearing, rank 1 only when
# the effective ceiling is secret-bearing. The client's contract
# is to reproduce the stub's ranking exactly and to surface the
# clamp; the LIVE oracle (the real service vs the real
# atlas-search) is exercised in the step's live proofs on #5433.
# A stub `atlas-search` on PATH stands in for the kept standalone
# artifact: delegation cases assert fleet hands it the caller's
# arguments in atlas-search's own shape.
SRCH="$TMP/search"
mkdir -p "$SRCH/bin" "$SRCH/home" "$SRCH/home-laptop/.local/state/atlas-search"
touch "$SRCH/home-laptop/.local/state/atlas-search/service.json"
cat >"$SRCH/stub.py" <<'PYEOF'
import json, re
from http.server import BaseHTTPRequestHandler, HTTPServer
CAPS = {"smoke-personal-token": "personal", "smoke-secret-token": "secret-bearing"}
LADDER = ["shareable", "personal", "local-only", "secret-bearing"]
def clamp(requested, cap):
    return LADDER[min(LADDER.index(requested), LADDER.index(cap))]
def ranking(effective):
    ids = [537, 11, 12] if effective == "secret-bearing" else [11, 12, 13]
    return [{"chunk_id": i, "rank": n + 1} for n, i in enumerate(ids)]
JOBS = {}
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        if self.path == "/health":
            self._send(200, {"status": "ok", "embedder_state": "ready",
                             "queue_depth": 0, "active_branch_executions": 0,
                             "active_jobs": 0})
            return
        m = re.match(r"^/v1/search/jobs/([^/]+)$", self.path)
        if m and m.group(1) in JOBS:
            eff, _query = JOBS[m.group(1)]
            self._send(200, {"job_id": m.group(1), "status": "done",
                             "branch_states": {"fts": "done", "vector": "done"},
                             "effective_ceiling": eff, "pool": 100, "notes": [],
                             "submitted_at_ms": 0, "expires_at_ms": 999999999})
            return
        m = re.match(r"^/v1/search/jobs/([^/]+)/results", self.path)
        if m and m.group(1) in JOBS:
            eff, query = JOBS[m.group(1)]
            results = ranking(eff)
            envelope = {"query": query, "mode": "hybrid",
                        "filters": {"max_sensitivity": eff}, "results": []}
            self._send(200, {"job_id": m.group(1), "status": "done",
                             "effective_ceiling": eff, "envelope": envelope,
                             "results": results, "next_cursor": None,
                             "total_results": len(results)})
            return
        self._send(404, {"error": {"code": "not_found", "message": "no such route"}})
    def do_POST(self):
        if self.path != "/v1/search/jobs":
            self._send(404, {"error": {"code": "not_found", "message": "no such route"}})
            return
        token = (self.headers.get("Authorization") or "").removeprefix("Bearer ")
        cap = CAPS.get(token)
        if cap is None:
            self._send(401, {"error": {"code": "unauthorized", "message": "unknown client"}})
            return
        length = int(self.headers.get("Content-Length") or 0)
        req = json.loads(self.rfile.read(length) or b"{}")
        eff = clamp(req.get("ceiling", "personal"), cap)
        JOBS["job-smoke"] = (eff, req.get("query", ""))
        self._send(202, {"job_id": "job-smoke", "job_token": "job-token",
                         "status": "queued", "effective_ceiling": eff,
                         "queue_depth": 0, "submitted_at_ms": 0,
                         "expires_at_ms": 999999999})
HTTPServer(("127.0.0.1", 14877), H).serve_forever()
PYEOF
python3 "$SRCH/stub.py" >"$SRCH/stub.log" 2>&1 &
SRCH_PID=$!
cat >"$SRCH/bin/atlas-search" <<'EOF'
#!/bin/sh
# The kept standalone artifact, stubbed: echo the argv it was
# handed so delegation is provable from the output alone.
echo "stub-atlas-search: $*"
EOF
chmod +x "$SRCH/bin/atlas-search"
sleep 1
search_env() {
  env HOME="$SRCH/home" PATH="$SRCH/bin:$PATH" \
    ATLAS_SEARCH_URL=http://127.0.0.1:14877 "$@"
}
# Ceiling invariant, personal client: requesting secret-bearing is
# clamped by the server; the canary chunk is absent and the
# applied filter says personal.
search_env ATLAS_SEARCH_TOKEN=smoke-personal-token \
  "$FULL_OUT/bin/fleet" search "smoke query" --max-sensitivity secret-bearing \
  >"$SRCH/personal.out" 2>"$SRCH/personal.err"
if python3 -c "
import json, sys
doc = json.load(open('$SRCH/personal.out'))
ids = [r['chunk_id'] for r in doc['results']]
assert ids == [11, 12, 13], ids
assert doc['filters']['max_sensitivity'] == 'personal', doc['filters']
" && ! grep -q '"chunk_id": 537' "$SRCH/personal.out"; then
  ok "search: personal client is clamped; canary 537 absent; ranking is the service's"
else
  bad "search: personal client is clamped; canary 537 absent; ranking is the service's"
  head -5 "$SRCH/personal.out" | sed 's/^/       /'
fi
# Ceiling, secret-bearing client (drew-cli's shape): the canary
# leads the ranking.
search_env ATLAS_SEARCH_TOKEN=smoke-secret-token \
  "$FULL_OUT/bin/fleet" search "smoke query" --max-sensitivity secret-bearing \
  >"$SRCH/secret.out" 2>"$SRCH/secret.err"
if python3 -c "
import json
doc = json.load(open('$SRCH/secret.out'))
ids = [r['chunk_id'] for r in doc['results']]
assert ids == [537, 11, 12], ids
assert doc['filters']['max_sensitivity'] == 'secret-bearing', doc['filters']
"; then
  ok "search: secret-bearing client sees the canary at rank 1"
else
  bad "search: secret-bearing client sees the canary at rank 1"
  head -5 "$SRCH/secret.out" | sed 's/^/       /'
fi
# The atlas-search argv0 route lands on the same client.
ln -s "$FULL_OUT/bin/fleet" "$SRCH/atlas-search"
search_env ATLAS_SEARCH_TOKEN=smoke-personal-token \
  "$SRCH/atlas-search" search "smoke query" >"$SRCH/argv0.out" 2>/dev/null
if python3 -c "
import json
doc = json.load(open('$SRCH/argv0.out'))
assert [r['chunk_id'] for r in doc['results']] == [11, 12, 13]
"; then
  ok "search: argv0 atlas-search route reproduces the service ranking"
else
  bad "search: argv0 atlas-search route reproduces the service ranking"
fi
# get-chunk delegates to the standalone artifact, arguments intact.
run "search: get-chunk delegates to the standalone artifact" 0 \
  "stub-atlas-search: get-chunk 42 --neighbors 2" \
  search_env ATLAS_SEARCH_TOKEN=smoke-personal-token \
  "$FULL_OUT/bin/fleet" search get-chunk 42 --neighbors 2
# Laptop seat (the serving host's marker exists): standalone is
# the default route.
run "search: laptop seat defaults to standalone (delegates)" 0 \
  "stub-atlas-search: search laptop query" \
  env HOME="$SRCH/home-laptop" PATH="$SRCH/bin:$PATH" \
  "$FULL_OUT/bin/fleet" search "laptop query"
# Sandbox seat, service down: the fallback is LOUD and standalone
# still serves the query.
run "search: service down falls back loudly to standalone" 0 \
  "falling back to standalone" \
  env HOME="$SRCH/home" PATH="$SRCH/bin:$PATH" \
  ATLAS_SEARCH_URL=http://127.0.0.1:9 \
  "$FULL_OUT/bin/fleet" search "fallback query"
kill "$SRCH_PID" 2>/dev/null
SRCH_PID=""

echo "== name narrowing: full build invoked as fleet-connect =="
ln -s "$FULL_OUT/bin/fleet" "$TMP/fleet-connect"
run "narrowed: version lists only the connect set" 0 "$CONNECT_CAPS" "$TMP/fleet-connect" version
run "narrowed: queue stubs with narrowed wording" 69 "invoked as 'fleet-connect'" \
  "$TMP/fleet-connect" queue health
run "narrowed: proxy serve stubs with narrowed wording" 69 "invoked as 'fleet-connect'" \
  "$TMP/fleet-connect" proxy serve
run "narrowed: secrets stubs with narrowed wording" 69 "invoked as 'fleet-connect'" \
  "$TMP/fleet-connect" secrets get smoke.secret
run "narrowed: --help marks queue as narrowed out" 0 "[narrowed out by this name]" \
  "$TMP/fleet-connect" --help

echo "== name narrowing never widens: connect invoked as fleet =="
ln -s "$CONNECT_OUT/bin/fleet" "$TMP/fleet"
run "connect as fleet: queue still stubs not-in-build" 69 "this build does not include 'queue'" \
  "$TMP/fleet" queue

echo "== config-disabled domain (exit 69, config wording) =="
printf '[domains]\nqueue = false\n' >"$TMP/fleet.toml"
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
