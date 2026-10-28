#!/usr/bin/env bash
# Guard the id PR gate against flake-check drift (#300).
#
# id-check.yml builds an explicit list of fast checks (id-pr-checks.txt)
# instead of the full `nix flake check`, so a NEW check added to
# pkgs/id/flake.nix would silently get no PR coverage until someone
# remembered to list it. This script compares the flake's check names
# against the list and fails, naming the offenders in both directions:
#
#   - in the flake but not listed and not excluded -> unlisted check
#   - listed but no longer defined by the flake    -> stale entry
#
# The exclusion list below is the ONLY place it lives: the `default`
# aggregate (it re-runs the listed set in one derivation), the heavy
# checks that run nightly in id-e2e-nightly.yml (which also runs the
# full `nix flake check` as a safety net), and the aggregate test
# recipes `test`, `test-web` and `test-int` — on PRs their content is
# covered by composition instead: test-unit + test-shard-0/1/2/3 +
# test-web-unit + test-web-typecheck run every test they run (#869).
# The aggregate `crash-gate` is likewise covered by composition on
# PRs: crash-gate-sampled/-sweep/-control/-ingest partition its tests
# by name filter (#551). They still run in the nightly full flake check.
# That partition is a checked invariant, not just prose:
# check-crash-gate-coverage.sh runs in the same guard job and fails,
# naming the test, when an #[ignore]d crash test matches none of the
# split filters (#1635).
#
# Usage: check-id-pr-checks.sh [list-file]
# For tests, ID_FLAKE_CHECKS_FILE names a file with one flake check
# name per line to compare against instead of evaluating the flake.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
list_file="${1:-$repo_root/.github/scripts/id-pr-checks.txt}"

excluded=(
  default
  test-e2e
  nixos-serve
  nixos-e2e
  nixos-playwright-e2e
  nixos-integration
  test
  test-web
  test-int
  crash-gate
)

if [[ -n ${ID_FLAKE_CHECKS_FILE:-} ]]; then
  flake_checks="$(sort "$ID_FLAKE_CHECKS_FILE")"
else
  flake_checks="$(
    nix eval --raw "$repo_root/pkgs/id#checks.x86_64-linux" \
      --apply 'checks: builtins.concatStringsSep "\n" (builtins.attrNames checks)' |
      sort
  )"
fi

excluded_sorted="$(printf '%s\n' "${excluded[@]}" | sort)"
expected="$(comm -23 <(printf '%s\n' "$flake_checks") <(printf '%s\n' "$excluded_sorted"))"
listed="$(grep -v -e '^[[:space:]]*#' -e '^[[:space:]]*$' "$list_file" | sort)"

unlisted="$(comm -23 <(printf '%s\n' "$expected") <(printf '%s\n' "$listed"))"
stale="$(comm -13 <(printf '%s\n' "$expected") <(printf '%s\n' "$listed"))"

failed=0
if [[ -n $unlisted ]]; then
  failed=1
  while IFS= read -r name; do
    echo "::error::pkgs/id flake check '$name' is not in ${list_file#"$repo_root"/} — add it to the PR gate list, or to the exclusion list in check-id-pr-checks.sh if it belongs in the nightly only."
  done <<<"$unlisted"
fi
if [[ -n $stale ]]; then
  failed=1
  while IFS= read -r name; do
    echo "::error::${list_file#"$repo_root"/} lists '$name', which pkgs/id/flake.nix no longer defines — remove or rename the entry."
  done <<<"$stale"
fi
if [[ $failed -ne 0 ]]; then
  exit 1
fi

count="$(printf '%s\n' "$listed" | wc -l)"
echo "id PR check list matches pkgs/id/flake.nix: $count checks, ${#excluded[@]} excluded (nightly/aggregate)."
