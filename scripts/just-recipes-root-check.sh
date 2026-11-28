#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'usage: %s\n' "$(basename "$0")" >&2
}

if [[ $# -ne 0 ]]; then
  printf '%s: unexpected argument: %s\n' "$(basename "$0")" "$1" >&2
  usage
  exit 2
fi

# Anchor to this script's own tree: `just --dump` and the committed
# just-recipes.json are only meaningful for the root justfile, so
# resolve them there regardless of the caller's CWD. Without this,
# invoking the script by path from elsewhere either dies on a missing
# CWD-relative file or - worse - certifies whatever just-recipes.json
# the CWD happens to hold (the pkgs/id guard's #6957 lesson, applied
# to the root file under #8029).
cd "$(dirname "${BASH_SOURCE[0]}")/.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
just --dump --dump-format json >"$tmp/fresh.json"
# Compare semantic fields only: recipe presence, doc, parameters,
# dependencies and the private flag, plus alias targets. The top-level
# `source` is an absolute path from the generating machine and recipe
# bodies serialise differently between just versions, so both are
# projected out — a byte diff would false-positive forever (the same
# projection as pkgs/id/scripts/just-recipes-check.sh, #2729).
project() {
  jq -S '{recipes: (.recipes | to_entries | map({key, value: {doc: .value.doc, parameters: .value.parameters, dependencies: .value.dependencies, private: (.value.private // false)}}) | from_entries), aliases: ((.aliases // {}) | to_entries | map({key, value: .value.target}) | from_entries)}' "$1"
}
# Project separately: failures inside process substitutions are otherwise hidden
# by diff's exit status and can incorrectly report a match.
project just-recipes.json >"$tmp/committed.json"
project "$tmp/fresh.json" >"$tmp/projected.json"
if ! diff "$tmp/committed.json" "$tmp/projected.json"; then
  echo "error: root just-recipes.json has drifted from the justfile — run 'just just-recipes' and commit the result" >&2
  exit 1
fi
echo "✓ root just-recipes.json matches the justfile"
