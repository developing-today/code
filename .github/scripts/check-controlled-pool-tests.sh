#!/usr/bin/env bash
# check-controlled-pool-tests.sh — the controlled-pool-tests
# reference guard (#1342, #1334).
#
# The procedure (doc/agents/controlled-pool-tests.md) pins test arms
# by pool route and identifies them from the queue's claim stamps and
# route_usage readout; the results template
# (doc/agents/controlled-pool-test-template.md) carries the per-arm
# measurements a verdict needs. If the queue code renames those
# symbols, or a required section/field goes missing, the procedure
# silently goes stale — a lane would pin an arm the code can no
# longer stamp, or record a verdict the template cannot hold.
#
# This guard fails — naming the offender — when:
#   (a) the procedure doc is missing or lacks a required section
#       (Authorization gate, Batch design, Route pinning,
#       Measurements, Verdict, Records), or its Route pinning
#       section no longer requires launching each arm as a
#       separate session on its managed route before claiming
#       (the Codex P1 on #13419: --pool declares pool identity
#       only and does not launch a route);
#   (b) the template is missing or lacks a required field (flag
#       issue, candidate/incumbent route, tasks per arm, completion,
#       wall-clock, tokens, stalls/nudges, rework within 48h,
#       verdict, headroom evidence);
#   (c) a load-bearing code reference the procedure cites no longer
#       exists: stamp_claim_model, the claim_model TEXT column, or
#       derive_completion_telemetry / lastServing.
#
# Argument contract (#3352): the whole command line is one optional
# positional, REPO_ROOT (default: this repo). Any extra argument is
# a usage error naming the offender, exit 2.
#
# Usage: check-controlled-pool-tests.sh [REPO_ROOT]
# Exit: 0 = procedure, template and code agree; 1 = violations
# (named on stdout); 2 = usage error.
set -euo pipefail

if [ "$#" -gt 1 ]; then
  echo "usage: check-controlled-pool-tests.sh [REPO_ROOT]" >&2
  echo "check-controlled-pool-tests: usage error: unexpected argument(s): ${*:2}" >&2
  exit 2
fi

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
PROC="$ROOT/doc/agents/controlled-pool-tests.md"
TMPL="$ROOT/doc/agents/controlled-pool-test-template.md"
CLAIM="$ROOT/pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs"
SCHEMA="$ROOT/pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"
SUPPORT="$ROOT/pkgs/fleet/crates/fleet-queue/src/client/support.rs"
CLIENT="$ROOT/pkgs/fleet/crates/fleet-queue/src/client.rs"

failures=0
fail() {
  echo "::error::$1"
  failures=$((failures + 1))
}

# (a) Procedure sections.
if [ ! -f "$PROC" ]; then
  fail "controlled-pool-tests: procedure doc missing: $PROC"
else
  for section in "Authorization gate" "Batch design" "Route pinning" "Measurements" "Verdict" "Records"; do
    if ! grep -qF "## $section" "$PROC"; then
      fail "controlled-pool-tests: procedure doc lacks section: $section"
    fi
  done
  if ! grep -qF "Launch each arm as a separate session" "$PROC"; then
    fail "controlled-pool-tests: procedure doc lacks the separate-session launch requirement (Route pinning: --pool does not launch a route)"
  fi
fi

# (b) Template fields.
if [ ! -f "$TMPL" ]; then
  fail "controlled-pool-tests: results template missing: $TMPL"
else
  for field in "Flag issue" "Candidate route" "Incumbent route" "Tasks per arm" "Completion" "Wall-clock" "Tokens" "Stalls/nudges" "Rework within 48h" "Verdict" "Headroom evidence"; do
    if ! grep -qF "$field" "$TMPL"; then
      fail "controlled-pool-tests: template lacks field: $field"
    fi
  done
fi

# (c) Load-bearing code references.
if [ ! -f "$CLAIM" ] || ! grep -qF "stamp_claim_model" "$CLAIM"; then
  fail "controlled-pool-tests: stamp_claim_model missing from $CLAIM (route pinning cites it)"
fi
if [ ! -f "$SCHEMA" ] || ! grep -qF "claim_model TEXT" "$SCHEMA"; then
  fail "controlled-pool-tests: claim_model TEXT column missing from $SCHEMA (route pinning cites it)"
fi
if [ ! -f "$SUPPORT" ] || ! grep -qF "derive_completion_telemetry" "$SUPPORT"; then
  fail "controlled-pool-tests: derive_completion_telemetry missing from $SUPPORT (arm identity cites it)"
fi
if { [ ! -f "$SUPPORT" ] || ! grep -qF "lastServing" "$SUPPORT"; } && { [ ! -f "$CLIENT" ] || ! grep -qF "lastServing" "$CLIENT"; }; then
  fail "controlled-pool-tests: lastServing readout missing from $SUPPORT and $CLIENT (arm identity cites it)"
fi

if [ "$failures" -ne 0 ]; then
  echo "controlled-pool-tests: FAIL — $failures violation(s)"
  exit 1
fi
echo "controlled-pool-tests: OK — procedure, template and code agree"
