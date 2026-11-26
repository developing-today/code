#!/usr/bin/env bash
# Contract tests for check-fleet-static-release.sh (#14999).
# Builds a fixture root holding copies of the real wiring (workflow,
# registries, keeper, publisher), proves the checker passes it, then
# breaks one leg at a time and proves the checker fails naming that
# leg. Path-only, so it runs offline in seconds.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$SCRIPT_DIR/.github/scripts/check-fleet-static-release.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

failures=0
note_fail() {
  echo "FAIL: $1"
  failures=$((failures + 1))
}

FIX="$TMP/fixture"
mkdir -p "$FIX/.github/workflows" "$FIX/doc" "$FIX/scripts/muse"
cp "$SCRIPT_DIR/.github/workflows/fleet-static-release.yml" "$FIX/.github/workflows/"
cp "$SCRIPT_DIR/.github/test-suite-registry.json" "$FIX/.github/"
cp "$SCRIPT_DIR/doc/fleet-schedule-registry.md" "$FIX/doc/"
cp "$SCRIPT_DIR/scripts/muse/fleet-binary-keeper.sh" "$FIX/scripts/muse/"
cp "$SCRIPT_DIR/scripts/muse/publish-fleet.sh" "$FIX/scripts/muse/"

if bash "$CHECK" "$FIX" >/dev/null 2>&1; then
  echo "ok: the shipped wiring passes"
else
  note_fail "the shipped wiring should pass"
fi

expect_leg_failure() { # expect_leg_failure <name> <needle>
  local name=$1 needle=$2
  if out=$(bash "$CHECK" "$FIX" 2>&1); then
    note_fail "$name: checker should fail"
  elif ! printf '%s' "$out" | grep -qF "$needle"; then
    note_fail "$name: failure should name the leg ($needle)"
    printf '%s\n' "$out" | sed 's/^/       /'
  else
    echo "ok: $name fails, naming the leg"
  fi
}

# Break leg 1: a push trigger appears.
cp "$FIX/.github/workflows/fleet-static-release.yml" "$TMP/wf.bak"
printf '  push:\n    branches: [main]\n' >>"$FIX/.github/workflows/fleet-static-release.yml"
expect_leg_failure "push trigger" "schedule + dispatch only"
cp "$TMP/wf.bak" "$FIX/.github/workflows/fleet-static-release.yml"

# Break leg 2: the workflow stops publishing through publish-fleet.sh.
sed -i 's|scripts/muse/publish-fleet.sh|scripts/muse/publish-fleet-copy.sh|' "$FIX/.github/workflows/fleet-static-release.yml"
expect_leg_failure "publisher drift" "publish-fleet.sh"
cp "$TMP/wf.bak" "$FIX/.github/workflows/fleet-static-release.yml"

# Break leg 3: the registry row goes missing.
python3 - "$FIX/.github/test-suite-registry.json" <<'PYEOF'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["suites"] = [s for s in d["suites"] if s["id"] != "fleet.static-release"]
json.dump(d, open(p, "w"))
PYEOF
expect_leg_failure "registry row" "fleet.static-release is missing"
cp "$SCRIPT_DIR/.github/test-suite-registry.json" "$FIX/.github/"

# Break leg 4: the schedule registry row vanishes.
grep -v 'fleet-static-release' "$SCRIPT_DIR/doc/fleet-schedule-registry.md" >"$FIX/doc/fleet-schedule-registry.md"
expect_leg_failure "schedule registry row" "no fleet-static-release row"
cp "$SCRIPT_DIR/doc/fleet-schedule-registry.md" "$FIX/doc/"

# Break leg 5: the keeper disappears.
rm "$FIX/scripts/muse/fleet-binary-keeper.sh"
expect_leg_failure "keeper missing" "required file missing: scripts/muse/fleet-binary-keeper.sh"

if [ "$failures" -ne 0 ]; then
  echo "check-fleet-static-release.test: $failures failure(s)"
  exit 1
fi
echo "check-fleet-static-release.test: all cases passed"
