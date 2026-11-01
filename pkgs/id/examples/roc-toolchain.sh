#!/usr/bin/env bash
# Shared compiler guard for every checked-in Roc example module.

ID_ROC_VERSION_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/nix/roc-version"
ID_ROC_EXPECTED_VERSION="$(<"$ID_ROC_VERSION_FILE")"
ID_ROC_EXPECTED_OUTPUT="Roc compiler version $ID_ROC_EXPECTED_VERSION"
ID_ROC_BIN="${ROC:-roc}"

if ! ID_ROC_ACTUAL_OUTPUT="$("$ID_ROC_BIN" version 2>&1)"; then
  printf 'Roc toolchain check failed: could not run %s version:\n%s\n' \
    "$ID_ROC_BIN" "$ID_ROC_ACTUAL_OUTPUT" >&2
  exit 1
fi

if [[ $ID_ROC_ACTUAL_OUTPUT != "$ID_ROC_EXPECTED_OUTPUT" ]]; then
  printf 'Roc toolchain mismatch: expected %s from %s, got %s from %s; refusing to build.\n' \
    "$ID_ROC_EXPECTED_OUTPUT" "$ID_ROC_VERSION_FILE" "$ID_ROC_ACTUAL_OUTPUT" "$ID_ROC_BIN" >&2
  exit 1
fi

echo "Using pinned Roc compiler $ID_ROC_EXPECTED_VERSION"
