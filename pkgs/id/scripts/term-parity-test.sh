#!/usr/bin/env bash
# Cross-language parity oracle for the primary-mode painter contract
# (#14861 slice 2, issue #14936).
#
# The shared frame fixtures (test-data/paint-primary-fixtures.json) are
# painted with the ACTUAL Roc contract: this script generates two
# `expect`s per case into a scratch copy of examples/roc-world/Term.roc
# and runs the pinned `roc test` over it — Term.paint_primary and
# Term.paint_primary_full must reproduce the pinned bytes for every
# fixture frame. The same fixtures pin the host-side Rust painter
# (ScreenSurface::paint_primary, crates/id-core tests), so the two
# implementations cannot drift. There is no second handwritten painter
# anywhere in this check.
#
# Term.roc imports Screen, which imports RocGrid, so the scratch copy
# carries all three modules; imports resolve beside the copy.
#
# Missing pinned artifacts (roc, python3, the fixtures, the Roc
# modules) FAIL this script; the oracle never silently skips.
# Usage: term-parity-test.sh — accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: term-parity-test.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

fail() {
  echo "term-parity-test: $*" >&2
  exit 1
}

if ! command -v roc >/dev/null 2>&1; then
  fail "the pinned Roc compiler (nix/roc.nix) is not on PATH. Run inside 'nix develop' or the roc-test flake check — refusing to skip silently (#14936)."
fi
if ! command -v python3 >/dev/null 2>&1; then
  fail "python3 is not on PATH — the fixture generator cannot run; refusing to skip silently (#14936)."
fi
[[ -f test-data/paint-primary-fixtures.json ]] ||
  fail "the shared fixtures test-data/paint-primary-fixtures.json are missing; refusing to skip silently (#14936)."
for module in Term Screen RocGrid; do
  [[ -f "examples/roc-world/${module}.roc" ]] ||
    fail "examples/roc-world/${module}.roc is missing; refusing to skip silently (#14936)."
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cp examples/roc-world/Screen.roc "${tmp}/Screen.roc"
cp examples/roc-world/RocGrid.roc "${tmp}/RocGrid.roc"
python3 scripts/term-parity-generate.py \
  test-data/paint-primary-fixtures.json \
  examples/roc-world/Term.roc \
  "${tmp}/Term.roc"

echo "=== roc test (generated parity copy of examples/roc-world/Term.roc) ==="
roc test "${tmp}/Term.roc"
echo "✓ term parity: the Roc Term contract reproduces every pinned paint-primary fixture byte string"
