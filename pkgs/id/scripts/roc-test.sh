#!/usr/bin/env bash
# Run the Roc library expect() tests (RocJson, Screen, Key) with the
# pinned Roc compiler (#813). Invoked as `bash scripts/roc-test.sh` by
# the roc-test just recipe: a just shebang recipe cannot execute in
# the Nix check sandbox, which has no /usr/bin/env (os error 2).
set -euo pipefail

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
echo "✓ Roc library expectations passed (RocJson, Screen, Key)"
