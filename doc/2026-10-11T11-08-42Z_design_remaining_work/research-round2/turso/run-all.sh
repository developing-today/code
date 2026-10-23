#!/usr/bin/env bash
# Session 2 measurements: the three stores in one session, one binary, one load window
# (as far as the shared machine allows). Outputs go to s6-index/run/results/*-session2.txt.
set -u
export RUSTUP_TOOLCHAIN=1.97.0
cd "$(dirname "$0")/s6-index" || exit 1
B=./target/release/s6-index
R=run/results
mark() { echo "--- $1 | $(date -u +%H:%M:%S) | $(uptime)"; }

{
  mark "start"
  echo "nproc=$(nproc) mem=$(free -g | awk '/Mem:/{print $2" GiB total"}')"
  echo "binary: $(sha256sum $B | cut -c1-16)  cargo=$(rustup which --toolchain 1.97.0 cargo)"
  for s in redb sqlite turso; do
    mark "bench $s"
    ( time $B bench $s run/bench-$s 200000 ) 2>&1
    echo "exit=$?"
    echo "files after close:"; ls -l run/bench-$s/ | tail -n +2
  done
  mark "compact redb (same file as the bench)"
  $B compact-redb run/bench-redb/index-redb.db 2>&1
  for s in redb sqlite turso; do
    mark "durability $s 20 trials"
    $B durable-test $s 20 run/durability-$s 2>&1
  done
  mark "concurrency (redb, sqlite; S6 orchestrator)"
  $B concurrency run/concurrency-session2 2>&1
  mark "turso concurrency"
  $B turso-concurrency run/conc-turso 2>&1
  mark "turso multiprocess WAL, two writers"
  $B turso-mp-test run/mp-turso 2>&1
  mark "done"
} > $R/session2-all.txt 2>&1
echo "finished $(date -u +%FT%TZ)" >> $R/session2-all.txt
