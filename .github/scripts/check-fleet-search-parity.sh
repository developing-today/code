#!/usr/bin/env bash
# Byte-parity drift guard for the fleet-search / atlas-search writers
# (#5920, contract origin #5433).
#
# pkgs/fleet/crates/fleet-search/src/{pyjson,render}.rs are verbatim
# copies of pkgs/atlas-search/crates/atlas-search/src/{pyjson,render}.rs
# (fleet step 5, #5433): the output-parity contract lives in these
# writers, so the fleet verb renders service envelopes byte-identically
# to atlas-search. Nothing enforced that after the port, so a fix to a
# float-formatting or escaping edge case in one copy would silently
# break the contract.
#
# Rule: after removing each file's parity header -- the four `//!`
# lines at lines 2-5, directly after the shared first line -- the two
# copies of each pair must be byte-identical. The headers are not
# free-form: this script pins their exact text, so a header that
# changes or disappears fails here too, and an intentional header
# change must update this script in the same PR. Any other divergence
# fails with a diff.
#
# Usage: check-fleet-search-parity.sh [REPO_ROOT]
# REPO_ROOT defaults to the tree two directories above this script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"

ATLAS_DIR="$ROOT/pkgs/atlas-search/crates/atlas-search/src"
FLEET_DIR="$ROOT/pkgs/fleet/crates/fleet-search/src"

ATLAS_HEADER='//! Source of the verbatim copy in pkgs/fleet'\''s fleet-search crate,
//! ported in fleet step 5 (#5433): the output-parity contract lives
//! in these writers, so mirror every change there. Drift guard:
//! .github/scripts/check-fleet-search-parity.sh (#5920).'

FLEET_HEADER='//! Ported verbatim from pkgs/atlas-search'\''s bin crate in fleet step 5
//! (#5433): the output-parity contract lives in these writers, so the
//! fleet verb renders service envelopes byte-identically to atlas-search.
//! Drift guard: .github/scripts/check-fleet-search-parity.sh (#5920).'

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0

# check_pair <name> : compare $ATLAS_DIR/<name> with $FLEET_DIR/<name>.
check_pair() {
  local name="$1"
  local atlas="$ATLAS_DIR/$name" fleet="$FLEET_DIR/$name"
  local f
  for f in "$atlas" "$fleet"; do
    if [ ! -f "$f" ]; then
      echo "::error::check-fleet-search-parity: missing file: $f (#5920)"
      fail=1
      return
    fi
  done

  # The parity header is lines 2-5 of each copy; pin its exact text.
  local got_atlas got_fleet
  got_atlas="$(sed -n '2,5p' "$atlas")"
  got_fleet="$(sed -n '2,5p' "$fleet")"
  if [ "$got_atlas" != "$ATLAS_HEADER" ]; then
    echo "::error::check-fleet-search-parity: parity header changed or missing in $atlas -- the header is pinned by this guard; update the header and this script together (#5920)"
    fail=1
  fi
  if [ "$got_fleet" != "$FLEET_HEADER" ]; then
    echo "::error::check-fleet-search-parity: parity header changed or missing in $fleet -- the header is pinned by this guard; update the header and this script together (#5920)"
    fail=1
  fi

  # Everything except the parity header (line 1 + lines 6+) must match.
  sed '2,5d' "$atlas" >"$TMP/$name.atlas"
  sed '2,5d' "$fleet" >"$TMP/$name.fleet"
  if ! diff -u "$TMP/$name.atlas" "$TMP/$name.fleet" >"$TMP/$name.diff"; then
    echo "::error::check-fleet-search-parity: $name diverged between atlas-search and fleet-search beyond the parity header -- the #5433 byte-parity contract is broken; mirror the change in both copies (#5920)"
    cat "$TMP/$name.diff"
    fail=1
  fi
}

check_pair pyjson.rs
check_pair render.rs

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "fleet-search parity: OK - pyjson.rs and render.rs match atlas-search byte-for-byte outside the pinned parity headers (#5433 contract, guard #5920)"
