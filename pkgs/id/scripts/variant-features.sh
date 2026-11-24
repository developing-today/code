#!/usr/bin/env bash
# Print one build variant's cargo feature set from the single profile
# source (composition/variants.toml, S2 of #14056 / plan §7.4).
#
# Usage: variant-features.sh <web|lib> [--minus-defaults]
#   <variant>          the variant name in composition/variants.toml.
#   --minus-defaults   omit the root default features (for callers that
#                      build WITH cargo defaults on, e.g. a bare
#                      `cargo build --features ...`).
#
# Output: a comma-separated feature list on stdout (no trailing newline
# on the value, one terminating newline), suitable for
# `--features "$(variant-features.sh web)"`.
#
# Operator-script argument contract (#3352 / #10567): the whole command
# line is declared above; anything else is a usage error before any file
# read.
set -euo pipefail

usage() {
  echo "Usage: variant-features.sh <web|lib> [--minus-defaults]" >&2
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  echo "Error: expected 1 or 2 arguments, got $#" >&2
  exit 1
fi

variant="$1"
minus_defaults=false
if [[ $# -eq 2 ]]; then
  if [[ "$2" != "--minus-defaults" ]]; then
    usage
    echo "Error: unknown option '$2'" >&2
    exit 1
  fi
  minus_defaults=true
fi

here="$(cd "$(dirname "$0")" && pwd)"
source_file="$here/../composition/variants.toml"
if [[ ! -f "$source_file" ]]; then
  echo "Error: profile source not found: $source_file" >&2
  exit 1
fi

python3 -I - "$source_file" "$variant" "$minus_defaults" <<'PY'
import sys, tomllib
path, variant, minus = sys.argv[1], sys.argv[2], sys.argv[3] == "true"
with open(path, "rb") as f:
    data = tomllib.load(f)
variants = data.get("variant", {})
if variant not in variants:
    print(f"Error: unknown variant '{variant}'; known: {sorted(variants)}", file=sys.stderr)
    sys.exit(1)
features = list(variants[variant].get("features", []))
if minus:
    default = list(data.get("root", {}).get("default", []))
    features = [f for f in features if f not in default]
print(",".join(features))
PY
