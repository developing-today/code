#!/usr/bin/env bash
# lane-scratch: n/a (repo tool) — restore row-count diff gate (#6994)
#
# Post-restore row-count diff gate for selective (per-table) restores.
# Companion to doc/fleet/restore-procedure.md. The gate compares the
# export manifest against live per-table counts and fails (exit 1) on:
#   - a source-inventory table with no manifest row (silence is not a
#     disposition),
#   - an archive/retire row with no destination/reason note,
#   - a load table missing from the live counts,
#   - a load table whose live count is short of the source count by
#     more than its allowlisted explained delta,
#   - a load table that is non-empty in the source and empty live,
#     unless the shortfall is fully allowlisted.
# Live counts ABOVE source are reported as info (post-cutover writes
# are expected), never a failure.
#
# Inputs are TSV files so any store can feed the gate: run the query
# set in restore-row-count-queries.sql against the snapshot and the
# live store, or generate the same shapes from a pg_dump / Turso dump.
#
# Manifest TSV (header required):
#   table <TAB> source_count <TAB> disposition <TAB> allowed_delta <TAB> note
#   disposition: load | archive | retire
#   allowed_delta: explained missing rows for a load row (integer >= 0)
#   note: REQUIRED for archive (destination) and retire (reason);
#         the explanation for any allowed_delta > 0.
# Live counts TSV (header required):
#   table <TAB> live_count
# Inventory (optional, one table name per line): every source table in
#   scope; the manifest must cover each one.
#
# Usage:
#   restore-row-count-diff.sh --manifest MANIFEST.tsv --live LIVE.tsv #       [--inventory INVENTORY.txt]
# Exit: 0 = gate passes, 1 = gate fails, 2 = usage/input error.
set -euo pipefail

manifest=""
live=""
inventory=""
while [ $# -gt 0 ]; do
  case "$1" in
  --manifest)
    manifest="$2"
    shift 2
    ;;
  --live)
    live="$2"
    shift 2
    ;;
  --inventory)
    inventory="$2"
    shift 2
    ;;
  *)
    echo "unknown argument: $1" >&2
    exit 2
    ;;
  esac
done
[ -n "$manifest" ] && [ -n "$live" ] || {
  echo "usage: restore-row-count-diff.sh --manifest M.tsv --live L.tsv [--inventory I.txt]" >&2
  exit 2
}
[ -f "$manifest" ] && [ -f "$live" ] || {
  echo "input file missing" >&2
  exit 2
}
# An empty counts export must never reach the comparison: with no live
# rows at all the gate has nothing to verify, and a failed export would
# otherwise be indistinguishable from a pass (Codex review, PR #8406).
[ -s "$live" ] || {
  echo "live counts file is empty: $live" >&2
  exit 2
}
[ -s "$manifest" ] || {
  echo "manifest file is empty: $manifest" >&2
  exit 2
}
if [ -n "$inventory" ] && [ ! -f "$inventory" ]; then
  echo "inventory file missing: $inventory" >&2
  exit 2
fi

# The whole comparison is one awk pass: manifest and live counts are
# loaded first, the inventory check runs at END.
exec awk -v inventory="$inventory" '
  BEGIN { FS = "\t"; fails = 0; infos = 0 }
  function fail(msg) { printf "FAIL  %s\n", msg; fails++ }
  function info(msg) { printf "INFO  %s\n", msg; infos++ }
  # Pass 1: live counts (the first file argument). Identified by
  # filename, not FNR == NR: with an empty first file the counters
  # never diverge and manifest rows would be parsed as live counts.
  FILENAME == ARGV[1] {
    if (FNR == 1) next
    live[$1] = $2 + 0; seen_live[$1] = 1
    next
  }
  # Pass 2: the manifest.
  {
    if (FNR == 1) next
    table = $1; src = $2 + 0; disp = $3; allow = $4 + 0; note = $5
    in_manifest[table] = 1
    if (disp == "archive" || disp == "retire") {
      if (note == "") {
        fail(table ": disposition " disp " has no destination/reason note")
      } else {
        info(table ": " disp " (" note ") — not loaded, by disposition")
      }
      next
    }
    if (disp != "load") {
      fail(table ": unknown disposition " disp)
      next
    }
    if (allow > 0 && note == "") {
      fail(table ": load row allowlists a delta of " allow " with no explanation note")
      next
    }
    if (!(table in seen_live)) {
      fail(table ": in manifest as load but missing from live counts")
      next
    }
    lv = live[table]
    if (src > 0 && lv == 0 && allow < src) {
      fail(table ": non-empty in source (" src ") but EMPTY live")
      next
    }
    if (lv < src - allow) {
      fail(table ": live " lv " is short of source " src " by " (src - lv) "; allowlisted delta is only " allow)
      next
    }
    if (lv < src) {
      info(table ": live " lv " vs source " src " — shortfall " (src - lv) " within allowlisted delta " allow " (" note ")")
    } else if (lv > src) {
      info(table ": live " lv " vs source " src " — +" (lv - src) " (post-cutover writes)")
    } else {
      info(table ": live " lv " == source " src)
    }
  }
  END {
    if (inventory != "") {
      while ((getline line < inventory) > 0) {
        if (line == "") continue
        if (!(line in in_manifest)) {
          fail(line ": in source inventory but MISSING from the manifest — silence is not a disposition")
        }
      }
    }
    printf "----\n%s: %d failure(s), %d info line(s)\n", (fails > 0 ? "GATE FAILED" : "GATE PASSED"), fails, infos
    exit (fails > 0 ? 1 : 0)
  }
' "$live" "$manifest"
