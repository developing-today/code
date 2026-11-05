#!/usr/bin/env bash
# Tests for the roc-cap-demo example (#5957): a demo library (Greeter,
# in lib/) that depends on a platform capability (records.read, via the
# platform's RocCap module), a demo app (main.roc) importing it, and
# BOTH negative tests the issue requires:
#
#   A. main-missing-cap.roc — the same app + library compiled against
#      the reduced platform (roc-world/platform-no-caps.roc), which
#      does not provide RocCap. Must FAIL, and the failure must name
#      the missing pf.RocCap module.
#   B. main-wrong-usage.roc — the app against the full platform, but
#      calling RocCap.schedule_at_request with a wrong signature.
#      Must FAIL, and the failure must be a type mismatch at that call.
#
# The positive path (library expect tests + app typecheck) runs first.
# Invoked from scripts/roc-test.sh (the roc-test just recipe / flake
# check), so these tests are part of the normal test flow and cannot
# silently rot. A negative test that starts compiling, or that fails
# for a different reason, fails this script.
set -euo pipefail

cd "$(dirname "$0")"

if ! command -v roc >/dev/null 2>&1; then
  echo "roc-cap-demo test: the pinned Roc compiler is not on PATH; refusing to skip silently." >&2
  exit 1
fi

fail() { echo "roc-cap-demo test FAILED: $*" >&2; exit 1; }

echo "=== roc-cap-demo: library expect tests (roc test lib/main.roc) ==="
roc test lib/main.roc

echo "=== roc-cap-demo: demo app typechecks (roc check main.roc) ==="
roc check main.roc

# The negative-A package binds a byte-identical copy of the library to
# the reduced platform (Roc resolves package modules only inside the
# package root, so a symlink out of lib-no-caps/ is not allowed). Keep
# the copy honest: any drift between the two copies fails here.
diff lib/Greeter.roc lib-no-caps/Greeter.roc >/dev/null \
  || fail "lib-no-caps/Greeter.roc has drifted from lib/Greeter.roc; re-copy it"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "=== roc-cap-demo: negative test A (capability not on the platform) ==="
if roc check main-missing-cap.roc >"$TMP/neg-a.log" 2>&1; then
  fail "main-missing-cap.roc compiled, but the reduced platform must not satisfy the library"
fi
grep -q "pf.RocCap" "$TMP/neg-a.log" \
  || { cat "$TMP/neg-a.log" >&2; fail "negative A failed, but not over the missing pf.RocCap module"; }
grep -q "does not exist" "$TMP/neg-a.log" \
  || { cat "$TMP/neg-a.log" >&2; fail "negative A failed, but the error does not say pf.RocCap is missing"; }
echo "negative A failed as required: pf.RocCap is not provided by the reduced platform"

echo "=== roc-cap-demo: negative test B (platform used wrong) ==="
if roc check main-wrong-usage.roc >"$TMP/neg-b.log" 2>&1; then
  fail "main-wrong-usage.roc compiled, but the wrong-signature capability call must not typecheck"
fi
grep -q "schedule_at_request" "$TMP/neg-b.log" \
  || { cat "$TMP/neg-b.log" >&2; fail "negative B failed, but not at the schedule_at_request call"; }
grep -qi "type mismatch" "$TMP/neg-b.log" \
  || { cat "$TMP/neg-b.log" >&2; fail "negative B failed, but not with a type mismatch"; }
echo "negative B failed as required: type mismatch at the schedule_at_request call"

echo "✓ roc-cap-demo tests passed (positive path + both negative tests)"
