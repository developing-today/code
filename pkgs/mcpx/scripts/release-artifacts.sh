#!/usr/bin/env bash
# Build the release artifacts for one mcpx version.
#
# This is the one implementation of "what a release is" (decision 0005 §2.1,
# §5): three CGO_ENABLED=0 static binaries for the flake's systems, one
# tarball each, and a SHA256SUMS file. The release workflow
# (.github/workflows/mcpx-release.yml) calls it, and so can a person on a
# laptop -- a snapshot is a workflow_dispatch run of the same script, not a
# second code path that could drift.
#
#   scripts/release-artifacts.sh <out-dir> [version]
#
# The version defaults to pkgs/mcpx/VERSION (the single source, §3). The
# workflow passes the tag's version instead, so a tag that disagrees with
# VERSION is caught by the caller before anything is built.
#
# Linux binaries are fully static. The behaviour that buys is recorded where
# the install page reads it (docs/container.md, decision 0005 §5): pure-Go
# os/user parses /etc/passwd instead of asking NSS, and the pure-Go resolver
# does not honour nsswitch.conf.
set -euo pipefail

out=${1:?usage: release-artifacts.sh <out-dir> [version]}
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
version=${2:-$(tr -d '\n' <"$here/VERSION")}

# The tag scheme and the tarball names both use the version without a
# leading v: tags are mcpx/vX.Y.Z, artifacts mcpx-<X.Y.Z>-<system>.tar.gz.
case "$version" in
v*) version=${version#v} ;;
esac

mkdir -p "$out"
out=$(cd -- "$out" && pwd)

# The three release targets, in the flake's own system spelling so the names
# match what a reader of flake.nix expects (decision 0005 §1).
targets=(
  x86_64-linux:linux:amd64
  aarch64-linux:linux:arm64
  aarch64-darwin:darwin:arm64
)

cd "$here"
for t in "${targets[@]}"; do
  IFS=: read -r system goos goarch <<<"$t"
  name="mcpx-${version}-${system}"
  stage="$out/.stage/$name"
  mkdir -p "$stage"

  echo "==> $name (GOOS=$goos GOARCH=$goarch CGO_ENABLED=0)"
  CGO_ENABLED=0 GOOS=$goos GOARCH=$goarch go build \
    -trimpath \
    -buildvcs=false \
    -ldflags "-s -w -X main.version=${version}" \
    -o "$stage/mcpx" ./cmd/mcpx

  # A static binary is the promise, so check it rather than trust the flag.
  # file(1) reads a cross-built ELF or Mach-O as happily as the host's own,
  # so every target is checked here, not only the one this machine can run.
  if command -v file >/dev/null 2>&1; then
    desc=$(file -b "$stage/mcpx")
    case "$goos" in
    linux)
      case "$desc" in
      *"statically linked"*) ;;
      *) echo "error: $name is not a static binary: $desc" >&2; exit 1 ;;
      esac
      ;;
    darwin)
      case "$desc" in
      *Mach-O*) ;;
      *) echo "error: $name is not a Mach-O binary: $desc" >&2; exit 1 ;;
      esac
      ;;
    esac
  fi

  tar -C "$stage" -czf "$out/$name.tar.gz" mcpx
  rm -rf "$out/.stage"
done

(
  cd "$out"
  sha256sum mcpx-*.tar.gz >SHA256SUMS
)

echo "==> artifacts in $out"
ls -l "$out"
echo "==> SHA256SUMS"
cat "$out/SHA256SUMS"
