#!/usr/bin/env bash
# Regression tests for check-openclaw-unit-wants.sh (#2581).
#
# The guard exists because a *clean* merge silently restored the openclaw
# proxy `Wants` lines that #2092 had deliberately removed (found by hand
# during the merge-everything pass, 2026-10-30). This suite drives the
# REAL guard (no pasted copy) against copied trees so the failure modes
# are pinned, not assumed:
#
#   * current tree                — passes unchanged (no unit behaviour change)
#   * untouched fixture           — passes; control for the mutants below
#   * #2092 resurrection           — the three removed lines put back into
#     openclaw-gateway's Wants    — fails, naming all three units
#   * single resurrection          — one manual proxy line back — fails
#   * openchamber resurrection     — the same #2092 follow-up pattern — fails
#   * a *new* proxy dependency     — a proxy the allowlist never mentioned
#     is still caught (the assertion is semantic, not a golden list)
#   * the same Wants added from a different file — union across
#     declarations is caught
#   * parse drift                  — a missing Wants list, a missing unit
#     block, or an empty proxy universe fails instead of passing silently
#
# Run: .github/scripts/check-openclaw-unit-wants.test.sh
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SELF_DIR/check-openclaw-unit-wants.sh"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

failures=0
pass() { echo "ok   - $*"; }
fail() {
  echo "FAIL - $*"
  failures=$((failures + 1))
}

# fixture NAME -> echoes a fresh directory holding the two real sources
fixture() {
  local dir="$TMP_ROOT/$1"
  mkdir -p "$dir/home/openclaw" "$dir/home/common"
  cp "$REPO_ROOT/home/openclaw/default.nix" "$dir/home/openclaw/default.nix"
  cp "$REPO_ROOT/home/common/default.nix" "$dir/home/common/default.nix"
  printf '%s\n' "$dir"
}

# run_guard DIR -> sets status/output
run_guard() {
  status=0
  output="$("$GUARD" "$1" 2>&1)" || status=$?
}

expect_pass() {
  local label=$1 dir=$2
  run_guard "$dir"
  if [[ $status -eq 0 ]]; then
    pass "$label"
  else
    fail "$label (guard exited $status)"
    printf '%s\n' "$output" | sed 's/^/       /'
  fi
}

# expect_fail LABEL DIR SUBSTRING...
expect_fail() {
  local label=$1 dir=$2
  shift 2
  run_guard "$dir"
  if [[ $status -eq 0 ]]; then
    fail "$label (guard passed; expected failure)"
    return
  fi
  local missing=0 needle
  for needle in "$@"; do
    if ! grep -qF -- "$needle" <<<"$output"; then
      fail "$label (failure did not name '$needle')"
      missing=1
    fi
  done
  if [[ $missing -eq 0 ]]; then
    pass "$label"
  else
    printf '%s\n' "$output" | sed 's/^/       /'
  fi
}

# insert_wants_line FILE UNIT ENTRY — add ENTRY right after UNIT's Wants list
# opener (first Wants inside that unit's block).
insert_wants_line() {
  local file=$1 unit=$2 entry=$3 tmp
  tmp="$(mktemp "$TMP_ROOT/insert.XXXXXX")"
  awk -v marker="systemd.user.services.${unit} = {" -v entry="        \"$entry\"" '
    index($0, marker) { inunit = 1 }
    inunit && !done && $0 ~ /^[[:space:]]*Wants = \[$/ { print; print entry; done = 1; next }
    { print }
  ' "$file" >"$tmp"
  mv "$tmp" "$file"
}

# drop_unit FILE UNIT — remove the unit's whole `= { ... }` block.
drop_unit() {
  local file=$1 unit=$2 tmp
  tmp="$(mktemp "$TMP_ROOT/drop.XXXXXX")"
  awk -v marker="systemd.user.services.${unit} = {" '
    index($0, marker) { skipping = 1 }
    !skipping { print }
    skipping && $0 == "  };" { skipping = 0 }
  ' "$file" >"$tmp"
  mv "$tmp" "$file"
}

# --- current tree: the guard must pass unchanged ---------------------------
expect_pass "current tree passes (units unchanged)" "$REPO_ROOT"

# --- control fixture -------------------------------------------------------
control="$(fixture control)"
expect_pass "untouched fixture passes" "$control"

# --- the #2228 merge shape: the three #2092 lines come back -----------------
resurrected="$(fixture resurrected)"
insert_wants_line "$resurrected/home/openclaw/default.nix" openclaw-gateway commandcode-proxy.service
insert_wants_line "$resurrected/home/openclaw/default.nix" openclaw-gateway design-code-proxy.service
insert_wants_line "$resurrected/home/openclaw/default.nix" openclaw-gateway agy-proxy.service
expect_fail "#2092 resurrection via merge is caught" "$resurrected" \
  "openclaw-gateway" "commandcode-proxy.service" "design-code-proxy.service" "agy-proxy.service"

single="$(fixture single)"
insert_wants_line "$single/home/openclaw/default.nix" openclaw-gateway agy-proxy.service
expect_fail "one resurrected line is caught" "$single" "agy-proxy.service"

# --- the same pattern on openchamber (the #2092 follow-up) -----------------
openchamber="$(fixture openchamber)"
insert_wants_line "$openchamber/home/common/default.nix" openchamber design-code-proxy.service
expect_fail "openchamber proxy Wants resurrection is caught" "$openchamber" \
  "openchamber" "design-code-proxy.service"

# --- semantic, not the three known names: a proxy never allowlisted --------
newproxy="$(fixture newproxy)"
insert_wants_line "$newproxy/home/openclaw/default.nix" openclaw-gateway claude-code-proxy.service
expect_fail "a newly introduced proxy dependency is caught" "$newproxy" "claude-code-proxy.service"

# --- the same dependency declared from another file ------------------------
elsewhere="$(fixture elsewhere)"
cat >>"$elsewhere/home/common/default.nix" <<'EOF'

{
  systemd.user.services.openclaw-gateway = {
    Unit.Wants = [ "muse-proxy.service" ];
  };
}
EOF
expect_fail "a Wants added by a second declaration is caught" "$elsewhere" "muse-proxy.service"

# --- parse drift fails loudly ---------------------------------------------
no_wants="$(fixture no-wants)"
awk '
  /^      Wants = \[$/ { skipping = 1 }
  skipping { if ($0 == "      ];") skipping = 0; next }
  { print }
' "$no_wants/home/openclaw/default.nix" >"$no_wants/nix.tmp"
mv "$no_wants/nix.tmp" "$no_wants/home/openclaw/default.nix"
expect_fail "a unit that lost its Wants list is parse drift" "$no_wants" "declares no Wants list"

no_unit="$(fixture no-unit)"
drop_unit "$no_unit/home/openclaw/default.nix" openclaw-gateway
expect_fail "a missing unit block is parse drift" "$no_unit" "no declaration of systemd.user.services.openclaw-gateway"

no_proxies="$(fixture no-proxies)"
grep -vE '^[[:space:]]*systemd\.user\.services\.[A-Za-z0-9_-]*-proxy = \{' \
  "$no_proxies/home/common/default.nix" >"$no_proxies/nix.tmp"
mv "$no_proxies/nix.tmp" "$no_proxies/home/common/default.nix"
expect_fail "an empty proxy universe is parse drift" "$no_proxies" "no *-proxy services found"

# --- a stale allowlist entry (guard rot) fails instead of over-permitting --
stale_guard="$TMP_ROOT/check-openclaw-unit-wants.stale.sh"
sed 's#^  "openclaw-gateway|opencode-route-proxy.service"$#  "openclaw-gateway|opencode-route-proxy.service nosuch-proxy.service"#' \
  "$GUARD" >"$stale_guard"
chmod +x "$stale_guard"

if cmp -s "$GUARD" "$stale_guard"; then
  fail "stale allowlist case could not rewrite the guard's allowlist (pattern drift)"
else
  stale_status=0
  stale_output="$("$stale_guard" "$REPO_ROOT" 2>&1)" || stale_status=$?
  if [[ $stale_status -ne 0 ]] && grep -qF "nosuch-proxy.service" <<<"$stale_output"; then
    pass "a stale allowlist entry fails instead of over-permitting"
  else
    fail "a stale allowlist entry was not rejected (guard exited $stale_status)"
    printf '%s\n' "$stale_output" | sed 's/^/       /'
  fi
fi

echo
if [[ $failures -ne 0 ]]; then
  echo "$failures case(s) failed"
  exit 1
fi
echo "all cases passed"
