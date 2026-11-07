#!/usr/bin/env bash
set -u
cd ~/git/fleet/5375/pkgs/id
MOLD=/nix/store/9dcddrfxlfl9n3wfibi5jkmp22bjkvkh-mold-unwrapped-wrapper-2.42.1/bin/mold
run() {
  name=$1; shift
  td=/tmp/5375-target-$name
  rm -rf "$td"
  echo "=== RUN $name start $(date -u +%FT%TZ) load: $(cat /proc/loadavg) ==="
  s=$SECONDS
  CARGO_TARGET_DIR="$td" "$@" cargo check --profile ci-check --all-targets > /tmp/5375-check-$name.log 2>&1
  rc=$?
  echo "=== RUN $name end rc=$rc elapsed=$((SECONDS-s))s load: $(cat /proc/loadavg) ==="
}
~/tools/fleet-build-lock bash -c '
cd ~/git/fleet/5375/pkgs/id
td=/tmp/5375-target-default; rm -rf "$td"
echo "=== RUN default start $(date -u +%FT%TZ) load: $(cat /proc/loadavg) ==="
s=$SECONDS
CARGO_TARGET_DIR="$td" cargo check --profile ci-check --all-targets > /tmp/5375-check-default.log 2>&1
echo "=== RUN default end rc=$? elapsed=$((SECONDS-s))s load: $(cat /proc/loadavg) ==="
td=/tmp/5375-target-mold; rm -rf "$td"
echo "=== RUN mold start $(date -u +%FT%TZ) load: $(cat /proc/loadavg) ==="
s=$SECONDS
CARGO_TARGET_DIR="$td" RUSTFLAGS="-C link-arg=-fuse-ld=/nix/store/9dcddrfxlfl9n3wfibi5jkmp22bjkvkh-mold-unwrapped-wrapper-2.42.1/bin/mold" cargo check --profile ci-check --all-targets > /tmp/5375-check-mold.log 2>&1
echo "=== RUN mold end rc=$? elapsed=$((SECONDS-s))s load: $(cat /proc/loadavg) ==="
echo ALLDONE
' > /tmp/5375-measure.log 2>&1
