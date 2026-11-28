#!/usr/bin/env bash
# Run the Roc library expect() tests (RocJson, Screen, Key) with the
# pinned Roc compiler (#813). Invoked as `bash scripts/roc-test.sh` by
# the roc-test just recipe: a just shebang recipe cannot execute in
# the Nix check sandbox, which has no /usr/bin/env (os error 2).
# Usage: roc-test.sh
#   Accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: roc-test.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

if ! command -v roc >/dev/null 2>&1; then
  echo "roc-test: the pinned Roc compiler (nix/roc.nix) is not on PATH." >&2
  echo "roc-test: run inside 'nix develop' or the roc-test flake check — refusing to skip silently (#813)." >&2
  exit 1
fi
echo "roc-test: $(roc version)"
for module in RocJson Screen Key; do
  echo "=== roc test examples/roc-world/${module}.roc ==="
  roc test "examples/roc-world/${module}.roc"
done
echo "=== key-parity: kitty-disambiguate-v1 corpus vs the actual Roc decoder (#787 Slice D) ==="
bash scripts/key-parity-test.sh
echo "=== roc-cap-demo: capability-dependent library + app, both negative tests (#5957) ==="
bash examples/roc-cap-demo/roc-cap-demo-test.sh
echo "=== roc-import-policy: #5994 section 4 fixture matrix for the import gate (#10027) ==="
if command -v cargo >/dev/null 2>&1; then
  bash examples/roc-import-policy/roc-import-policy-test.sh
else
  echo "roc-test: cargo not on PATH - the roc-import-policy fixtures run in the cargo test suite (cargo test -p id-caps --test roc_import_policy); not skipped silently."
fi
echo "✓ Roc library expectations passed (RocJson, Screen, Key; roc-cap-demo positive + negative tests)"
