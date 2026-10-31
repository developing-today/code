#!/usr/bin/env bash
# Guard the generated proxy units' Wants against silent resurrection (#2581).
#
# The systemd units are generated from the Nix definitions in
# home/openclaw/default.nix and home/common/default.nix, so a unit's
# generated `Wants=` is exactly the union of the `Unit.Wants` lists those
# files declare for it (home-manager concatenates list options; nothing
# else in the repo may add one).
#
# #2092 deliberately removed `Wants` on the manual per-proxy units from
# openclaw-gateway (commandcode/design-code/agy). Those manual units are
# one-off fallbacks whose lifecycle is not tied to OpenClaw's, and the
# commandcode-proxy unit crash-loops on its already-taken port (#883); a
# gateway restart must not re-pull them. The gateway reaches the proxies
# by port, not by unit. Merging a stale branch cleanly put the three
# lines back once (caught only by a hand post-merge tree comparison);
# nothing in CI noticed.
#
# This guard therefore checks the invariant semantically, not by diffing
# whole units: a consumer unit may only Want a `*-proxy` service that is
# named in the allowlist below, which encodes the deliberate #2092 (and
# #2092 follow-up, openchamber) removal. Any merge or regeneration that
# reintroduces a proxy dependency fails, naming the consumer, the unit
# and the file it was declared in.
#
# The proxy universe is derived from the source: every
# `systemd.user.services.<name>-proxy` declaration in home/common/default.nix
# (plus any `*-proxy.service` name, so a proxy defined elsewhere is still
# caught). Parse drift — a missing unit block, a missing Wants list, no
# proxy units found at all — is a failure too; a guard that silently
# passes on empty input is the failure this script exists to prevent.
#
# Usage: check-openclaw-unit-wants.sh [repo-root]
set -euo pipefail

repo_root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
repo_root="${repo_root%/}"
common_nix="$repo_root/home/common/default.nix"

# consumer | allowed proxy Wants (space-separated)
#
# openclaw-gateway: the opencode-route proxy is a real unit dependency
# (an automatic service, Install.WantedBy default.target, #2104/#2229).
# mcpx is not a proxy and is not the guard's concern.
#
# openchamber: same #2092 treatment (afb84957) — mcpx only, no proxy
# units. Remove an entry here only in the change that deliberately adds
# that dependency to the unit.
consumers=(
  "openclaw-gateway|opencode-route-proxy.service"
  "openchamber|"
)

fail() {
  echo "::error::$*"
  exit 1
}

# The `systemd.user.services.<unit> = { ... }` block of one file, with
# string literals stripped so braces inside them cannot skew the depth
# count. The declaration line is known to exist (the caller greps for it).
unit_block() {
  local file=$1 unit=$2
  awk -v marker="systemd.user.services.${unit} = {" '
    {
      line = $0
      # drop double-quoted strings; these blocks have no escaped quotes
      gsub(/"[^"]*"/, "", line)
      opens = gsub(/\{/, "{", line)
      closes = gsub(/\}/, "}", line)
    }
    !started && index($0, marker) { started = 1 }
    started {
      print
      depth += opens - closes
      if (depth <= 0) exit
    }
  ' "$file"
}

# The quoted entries of every `Wants = [ ... ];` list in the block on
# stdin, one per line.
wants_entries() {
  awk '
    !inlist && $0 ~ /(^|[^A-Za-z0-9_])Wants[[:space:]]*=[[:space:]]*\[/ { inlist = 1 }
    inlist {
      line = $0
      while (match(line, /"[^"]*"/)) {
        print substr(line, RSTART + 1, RLENGTH - 2)
        line = substr(line, RSTART + RLENGTH)
      }
      # the list ends on the line carrying its closing bracket
      if (index($0, "]")) inlist = 0
    }
  '
}

[[ -f $common_nix ]] || fail "$common_nix not found — cannot derive the proxy unit universe."

# --- the proxy universe, from the source definitions -----------------------
proxy_units=()
while IFS= read -r name; do
  [[ -n $name ]] && proxy_units+=("${name}.service")
done < <(
  grep -oE '^[[:space:]]*systemd\.user\.services\.[A-Za-z0-9_-]+ = \{' "$common_nix" |
    sed -E 's/.*systemd\.user\.services\.([A-Za-z0-9_-]+) = \{/\1/' |
    grep -E '[-]proxy$' |
    sort -u
)
[[ ${#proxy_units[@]} -gt 0 ]] ||
  fail "no *-proxy services found in home/common/default.nix — the proxy universe parser has drifted."

is_proxy_unit() {
  local unit=$1 candidate
  [[ $unit == *-proxy.service ]] && return 0
  for candidate in "${proxy_units[@]}"; do
    [[ $candidate == "$unit" ]] && return 0
  done
  return 1
}

is_allowed() {
  local unit=$1 allowed=$2 entry
  for entry in $allowed; do
    [[ $entry == "$unit" ]] && return 0
  done
  return 1
}

failed=0
checked=0
for consumer_entry in "${consumers[@]}"; do
  IFS='|' read -r consumer allowed <<<"$consumer_entry"

  # every tracked source tree that declares this consumer unit
  mapfile -t files < <(
    grep -rl --include='*.nix' -F "systemd.user.services.${consumer}" \
      "$repo_root/home" "$repo_root/nixos" 2>/dev/null | sort -u
  )
  [[ ${#files[@]} -gt 0 ]] ||
    fail "no declaration of systemd.user.services.${consumer} found under home/ or nixos/ — parse drift."

  wants_seen=0
  for file in "${files[@]}"; do
    block="$(unit_block "$file" "$consumer")"
    [[ -n $block ]] ||
      fail "found systemd.user.services.${consumer} in ${file#"$repo_root"/} but could not read its block — parse drift."

    mapfile -t wants < <(printf '%s\n' "$block" | wants_entries)
    [[ ${#wants[@]} -gt 0 ]] ||
      fail "systemd.user.services.${consumer} in ${file#"$repo_root"/} declares no Wants list — parse drift."
    wants_seen=1

    for unit in "${wants[@]}"; do
      is_proxy_unit "$unit" || continue
      checked=$((checked + 1))
      is_allowed "$unit" "$allowed" && continue
      echo "::error::${consumer} in ${file#"$repo_root"/} Wants ${unit}, a proxy service its definition does not sanction. #2092 removed proxy Wants deliberately: these units have their own lifecycle and a consumer restart must not re-pull them (commandcode-proxy crash-loops on its taken port, #883). Reverting that needs a deliberate allowlist entry in .github/scripts/check-openclaw-unit-wants.sh."
      failed=1
    done
  done
  [[ $wants_seen -eq 1 ]] || fail "no Wants list read for systemd.user.services.${consumer} — parse drift."

  # a stale allowlist entry would silently stop protecting the dependency
  for unit in $allowed; do
    if [[ $unit != *-proxy.service ]]; then
      fail "allowlist for ${consumer} names ${unit}, which is not a proxy service."
    fi
    if ! printf '%s\n' "${proxy_units[@]}" | grep -qxF "$unit"; then
      fail "allowlist for ${consumer} names ${unit}, which home/common/default.nix does not declare as a proxy unit."
    fi
  done
done

if [[ $failed -ne 0 ]]; then
  exit 1
fi

echo "openclaw proxy Wants guard: ${#consumers[@]} consumers, ${checked} proxy dependencies, all sanctioned (${#proxy_units[@]} proxy units in the universe)."
