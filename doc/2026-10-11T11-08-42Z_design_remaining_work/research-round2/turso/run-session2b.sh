#!/usr/bin/env bash
# Session 2b: the final binary, every step under its own timeout. The Turso WITHOUT ROWID
# bench is not repeated at 200k: its planner scans every lookup (see turso-plan), and the
# first attempt stalled. Its numbers come from the stress run and turso-plan.
set -u
export RUSTUP_TOOLCHAIN=1.97.0
cd "$(dirname "$0")/s6-index" || exit 1
B=./target/release/s6-index
R=run/results
mark() { echo "--- $1 | $(date -u +%H:%M:%S)Z | $(uptime)"; }
step() { # step <label> <timeout-seconds> <command...>
  local label=$1 to=$2; shift 2
  mark "$label"
  ( time timeout "$to" "$@" ) 2>&1
  echo "exit=$?"
}
{
  mark "start"
  echo "binary: $(sha256sum $B | cut -c1-16)"
  step "unique turso-rowid" 120 $B unique turso-rowid run/unique-rowid
  step "bench turso-rowid 200000" 900 $B bench turso-rowid run/bench-turso-rowid 200000
  step "bench redb 200000" 300 $B bench redb run/bench-redb 200000
  step "bench sqlite 200000" 300 $B bench sqlite run/bench-sqlite 200000
  step "compact-redb" 300 $B compact-redb run/bench-redb/index-redb.db
  step "durability redb 20" 900 $B durable-test redb 20 run/durability-redb
  step "durability sqlite 20" 900 $B durable-test sqlite 20 run/durability-sqlite
  step "durability turso-rowid 20" 900 $B durable-test turso-rowid 20 run/durability-turso-rowid
  step "durability turso (WITHOUT ROWID) 20" 900 $B durable-test turso 20 run/durability-turso
  step "concurrency redb/sqlite (S6 orchestrator)" 300 $B concurrency run/concurrency-session2
  step "turso concurrency (open-time lock)" 300 $B turso-concurrency run/conc-turso
  step "turso multiprocess WAL, two writers" 300 $B turso-mp-test run/mp-turso
  mark "done"
} > $R/session2b-all.txt 2>&1
echo "finished $(date -u +%FT%TZ)" >> $R/session2b-all.txt
