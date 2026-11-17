#!/usr/bin/env bash
# check-suite-registry.test.sh — contract tests for the coverage
# ratchet (#6188). Builds fixture trees in a temp dir and asserts:
# a consistent registry passes; an unwired on-disk suite fails
# naming it; a dangling registry path fails naming it; a venue that
# stops naming its suite fails naming it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-suite-registry.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail_count=0
expect_exit() {
  local want="$1" name="$2" pat="${3:-}"
  local out rc
  out="$(bash "$CHECK" "$FIXTURE" 2>&1)" && rc=0 || rc=$?
  if [ "$rc" -ne "$want" ]; then
    echo "FAIL $name: exit $rc, want $want"
    echo "$out" | head -5
    fail_count=$((fail_count + 1))
  elif [ "$want" -ne 0 ] && ! grep -q "$pat" <<<"$out"; then
    echo "FAIL $name: output does not name $pat"
    echo "$out" | head -5
    fail_count=$((fail_count + 1))
  else
    echo "pass $name"
  fi
}

FIXTURE="$TMP/repo"
mkdir -p "$FIXTURE/.github/workflows" "$FIXTURE/pkgs/id/tests" "$FIXTURE/pkgs/id/src"
cat >"$FIXTURE/.github/test-suite-registry.json" <<'JSON'
{
  "schema_version": 1,
  "scope": {"boundaries": [
    {"path": "pkgs/id/**", "decision": "in_scope"},
    {"path": "pkgs/id/vendor/**", "decision": "out_of_scope", "reason": "vendored"}
  ]},
  "excluded_suites": [],
  "waivers": [],
  "suites": [
    {"id": "id.rust.integration", "paths": ["pkgs/id/tests/*.rs"],
     "invocation": "bash scripts/test-shard-sandbox.sh",
     "venue": {"workflow": ".github/workflows/id-check.yml", "job_step": "basic: shard runs"},
     "status": "mapped"}
  ]
}
JSON
cat >"$FIXTURE/.github/workflows/id-check.yml" <<'YAML'
name: id-check
on: workflow_dispatch
jobs:
  checks:
    steps:
      - run: bash scripts/test-shard-sandbox.sh
YAML
touch "$FIXTURE/pkgs/id/tests/alpha.rs"

# 1. Consistent tree passes.
expect_exit 0 "consistent tree passes"

# 2. An unwired on-disk suite fails, naming the file.
touch "$FIXTURE/pkgs/id/tests/stray.rs"
mkdir -p "$FIXTURE/pkgs/id/crates/id-core/tests"
touch "$FIXTURE/pkgs/id/crates/id-core/tests/stray_test.rs"
expect_exit 1 "unwired suite fails" "stray_test.rs"
rm "$FIXTURE/pkgs/id/tests/stray.rs" "$FIXTURE/pkgs/id/crates/id-core/tests/stray_test.rs"

# 3. A dangling registry path fails, naming the suite.
python3 - "$FIXTURE" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1]) / ".github/test-suite-registry.json"
d = json.loads(p.read_text())
d["suites"][0]["paths"] = ["pkgs/id/tests/gone.rs"]
p.write_text(json.dumps(d))
PY
expect_exit 1 "dangling path fails" "id.rust.integration"
python3 - "$FIXTURE" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1]) / ".github/test-suite-registry.json"
d = json.loads(p.read_text())
d["suites"][0]["paths"] = ["pkgs/id/tests/*.rs"]
p.write_text(json.dumps(d))
PY

# 4. A venue that stops naming the suite fails, naming the suite.
cat >"$FIXTURE/.github/workflows/id-check.yml" <<'YAML'
name: id-check
on: workflow_dispatch
jobs:
  checks:
    steps:
      - run: echo nothing relevant here
YAML
expect_exit 1 "venue no longer naming suite fails" "id.rust.integration"

if [ "$fail_count" -ne 0 ]; then
  echo "check-suite-registry.test: $fail_count failure(s)"
  exit 1
fi
echo "check-suite-registry.test: all pass"
