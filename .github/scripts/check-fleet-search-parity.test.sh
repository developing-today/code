#!/usr/bin/env bash
# Contract tests for check-fleet-search-parity.sh (#5920): the guard
# passes the current tree, and fails on body drift in either copy, on
# a changed parity header, and on a removed parity header. The sole
# service-specific HOME diagnostic difference from #5640 is narrowly pinned.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SCRIPT_DIR/check-fleet-search-parity.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Copy the six guarded files into a scratch tree shaped like the repo.
make_tree() {
  local dest="$1"
  mkdir -p "$dest/pkgs/atlas-search/crates/atlas-search/src" \
    "$dest/pkgs/fleet/crates/fleet-search/src"
  local pair
  for pair in \
    "pkgs/atlas-search/crates/atlas-search/src/pyjson.rs" \
    "pkgs/atlas-search/crates/atlas-search/src/render.rs" \
    "pkgs/atlas-search/crates/atlas-search/src/server.rs" \
    "pkgs/fleet/crates/fleet-search/src/pyjson.rs" \
    "pkgs/fleet/crates/fleet-search/src/render.rs" \
    "pkgs/fleet/crates/fleet-search/src/server.rs"; do
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
expect_pass "current copies and pinned #5640 HOME diagnostic"

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

# 7. The rehomed service (#5604): body drift in either server.rs copy,
# and a changed server.rs header, fail; a mirrored change passes.
ATLAS_SRV="$TMP/tree/pkgs/atlas-search/crates/atlas-search/src/server.rs"
FLEET_SRV="$TMP/tree/pkgs/fleet/crates/fleet-search/src/server.rs"
make_tree "$TMP/tree"
echo "// drift" >>"$FLEET_SRV"
run_guard "$TMP/tree"
expect_fail "body drift in fleet server.rs"

make_tree "$TMP/tree"
echo "// drift" >>"$ATLAS_SRV"
run_guard "$TMP/tree"
expect_fail "body drift in atlas server.rs"

make_tree "$TMP/tree"
sed -i '2s/Rehomed verbatim/Copied/' "$FLEET_SRV"
run_guard "$TMP/tree"
expect_fail "changed fleet server.rs parity header"

make_tree "$TMP/tree"
echo "// mirrored" >>"$ATLAS_SRV"
echo "// mirrored" >>"$FLEET_SRV"
run_guard "$TMP/tree"
expect_pass "mirrored server.rs change in both copies"

# 8. Only the exact service-specific HOME diagnostic names are normalized.
make_tree "$TMP/tree"
sed -i 's/HOME is not set or is empty: fleet-search requires/HOME is not set or is empty: another-service requires/' "$FLEET_SRV"
run_guard "$TMP/tree"
expect_fail "unapproved server HOME wording"

make_tree "$TMP/tree"
sed -i '/HOME is not set or is empty: fleet-search requires/s|$| HOME is not set or is empty: fleet-search requires a non-empty HOME to resolve its state/config paths (issue #5640)|' "$FLEET_SRV"
run_guard "$TMP/tree"
expect_fail "duplicate pinned HOME diagnostic"
if ! grep -Fq 'expected exactly one #5640 HOME diagnostic' "$TMP/out"; then
  echo "FAIL: the exact-count guard did not reject a duplicate HOME diagnostic" >&2
  cat "$TMP/out" >&2
  exit 1
fi
echo "pass: duplicate HOME diagnostic rejected by exact-count guard"

echo "check-fleet-search-parity: all contract tests pass (#5920)"
