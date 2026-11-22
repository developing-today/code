#!/usr/bin/env bash
# Fleet Cargo.lock freshness guard (#10363).
#
# fleet-search path-depends on atlas-search crates (atlas-core,
# atlas-embed, atlas-proto) whose manifests live outside the fleet
# workspace. When one of them gains a dependency, pkgs/fleet/Cargo.lock
# silently falls behind: nothing in the fleet tree changed, and crane's
# own lockfile check compares the lock with the vendor dir built from
# that same lock, so a stale lock validates against itself. The first
# failure used to be a switch build dying at fleet-0.1.0.drv with
# "no matching package named ... found" (#8260, then #10362).
#
# This guard re-resolves the fleet workspace against the real manifests
# with `cargo metadata --locked`, which refuses to run when the
# committed lock is not the lock cargo would write. It compiles
# nothing, so it takes seconds.
#
# Usage: check-fleet-lock.sh [MANIFEST]
#   MANIFEST defaults to pkgs/fleet/Cargo.toml in this repo.
#   Extra cargo flags (for example --offline) can be passed in
#   FLEET_LOCK_CARGO_FLAGS.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="${1:-$ROOT/pkgs/fleet/Cargo.toml}"

if [ ! -f "$MANIFEST" ]; then
  echo "::error::fleet lock guard: manifest not found: $MANIFEST"
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "::error::fleet lock guard: cargo is not on PATH"
  exit 1
fi

# shellcheck disable=SC2086 # FLEET_LOCK_CARGO_FLAGS is a flag list.
if out=$(cargo metadata --locked --format-version 1 ${FLEET_LOCK_CARGO_FLAGS:-} \
  --manifest-path "$MANIFEST" 2>&1 >/dev/null); then
  echo "fleet lock guard: OK — $(dirname "$MANIFEST")/Cargo.lock matches the resolved workspace"
  exit 0
fi

echo "$out" | tail -20
if printf '%s' "$out" | grep -qE 'lock file .*needs to be updated|cannot update the lock file|--locked was passed'; then
  echo "::error::fleet lock guard: $(dirname "$MANIFEST")/Cargo.lock is stale against the manifests it resolves (often an atlas-search crate gained a dependency). Fix: cargo update --workspace --manifest-path $MANIFEST, then commit the lock."
else
  echo "::error::fleet lock guard: cargo could not resolve $MANIFEST (see the output above)"
fi
exit 1
