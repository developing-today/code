#!/usr/bin/env bash
# Per-variant smoke suite for the fleet workspace skeleton
# (step 1, developing-today/code#5429).
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
trap 'rm -rf "$TMP"' EXIT

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
run "serve as queue-broker reaches the (unimplemented) broker role" 69 "not implemented yet" "$TMP/queue-broker"

echo "== compiled in but not implemented yet (exit 69) =="
run "full: fleet proxy connect is the skeleton stub" 69 "not implemented yet" \
  "$FULL_OUT/bin/fleet" proxy connect
run "connect: fleet proxy connect is the skeleton stub" 69 "not implemented yet" \
  "$CONNECT_OUT/bin/fleet" proxy connect

echo "== name narrowing: full build invoked as fleet-connect =="
ln -s "$FULL_OUT/bin/fleet" "$TMP/fleet-connect"
run "narrowed: version lists only the connect set" 0 "$CONNECT_CAPS" "$TMP/fleet-connect" version
run "narrowed: queue stubs with narrowed wording" 69 "invoked as 'fleet-connect'" \
  "$TMP/fleet-connect" queue health
run "narrowed: proxy connect still reaches the skeleton stub" 69 "not implemented yet" \
  "$TMP/fleet-connect" proxy connect
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
