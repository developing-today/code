#!/bin/sh
# Install mcpx from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/pkgs/mcpx/scripts/install.sh | sh
#
# Channel 2 of decision 0005 §2: it installs the release binaries from
# channel 1, so nothing here builds anything.
#
# Per-user by default (~/.local/bin, no sudo). The system-wide variant is
# documented, not default:
#
#   curl -fsSL .../install.sh | sh -s -- --system
#
# --system installs into /usr/local/bin and escalates only the copy with
# sudo when that directory is not writable. MCPX_PREFIX overrides either
# default:
#
#   curl -fsSL .../install.sh | sudo MCPX_PREFIX=/usr/local/bin sh
#
# The checksum is verified before anything is installed. An installer that
# skips verification teaches people to pipe unverified binaries, which is
# exactly the habit this channel exists to avoid.
#
# A bare binary has no JavaScript runtime, so `mcpx run` and `mcpx exec` need
# Deno, Bun or Node on PATH. The install text says so; `mcpx doctor` reports
# what it found.
#
# Overrides, all optional:
#   MCPX_VERSION   version to install (default: the latest mcpx release)
#   MCPX_PREFIX    install directory (default: ~/.local/bin)
#   MCPX_RELEASES_URL  releases-list endpoint used to resolve the default
#                  version; CI points it at a fixture file:// URL
#   MCPX_BASE_URL  where the tarballs and SHA256SUMS live; defaults to the
#                  release download URL. A local directory or file:// URL
#                  works, which is how CI exercises this script against a
#                  snapshot without network or a published release.
set -eu

repo=developing-today/code

# Arguments: --system selects the system-wide prefix (/usr/local/bin);
# --user is the explicit form of the default. Anything else is a mistake
# worth refusing rather than silently installing somewhere unexpected --
# an earlier version of this script ignored its arguments entirely, so
# `sh -s -- --system` quietly did a per-user install (Codex finding on
# PR #2584).
system=0
for arg in "$@"; do
  case "$arg" in
  --system) system=1 ;;
  --user) system=0 ;;
  *)
    printf '%s\n' "mcpx installer: unknown argument '$arg' (expected --system or --user)" >&2
    exit 1
    ;;
  esac
done

if [ -n "${MCPX_PREFIX:-}" ]; then
  prefix=$MCPX_PREFIX
elif [ "$system" -eq 1 ]; then
  prefix=/usr/local/bin
else
  prefix=${HOME}/.local/bin
fi
base_url=${MCPX_BASE_URL:-}

say() { printf '%s\n' "$*" >&2; }
die() {
  say "mcpx installer: $*"
  exit 1
}

# ---- what this machine is -------------------------------------------------

case "$(uname -s)" in
Linux) os=linux ;;
Darwin) os=darwin ;;
*) die "unsupported system $(uname -s); mcpx releases are Linux and macOS only (decision 0005 §1)" ;;
esac

case "$(uname -m)" in
x86_64 | amd64) arch=amd64 ;;
arm64 | aarch64) arch=arm64 ;;
*) die "unsupported machine $(uname -m)" ;;
esac

# The release asset names use the flake's system spelling (decision 0005 §1),
# so the mapping is explicit rather than composed at the call site.
case "${os}-${arch}" in
linux-amd64) system=x86_64-linux ;;
linux-arm64) system=aarch64-linux ;;
darwin-arm64) system=aarch64-darwin ;;
darwin-amd64) die "no x86_64 macOS release: the flake has no x86_64-darwin system (decision 0005 §1)" ;;
*) die "no release for ${os}/${arch}" ;;
esac

# ---- which version --------------------------------------------------------

version=${MCPX_VERSION:-}
if [ -z "$version" ]; then
  # Ask the API for the latest mcpx release. `gh` is not assumed; this is a
  # curl|sh installer, so it uses curl.
  #
  # NOT /releases/latest: that endpoint returns the newest release of the
  # whole repository, and this monorepo publishes other packages' releases
  # too (graph, via .github/workflows/goreleaser.yml) — when one of those
  # is newest there is no mcpx version to extract and the default install
  # breaks. List releases instead (the API returns them newest-first) and
  # take the first non-prerelease mcpx/v tag. In each release object
  # tag_name precedes prerelease, so the awk below can carry the candidate
  # from one line to the other.
  releases_url=${MCPX_RELEASES_URL:-"https://api.github.com/repos/${repo}/releases?per_page=100"}
  version=$(curl -fsSL "$releases_url" | awk '
    /"tag_name":/ {
      pending = ""
      if (match($0, /"tag_name": *"mcpx\/v[^"]*"/)) {
        pending = substr($0, RSTART, RLENGTH)
        sub(/"tag_name": *"mcpx\/v/, "", pending)
        sub(/"$/, "", pending)
      }
      next
    }
    /"prerelease": *true/ { pending = "" }
    /"prerelease": *false/ && pending != "" { print pending; exit }
  ') || die "could not ask GitHub for the latest mcpx release"
  [ -n "$version" ] ||
    die "no mcpx release found; set MCPX_VERSION to a version that exists (see https://github.com/${repo}/releases)"
fi
version=${version#v}

if [ -z "$base_url" ]; then
  base_url="https://github.com/${repo}/releases/download/mcpx/v${version}"
fi

asset="mcpx-${version}-${system}.tar.gz"
say "mcpx installer: ${version} for ${system} -> ${prefix}"

# ---- fetch and verify -----------------------------------------------------

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch() {
  # $1 = base (a local directory, a file:// URL, or an https URL)
  # $2 = destination path
  file=$(basename "$2")
  case "$1" in
  file://*) cp "${1#file://}/${file}" "$2" ;;
  /*) cp "$1/${file}" "$2" ;;
  *) curl -fsSL "$1/${file}" -o "$2" ;;
  esac
}

say "  fetching ${asset}"
fetch "$base_url" "$tmp/$asset" || die "could not download ${asset} from ${base_url}"
say "  fetching SHA256SUMS"
fetch "$base_url" "$tmp/SHA256SUMS" || die "could not download SHA256SUMS from ${base_url}"

# Verify only this asset's line: SHA256SUMS covers all three platforms, and
# the others are not here to be checked.
line=$(grep " ${asset}\$" "$tmp/SHA256SUMS" || true)
[ -n "$line" ] || die "${asset} is not listed in SHA256SUMS"
say "  verifying checksum"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$tmp" && printf '%s\n' "$line" | sha256sum -c -) || die "checksum mismatch for ${asset}"
elif command -v shasum >/dev/null 2>&1; then
  (cd "$tmp" && printf '%s\n' "$line" | shasum -a 256 -c -) || die "checksum mismatch for ${asset}"
else
  die "neither sha256sum nor shasum is available; cannot verify the download"
fi

tar -C "$tmp" -xzf "$tmp/$asset" || die "could not unpack ${asset}"

# ---- install --------------------------------------------------------------

if ! mkdir -p "$prefix" 2>/dev/null; then
  say "  ${prefix} does not exist and cannot be created; trying sudo"
  sudo mkdir -p "$prefix"
fi
if [ -w "$prefix" ]; then
  mv "$tmp/mcpx" "$prefix/mcpx"
  chmod 0755 "$prefix/mcpx"
else
  # The per-user default never needs this; --system (or an MCPX_PREFIX
  # pointing somewhere unwritable) escalates only this copy, exactly as
  # docs/install.md promises.
  say "  ${prefix} is not writable; trying sudo"
  sudo mv "$tmp/mcpx" "$prefix/mcpx"
  sudo chmod 0755 "$prefix/mcpx"
fi

installed=$("$prefix/mcpx" --version 2>/dev/null || echo "mcpx (unknown version)")
say "  installed: ${installed} at ${prefix}/mcpx"

case ":${PATH}:" in
*":${prefix}:"*) ;;
*) say "  note: ${prefix} is not on PATH; add it to your shell profile" ;;
esac

# The caveat every non-nix channel states (decision 0005 §1, §5).
say ""
say "mcpx needs a JavaScript runtime for 'mcpx run' and 'mcpx exec'."
say "Install Deno, Bun or Node, then run 'mcpx doctor' to confirm what it found."
