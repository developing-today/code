#!/usr/bin/env bash
# id shell release snapshot (#12819): gate, build and package one shell
# for one platform, with the granular per-app, per-platform tag scheme
# from the Q3 ruling on #6297 (releases ship per platform as each is
# ready).
#
# Tag scheme: <app>/<platform>/v<version>, e.g.
#   id-native-shell/linux-x86_64/v0.1.0
# the monorepo prefix convention of mcpx-release (mcpx/vX.Y.Z), one level
# deeper so each platform of each shell ships on its own.
#
# Steps, all fail-closed:
#   1. gate: the shell's own test check, then the shell conformance suite
#      (`id world contract --check-suite contract/shell-suite.v1.json`),
#      which fails on any shell whose recorded session misses its
#      capability statement;
#   2. build: the shell's flake package for the platform's Nix system;
#   3. package: the package's runtime closure as a zstd-compressed
#      `nix-store --export` stream, the store path it installs, a SHA-256
#      sums file, and a short INSTALL note.
#
# This script never tags or publishes. The workflow that runs it uploads
# OUTDIR as a workflow artifact (a snapshot); publishing a release joins
# release-gate.sh when tag-push releases are re-armed (ci-policy.md).
#
# Usage: id-shell-release.sh APP PLATFORM OUTDIR
#        id-shell-release.sh --plan APP PLATFORM   (print tag/asset names only)
set -euo pipefail

usage() {
  echo "usage: id-shell-release.sh [--plan] APP PLATFORM [OUTDIR]" >&2
  exit 2
}

plan_only=0
if [[ ${1:-} == "--plan" ]]; then
  plan_only=1
  shift
fi
[[ $# -ge 2 ]] || usage
app=$1
platform=$2
outdir=${3:-}
[[ $plan_only == 1 || -n $outdir ]] || usage

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
idroot="$root/pkgs/id"

# Supported (app, platform) pairs and what gates them. A pair is added
# here when its package and its gate exist; anything else is refused.
case "$app/$platform" in
id-native-shell/linux-x86_64)
  system=x86_64-linux
  gate_check=test-native-shell
  ;;
*)
  echo "id-shell-release: unsupported app/platform: $app/$platform" >&2
  exit 2
  ;;
esac

manifest="$idroot/crates/$app/Cargo.toml"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$manifest" | head -n 1)
[[ -n $version ]] || {
  echo "id-shell-release: no version in $manifest" >&2
  exit 1
}
tag="$app/$platform/v$version"
base="$app-$platform-v$version"

echo "app=$app"
echo "platform=$platform"
echo "version=$version"
echo "tag=$tag"
echo "asset_base=$base"
[[ $plan_only == 0 ]] || exit 0

echo "--- gate: $gate_check"
nix build -L --no-link "$idroot#checks.$system.$gate_check"

echo "--- gate: shell conformance suite"
nix develop "$idroot" --command bash -c \
  'cd "$1" && cargo run --locked -q -p id --no-default-features --features world -- world contract --check-suite contract/shell-suite.v1.json' \
  _ "$idroot"

echo "--- build: $app for $system"
out=$(nix build -L --no-link --print-out-paths "$idroot#packages.$system.$app")
echo "store path: $out"

mkdir -p "$outdir"
if ! command -v zstd >/dev/null; then
  zstd() { nix run nixpkgs#zstd -- "$@"; }
fi
mapfile -t closure < <(nix-store --query --requisites "$out")
[[ ${#closure[@]} -gt 0 ]] || {
  echo "id-shell-release: empty closure for $out" >&2
  exit 1
}
nix-store --export "${closure[@]}" | zstd -19 -T0 -q -o "$outdir/$base.closure.zst"
printf '%s\n' "$out" >"$outdir/$base.storepath"
cat >"$outdir/$base.INSTALL.txt" <<NOTE
$app $version for $platform (tag $tag)

Install on a Linux machine with Nix:
  zstd -dc $base.closure.zst | nix-store --import
  nix profile install $out

Then run: $app --help
NOTE
(cd "$outdir" && sha256sum "$base".* >"$base.sha256.tmp" && mv "$base.sha256.tmp" "$base.sha256")
echo "--- packaged"
ls -la "$outdir"
cat "$outdir/$base.sha256"
