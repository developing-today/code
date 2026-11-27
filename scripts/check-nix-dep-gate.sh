#!/usr/bin/env bash
# Dependency-bump Nix gate (#14195).
#
# Dependency automation bumps lockfiles/manifests without regenerating the
# pinned Nix hashes that cover those trees (vendorHash, npmDepsHash, FOD),
# so the NixOS build breaks on main — four instances in one night
# (#14051, #14122, #14140, the openchamber standalone FOD hash) plus the
# Cargo-side variant (#14315: bump landed, adaptation did not).
#
# This gate enforces the companion-change rule: a change to a derivation's
# dependency inputs must ship alongside the pinned-hash refresh in the
# same diff. A bump that touches only the lockfile/manifest fails with an
# actionable message naming the derivation and the hash to refresh.
#
# Coverage (see --list): Go vendorHash derivations, npm npmDepsHash
# derivations (including nested lockfiles such as
# pkgs/cold-store-reaper/worker), Cargo crane derivations (pkgs/id), and
# standalone flake.lock files (pkgs/id/flake.lock).
#
# Deliberate exclusions (not oversights):
#   * pkgs/latest-cli: prebuilt release binaries — version and hash live in
#     the same file, so a companion-change check cannot distinguish a
#     version bump from its hash refresh. Needs content-hash verification.
#   * Cross-repo locks (the openchamber standalone flake.lock): another
#     repository; this repo's CI cannot gate it.
#   * Automatic hash rewriting (bot): explicitly out of scope for slice 1;
#     this gate blocks stale merges, it does not rewrite them.
#   * #14315 id-store adaptation: tracked separately, not this slice.
#
# Venue: nightly schedule + workflow_dispatch
# (see .github/workflows/nix-dep-gate.yml). Repo CI doctrine is
# nightly-main, not PR-tier (see .github/ci-policy.md), so pre-merge
# gating is left to local invocation: run this script with
# --base/--head spanning the PR diff before requesting review.
#
# Usage:
#   scripts/check-nix-dep-gate.sh [options]
#
# Options:
#   --base <ref>    Diff base ref (default: origin/main).
#   --head <ref>    Diff head ref (default: HEAD).
#   --repo <dir>    Repository root to inspect (default: this repo).
#   --list          Print the covered derivation table and exit 0.
#   -h, --help      Show this help message.
#
# Exit codes:
#   0: Clean — no stale dependency bump in the diff.
#   1: STALE — a dependency input changed without its hash refresh.
#   2: Usage error, unreadable repo, or uncomputable diff (fail closed:
#      an unreadable tree is never "clean").
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/check-nix-dep-gate.sh [options]

Gate dependency bumps on their pinned Nix hash refresh (#14195): any
change to a derivation's dependency inputs must ship with the hash
refresh in the same diff.

Options:
  --base <ref>    Diff base ref (default: origin/main).
  --head <ref>    Diff head ref (default: HEAD).
  --repo <dir>    Repository root to inspect (default: this repo).
  --list          Print the covered derivation table and exit 0.
  -h, --help      Show this help message.

Exit codes:
  0: Clean — no stale dependency bump in the diff.
  1: STALE — a dependency input changed without its hash refresh.
  2: Usage error, unreadable repo, or uncomputable diff (fail closed).
EOF
}

# Manifest: derivation-dir | space-separated inputs | space-separated
# companions | refresh hint. All paths repo-relative.
manifest() {
  cat <<'EOF'
pkgs/mcpx | go.mod go.sum | package.nix | refresh vendorHash in pkgs/mcpx/package.nix (nix build .#mcpx with an empty hash to obtain the fresh one)
pkgs/identity | go.mod go.sum | package.nix | refresh vendorHash in pkgs/identity/package.nix
pkgs/fleet/placement | go.mod go.sum | package.nix | refresh vendorHash in pkgs/fleet/placement/package.nix
pkgs/grok-cli | package.json package-lock.json | default.nix | regenerate package-lock.json from the published package.json, then refresh npmDepsHash in pkgs/grok-cli/default.nix
pkgs/wrangler | package.json package-lock.json | default.nix | refresh npmDepsHash in pkgs/wrangler/default.nix
pkgs/lab-ops-hub | package.json package-lock.json | package.nix | refresh npmDepsHash in pkgs/lab-ops-hub/package.nix
pkgs/cf | package.json package-lock.json | default.nix | refresh npmDepsHash in pkgs/cf/default.nix
pkgs/command-code | package.json package-lock.json | default.nix | refresh npmDepsHash in pkgs/command-code/default.nix
pkgs/desktop-commander | package.json package-lock.json | default.nix | refresh npmDepsHash in pkgs/desktop-commander/default.nix
pkgs/cf-container-hello | package.json package-lock.json | package.nix | refresh npmDepsHash in pkgs/cf-container-hello/package.nix
pkgs/cold-store-reaper/worker | package.json package-lock.json | package.nix | refresh npmDepsHash in pkgs/cold-store-reaper/worker/package.nix
pkgs/supergateway | package.json package-lock.json | default.nix | verify the lockfile installs against the published tarball manifest first (that class diverges, #14140), then refresh npmDepsHash in pkgs/supergateway/default.nix
pkgs/id | Cargo.toml Cargo.lock | default.nix | update the crane hash inputs covering pkgs/id/Cargo.lock in pkgs/id/default.nix
pkgs/id | flake.nix | flake.lock | re-lock pkgs/id/flake.lock against the new inputs
EOF
}

default_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base="origin/main"
head="HEAD"
repo="$default_repo"
list_only=0

while [[ $# -gt 0 ]]; do
  case "$1" in
  --base)
    if [[ $# -lt 2 || -z ${2:-} ]]; then
      echo "::error::--base requires a non-empty ref." >&2
      exit 2
    fi
    base="$2"
    shift 2
    ;;
  --base=*)
    if [[ -z ${1#--base=} ]]; then
      echo "::error::--base requires a non-empty ref." >&2
      exit 2
    fi
    base="${1#--base=}"
    shift
    ;;
  --head)
    if [[ $# -lt 2 || -z ${2:-} ]]; then
      echo "::error::--head requires a non-empty ref." >&2
      exit 2
    fi
    head="$2"
    shift 2
    ;;
  --head=*)
    if [[ -z ${1#--head=} ]]; then
      echo "::error::--head requires a non-empty ref." >&2
      exit 2
    fi
    head="${1#--head=}"
    shift
    ;;
  --repo)
    if [[ $# -lt 2 || -z ${2:-} ]]; then
      echo "::error::--repo requires a non-empty directory." >&2
      exit 2
    fi
    repo="$2"
    shift 2
    ;;
  --repo=*)
    if [[ -z ${1#--repo=} ]]; then
      echo "::error::--repo requires a non-empty directory." >&2
      exit 2
    fi
    repo="${1#--repo=}"
    shift
    ;;
  --list)
    list_only=1
    shift
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  -*)
    echo "::error::Unknown option: $1" >&2
    usage >&2
    exit 2
    ;;
  *)
    echo "::error::Unexpected positional argument: $1 (this gate takes options only)" >&2
    usage >&2
    exit 2
    ;;
  esac
done

if [[ $list_only -eq 1 ]]; then
  printf '%-32s | %-32s | %-22s\n' "DERIVATION" "INPUTS" "COMPANION"
  manifest | while IFS='|' read -r dir inputs companions _hint; do
    printf '%-32s | %-32s | %-22s\n' "$(echo "$dir" | tr -d ' ')" "$(echo "$inputs" | tr -s ' ')" "$(echo "$companions" | tr -s ' ')"
  done
  exit 0
fi

if [[ ! -d $repo/.git && ! -f $repo/.git ]]; then
  echo "::error::Not a git repository: $repo" >&2
  exit 2
fi

# Fail closed: an uncomputable diff is never "clean" (same stance as the
# retirement gate's unreadable-config rule, #5610).
changed="$(git -C "$repo" diff --name-only "$base" "$head" -- 2>/dev/null)" || {
  echo "::error::Cannot compute diff $base..$head in $repo; failing closed (exit 2)." >&2
  exit 2
}

violations=0
checked=0
while IFS='|' read -r dir inputs companions hint; do
  dir="$(echo "$dir" | tr -d '[:space:]')"
  [[ -z $dir ]] && continue
  checked=$((checked + 1))
  input_hit=""
  for input in $inputs; do
    input="$(echo "$input" | tr -d '[:space:]')"
    [[ -z $input ]] && continue
    if printf '%s\n' "$changed" | grep -qxF "$dir/$input"; then
      input_hit="$dir/$input"
      break
    fi
  done
  [[ -z $input_hit ]] && continue
  companion_hit=""
  for companion in $companions; do
    companion="$(echo "$companion" | tr -d '[:space:]')"
    [[ -z $companion ]] && continue
    if printf '%s\n' "$changed" | grep -qxF "$dir/$companion"; then
      companion_hit="$dir/$companion"
      break
    fi
  done
  if [[ -z $companion_hit ]]; then
    echo "::error::Stale Nix hash (#14195): $input_hit changed without its pinned-hash refresh." >&2
    echo "  Derivation: $dir — hint: $(echo "$hint" | sed 's/^ *//')" >&2
    violations=$((violations + 1))
  else
    echo "Dep gate OK: $input_hit ships with $companion_hit."
  fi
done < <(manifest)

if [[ $violations -gt 0 ]]; then
  echo "::error::Nix dependency gate FAILED: $violations stale bump(s) in $base..$head (#14195)." >&2
  exit 1
fi

echo "Nix dependency gate OK: no stale bump across $checked covered derivation(s) ($base..$head)."
exit 0
