#!/usr/bin/env bash
# Run the #5994 §4 fixture matrix for the Roc library import gate
# (#5956 §7 step 4, #10027): the gate-level cases in fixtures.json,
# executed by the id-caps integration test of the same name (manifest
# parse → transitive resolution → import gate, each negative case
# asserting its exact diagnostic — subreason and excess set).
#
# The matrix's two Roc-compile negative fixtures (missing module,
# wrong signature) are the roc-cap-demo tests, run separately by
# roc-test.sh; this script covers the rows the gate decides.
set -euo pipefail

cd "$(dirname "$0")/../.."

if ! command -v cargo >/dev/null 2>&1; then
  echo "roc-import-policy: cargo is not on PATH — refusing to skip silently." >&2
  echo "roc-import-policy: run inside the pkgs/id dev shell, or run 'cargo test -p id-caps --test roc_import_policy' directly." >&2
  exit 1
fi
cargo test -p id-caps --test roc_import_policy
echo "✓ Roc import policy fixtures passed (see fixtures.json for the matrix)"
