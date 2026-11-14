#!/usr/bin/env bash
# Tests for `lib/bws-to-sops.sh --check` (#5451).
#
# Oracle: fixture documents with known key sets and values. For each case the
# expected result is an exact exit code plus the exact drift lines (sorted key
# names, grouped + added / - removed / ~ value-changed). A second invariant
# oracle runs on every case: no fixture secret value may appear anywhere in
# the check's output, and the mirror file's bytes must be unchanged after the
# run (the check is read-only).
#
# bws and sops are stubbed on PATH: `bws secret list` prints the fixture BWS
# document ($BWS_FIXTURE) and `sops -d` prints the fixture mirror document
# ($SOPS_FIXTURE). No live credentials are used; BWS_ACCESS_TOKEN is a dummy.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
UNDER_TEST="$SCRIPT_DIR/bws-to-sops.sh"
[ -f "$UNDER_TEST" ] || {
  echo "FAIL: $UNDER_TEST not found" >&2
  exit 1
}
command -v jq >/dev/null 2>&1 || {
  echo "FAIL: jq not found in PATH" >&2
  exit 1
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
STUB_BIN="$TMP/bin"
mkdir -p "$STUB_BIN"

cat >"$STUB_BIN/bws" <<'STUB'
#!/usr/bin/env bash
# Fixture stub: only `secret list [--output json]` is supported.
cat "$BWS_FIXTURE"
STUB

cat >"$STUB_BIN/sops" <<'STUB'
#!/usr/bin/env bash
# Fixture stub: only decryption to stdout is supported.
if [ "${1:-}" = "-d" ]; then
  cat "$SOPS_FIXTURE"
  exit 0
fi
echo "sops stub: unsupported invocation: $*" >&2
exit 2
STUB
chmod +x "$STUB_BIN/bws" "$STUB_BIN/sops"

export PATH="$STUB_BIN:$PATH"
export BWS_ACCESS_TOKEN="dummy-token-for-fixtures"

FAILURES=0

# write_bws_fixture <file> <json-object-of-key:value>
# BWS `secret list` shape is a list of {key, value} objects.
write_bws_fixture() {
  jq -n --argjson doc "$2" '$doc | to_entries | map({key: .key, value: .value})' >"$1"
}

# run_case <name> <bws-json-object> <mirror-json-object|MISSING> <want-rc> <want-lines...>
run_case() {
  local name="$1" bws_doc="$2" mirror_doc="$3" want_rc="$4"
  shift 4
  local bws_fx="$TMP/$name.bws.json" mirror="$TMP/$name.mirror.yaml" out rc
  write_bws_fixture "$bws_fx" "$bws_doc"
  export BWS_FIXTURE="$bws_fx"
  if [ "$mirror_doc" = "BROKEN" ]; then
    # Mirror file exists, but decryption fails (stub is pointed at a
    # nonexistent fixture): an operational failure, not drift.
    printf 'not-a-real-sops-file\n' >"$mirror"
    export SOPS_FIXTURE="$TMP/does-not-exist.json"
  elif [ "$mirror_doc" = "MISSING" ]; then
    rm -f "$mirror"
    export SOPS_FIXTURE="$TMP/does-not-exist.json"
  else
    printf '%s\n' "$mirror_doc" >"$mirror"
    # The sops stub ignores the file argument and prints this fixture.
    export SOPS_FIXTURE="$mirror"
  fi
  local before="" after=""
  [ -f "$mirror" ] && before="$(sha256sum "$mirror" | cut -d' ' -f1)"

  # CASE_FLAGS is a word list on purpose (e.g. "--dry-run --check").
  local flags_str="${CASE_FLAGS:---check}"
  unset CASE_FLAGS
  local -a flags=($flags_str)
  out="$("$UNDER_TEST" "${flags[@]}" --out "$mirror" 2>&1)"
  rc=$?

  [ -f "$mirror" ] && after="$(sha256sum "$mirror" | cut -d' ' -f1)"

  local ok=true
  [ "$rc" -eq "$want_rc" ] || {
    ok=false
    echo "FAIL $name: exit $rc, want $want_rc"
  }
  local want
  for want in "$@"; do
    grep -qxF "$want" <<<"$out" || {
      ok=false
      echo "FAIL $name: missing line: $want"
    }
  done
  if grep -q 'VALUE-' <<<"$out"; then
    ok=false
    echo "FAIL $name: secret value leaked into output"
  fi
  if [ "$before" != "$after" ]; then
    ok=false
    echo "FAIL $name: --check mutated the mirror file"
  fi
  if [ "$ok" = true ]; then
    echo "PASS $name"
  else
    FAILURES=$((FAILURES + 1))
    echo "--- output for $name ---" >&2
    echo "$out" >&2
  fi
}

FRESH_BWS='{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA"}'
FRESH_MIRROR='{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA"}'

# 1. Fresh: identical key set and values -> exit 0, no drift lines.
run_case fresh "$FRESH_BWS" "$FRESH_MIRROR" 0

# 2. Added key in BWS, absent from the mirror -> exit 1, "+ delta".
run_case added \
  '{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA", "delta": "VALUE-DELTA"}' \
  "$FRESH_MIRROR" 1 \
  "  + delta"

# 3. Removed key: present in the mirror, gone from BWS -> exit 1, "- gone".
run_case removed \
  "$FRESH_BWS" \
  '{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA", "gone": "VALUE-GONE"}' 1 \
  "  - gone"

# 4. Value-changed key: same key set, one value differs -> exit 1, "~ beta".
run_case changed \
  '{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA-ROTATED"}' \
  "$FRESH_MIRROR" 1 \
  "  ~ beta"

# 5. Combined drift: one of each kind in a single run.
run_case combined \
  '{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA-ROTATED", "delta": "VALUE-DELTA"}' \
  '{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA", "gone": "VALUE-GONE"}' 1 \
  "  + delta" "  - gone" "  ~ beta"

# 6. Operational failure: mirror exists but cannot be decrypted -> exit 2,
#    never exit 1 (exit 1 is reserved for confirmed drift).
run_case operational \
  "$FRESH_BWS" "BROKEN" 2

# 7. Missing mirror file -> exit 1, every BWS key reported as added.
run_case missing \
  "$FRESH_BWS" "MISSING" 1 \
  "  + alpha" "  + beta"

# 8. Combined flags: --check takes precedence over --dry-run (#6631). A
#    stale mirror must report drift (exit 1) — the dry run's exit 0 for a
#    check that never compared anything was the defect.
CASE_FLAGS="--check --dry-run" run_case check_beats_dry_run \
  '{"alpha": "VALUE-ALPHA", "beta": "VALUE-BETA", "delta": "VALUE-DELTA"}' \
  "$FRESH_MIRROR" 1 \
  "  + delta"

# 9. Combined flags in the other order, fresh mirror -> exit 0, and the
#    output is the check's verdict, not the dry run's listing.
CASE_FLAGS="--dry-run --check" run_case dry_run_then_check_fresh \
  "$FRESH_BWS" "$FRESH_MIRROR" 0

if [ "$FAILURES" -gt 0 ]; then
  echo "FAIL: $FAILURES case(s) failed" >&2
  exit 1
fi
echo "OK: all bws-to-sops --check cases passed"
