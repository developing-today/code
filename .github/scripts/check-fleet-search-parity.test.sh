#!/usr/bin/env bash
# Contract tests for check-fleet-search-parity.sh (#5920): the guard
# passes the current tree, and fails on body drift in either copy, on
# a changed parity header, and on a removed parity header.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SCRIPT_DIR/check-fleet-search-parity.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Copy the four writer files into a scratch tree shaped like the repo.
make_tree() {
  local dest="$1"
  mkdir -p "$dest/pkgs/atlas-search/crates/atlas-search/src" \
    "$dest/pkgs/fleet/crates/fleet-search/src"
  local pair
  for pair in \
    "pkgs/atlas-search/crates/atlas-search/src/pyjson.rs" \
    "pkgs/atlas-search/crates/atlas-search/src/render.rs" \
    "pkgs/fleet/crates/fleet-search/src/pyjson.rs" \
    "pkgs/fleet/crates/fleet-search/src/render.rs"; do
    cp "$REPO_ROOT/$pair" "$dest/$pair"
  done
}

RC=0
run_guard() { # run_guard <root>
  RC=0
  bash "$GUARD" "$1" >"$TMP/out" 2>&1 || RC=$?
}

expect_pass() { # expect_pass <name>
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed the guard" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 passes"
}

expect_fail() { # expect_fail <name>
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed the guard" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 fails"
}

ATLAS_PY="$TMP/tree/pkgs/atlas-search/crates/atlas-search/src/pyjson.rs"
FLEET_PY="$TMP/tree/pkgs/fleet/crates/fleet-search/src/pyjson.rs"
FLEET_RENDER="$TMP/tree/pkgs/fleet/crates/fleet-search/src/render.rs"

# 1. The current tree, copied verbatim, passes.
make_tree "$TMP/tree"
run_guard "$TMP/tree"
expect_pass "verbatim copies"

# 2. A body change in the fleet copy only -> fail.
echo "// drift" >>"$FLEET_PY"
run_guard "$TMP/tree"
expect_fail "body drift in fleet pyjson.rs"

# 3. A body change in the atlas copy only -> fail.
make_tree "$TMP/tree"
sed -i 's/byte-identical/byte-identical-ish/' "$TMP/tree/pkgs/atlas-search/crates/atlas-search/src/render.rs" 2>/dev/null || true
printf '\n// drift\n' >>"$TMP/tree/pkgs/atlas-search/crates/atlas-search/src/render.rs"
run_guard "$TMP/tree"
expect_fail "body drift in atlas render.rs"

# 4. A changed fleet parity header -> fail.
make_tree "$TMP/tree"
sed -i '2s/Ported verbatim/Copied/' "$FLEET_RENDER"
run_guard "$TMP/tree"
expect_fail "changed fleet parity header"

# 5. A removed atlas parity header -> fail.
make_tree "$TMP/tree"
sed -i '2,5d' "$ATLAS_PY"
run_guard "$TMP/tree"
expect_fail "removed atlas parity header"

# 6. The mirrored change in BOTH copies -> pass (the contract is
# parity, not frozen content).
make_tree "$TMP/tree"
echo "// mirrored" >>"$ATLAS_PY"
echo "// mirrored" >>"$FLEET_PY"
run_guard "$TMP/tree"
expect_pass "mirrored change in both copies"

echo "check-fleet-search-parity: all contract tests pass (#5920)"
