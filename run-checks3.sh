#!/usr/bin/env bash
# lane-scratch: openchamber scheduled lane code-i, task 20651, verify3 at bcb79c8e44
# Re-run of the 21 checks named in run 43742380258 at current id main.
# Results: check-results-main3.txt (append-only, with START/PASS/FAIL + UTC times),
# per-check nix logs: log3-<check>.txt.
set -u
cd ~/git/fleet/queue-20651-10533-verify3/pkgs/id || exit 9
OUT=~/git/fleet/queue-20651-10533-verify3/check-results-main3.txt
: > "$OUT"
echo "head $(git rev-parse HEAD) started $(date -u +%FT%TZ)" >> "$OUT"
for c in biome-check cargo-check cargo-fmt-check clippy-lint clippy-native-shell \
         crash-gate-control crash-gate-ingest crash-gate-sampled crash-gate-sweep \
         nix-fmt-check prettier-check prod-lines-check rustfmt-check \
         test-shard-0 test-shard-1 test-shard-2 test-shard-3 test-unit test-web-unit \
         treefmt-check web-fmt-check; do
  echo "START $c $(date -u +%FT%TZ)" >> "$OUT"
  if nix build ".#checks.x86_64-linux.$c" --no-link > ~/git/fleet/queue-20651-10533-verify3/log3-$c.txt 2>&1; then
    echo "PASS $c $(date -u +%FT%TZ)" >> "$OUT"
  else
    echo "FAIL $c $(date -u +%FT%TZ)" >> "$OUT"
  fi
done
echo "DONE $(date -u +%FT%TZ)" >> "$OUT"
