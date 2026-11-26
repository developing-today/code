#!/usr/bin/env bash
# Temporary verification script for the #803 final slice. NOT part of the PR;
# removed before the PR is opened. Warning-mode clippy census, JSON counts.
set -u
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="$PWD/.census-target"
mkdir -p "$CARGO_TARGET_DIR"

count_crate() {
  pkg="$1"
  feats="$2"
  log="/tmp/census-$pkg.json"
  if [ -n "$feats" ]; then
    cargo clippy -p "$pkg" --all-targets --keep-going --features "$feats" \
      --message-format json -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "$log" 2>/dev/null
  else
    cargo clippy -p "$pkg" --all-targets --keep-going \
      --message-format json -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "$log" 2>/dev/null
  fi
  echo "clippy exit for $pkg: $?"
  python3 - "$log" "$pkg" <<'PYEOF'
import json, sys
log, pkg = sys.argv[1], sys.argv[2]
counts = {}
sites = []
for line in open(log):
    try:
        d = json.loads(line)
    except Exception:
        continue
    msg = d.get("message") or {}
    code = (msg.get("code") or {}).get("code", "")
    if code in ("clippy::assert_is_empty", "clippy::chunks_exact_to_as_chunks"):
        counts[code] = counts.get(code, 0) + 1
        for sp in msg.get("spans", []):
            if sp.get("is_primary"):
                sites.append("%s %s:%s" % (code, sp["file_name"], sp["line_start"]))
print("CENSUS %s assert_is_empty=%d chunks=%d" % (
    pkg,
    counts.get("clippy::assert_is_empty", 0),
    counts.get("clippy::chunks_exact_to_as_chunks", 0),
))
for s in sites:
    print("SITE %s %s" % (pkg, s))
PYEOF
}

count_crate id-core "web,test-utils,test-provider"
count_crate id-web "sandbox"
count_crate id-import-notion ""
count_crate id-caps ""
count_crate id-events ""
echo "CENSUS DONE"
