#!/usr/bin/env bash
# publish-fleet.sh — build the static fleet binary for Muse and upload it as
# a GitHub release asset on developing-today/code.
#
#   scripts/muse/publish-fleet.sh [REV]     # default: origin/main
#
# Prints the tag and sha256 to pin in scripts/muse/bootstrap.sh.
# Needs nix, git and gh (logged in with repo scope). Re-running for the same
# rev re-uses the existing release when its asset matches.
set -euo pipefail
# Command line: at most one positional REV, no flags; checked before any
# subprocess (doc/agents/operator-script-argument-contract.md).
case "${1-}" in
-h | --help)
  sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
  ;;
-*)
  echo "publish-fleet: unknown flag $1" >&2
  exit 2
  ;;
esac
[ $# -le 1 ] || {
  echo "publish-fleet: takes at most one REV, got $# arguments" >&2
  exit 2
}
REPO=developing-today/code
ASSET=fleet-x86_64-linux-musl
root=$(git rev-parse --show-toplevel)
git -C "$root" fetch -q origin main
rev=$(git -C "$root" rev-parse "${1:-origin/main}")
short=${rev:0:7}
tag="fleet-static-$short"
work=$(mktemp -d)
trap 'git -C "$root" worktree remove --force "$work/src" >/dev/null 2>&1 || true; rm -rf "$work"' EXIT
git -C "$root" worktree add -q --detach "$work/src" "$rev"
nix build "$work/src#fleet-static" -o "$work/result"
bin="$work/result/bin/fleet"
file -L "$bin" | grep -q 'statically linked\|static-pie' || {
  echo "publish-fleet: $bin is not static" >&2
  exit 1
}
"$bin" capabilities | python3 -c '
import json, sys
d = json.load(sys.stdin)
sys.exit(0 if any(x["name"] == "bridge" and x["available"] for x in d["domains"]) else 1)' ||
  {
    echo "publish-fleet: build has no bridge verb" >&2
    exit 1
  }
cp "$bin" "$work/$ASSET"
sum=$(sha256sum "$work/$ASSET" | cut -d' ' -f1)
echo "$sum  $ASSET" >"$work/$ASSET.sha256"
if gh release view "$tag" -R "$REPO" >/dev/null 2>&1; then
  gh release download "$tag" -R "$REPO" -p "$ASSET.sha256" -O - 2>/dev/null | grep -q "^$sum " ||
    gh release upload "$tag" -R "$REPO" --clobber "$work/$ASSET" "$work/$ASSET.sha256"
else
  gh release create "$tag" -R "$REPO" --target "$rev" --prerelease \
    --title "fleet static build $short" \
    --notes "Static musl fleet binary (full profile, includes bridge) built from $rev with \`nix build .#fleet-static\`. Used by scripts/muse/bootstrap.sh." \
    "$work/$ASSET" "$work/$ASSET.sha256"
fi
echo "tag=$tag"
echo "sha256=$sum"
