#!/usr/bin/env bash
set -euo pipefail
fresh="$(mktemp)"
trap 'rm -f "$fresh"' EXIT
just --dump --dump-format json > "$fresh"
# Compare semantic fields only: recipe presence, doc, parameters,
# dependencies and the private flag, plus alias targets. The top-level
# `source` is an absolute path from the generating machine and recipe
# bodies serialise differently between just versions (the `bench`
# nesting, #2729), so both are projected out — a byte diff would
# false-positive forever.
project() {
    jq -S '{recipes: (.recipes | to_entries | map({key, value: {doc: .value.doc, parameters: [.value.parameters[]? | {name, kind, default}], dependencies: [.value.dependencies[]? | .recipe], private: (.value.private // false)}}) | from_entries), aliases: ((.aliases // {}) | to_entries | map({key, value: .value.target}) | from_entries)}' "$1"
}
if ! diff <(project just-recipes.json) <(project "$fresh"); then
    echo "error: just-recipes.json has drifted from the justfile — run 'just lockfiles' and commit the result" >&2
    exit 1
fi
echo "✓ just-recipes.json matches the justfile"

