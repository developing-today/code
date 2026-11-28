#!/usr/bin/env bash
# Cross-language parity oracle for the kitty-disambiguate-v1 profile
# (#787 Slice D, design doc/2026-11-01_design_id787.md §6 row D).
#
# The shared corpus (doc/kitty-disambiguate-v1-fixtures.json) is decoded
# with the ACTUAL Roc decoder: this script generates one `expect` per
# corpus case into a scratch copy of examples/roc-world/Key.roc and
# runs the pinned `roc test` over it — terminal bytes and, for cases
# with a `browser` field, the browser encoder's bytes for the same
# keystroke must both decode to the corpus's expected rich events.
# There is no second handwritten decoder anywhere in this check.
#
# Missing pinned artifacts (roc, python3, the corpus, Key.roc) FAIL
# this script; the oracle never silently skips (design §6 row D).
# Usage: key-parity-test.sh — accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: key-parity-test.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

fail() {
  echo "key-parity-test: $*" >&2
  exit 1
}

if ! command -v roc >/dev/null 2>&1; then
  fail "the pinned Roc compiler (nix/roc.nix) is not on PATH. Run inside 'nix develop' or the roc-test flake check — refusing to skip silently (#787 Slice D)."
fi
if ! command -v python3 >/dev/null 2>&1; then
  fail "python3 is not on PATH — the corpus generator cannot run; refusing to skip silently (#787 Slice D)."
fi
[[ -f doc/kitty-disambiguate-v1-fixtures.json ]] ||
  fail "the shared corpus doc/kitty-disambiguate-v1-fixtures.json is missing; refusing to skip silently (#787 Slice D)."
[[ -f examples/roc-world/Key.roc ]] ||
  fail "examples/roc-world/Key.roc is missing; refusing to skip silently (#787 Slice D)."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

python3 scripts/key-parity-generate.py \
  doc/kitty-disambiguate-v1-fixtures.json \
  examples/roc-world/Key.roc \
  "${tmp}/Key.roc"

echo "=== roc test (generated parity copy of examples/roc-world/Key.roc) ==="
roc test "${tmp}/Key.roc"
echo "✓ key parity: terminal corpus and browser encoder output decode to equal rich events under the actual Roc parse_events"
