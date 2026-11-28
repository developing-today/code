#!/usr/bin/env bash
# Post-change queue measurement harness for #2605 (hosted-runner queue
# saturation). Snapshots queued/in_progress workflow runs and prints a
# stable, greppable summary: totals, queued by event, oldest queued run,
# fresh in_progress pickups (created within 2h = runners are live), and
# stale queued runs older than 24h (phantom residue class from the
# 2026-11-11 saturation: zero-job runs the API refuses to cancel).
#
# Contract: ci-queue-snapshot.sh [--queued-json FILE] [--in-progress-json FILE]
#   [--now ISO-8601] [--repo OWNER/REPO]
# No arguments: live fetch via `gh api` for the repo's default remote.
# --queued-json/--in-progress-json: read API-shaped JSON instead of the
#   network (tests, fixtures, archived snapshots). --opt=value works
#   identically to --opt value. Unknown flags, stray positionals, and
#   present-but-valueless options are usage errors (exit 2) naming the
#   offender. No other arguments exist.
# Output: ci-queue-snapshot: <key>=<value> lines on stdout.
set -euo pipefail

QUEUED_JSON=""
INPROGRESS_JSON=""
NOW=""
REPO=""

usage() {
  echo "Usage: ci-queue-snapshot.sh [--queued-json FILE] [--in-progress-json FILE] [--now ISO-8601] [--repo OWNER/REPO]" >&2
}

# Strict parse before any subprocess, network, or file write.
while [[ $# -gt 0 ]]; do
  case "$1" in
  --queued-json=*)
    [[ -n ${1#*=} ]] || {
      echo "ci-queue-snapshot: --queued-json requires a value" >&2
      usage
      exit 2
    }
    QUEUED_JSON="${1#*=}"
    shift
    ;;
  --queued-json)
    [[ $# -ge 2 && -n ${2:-} && $2 != --* ]] || {
      echo "ci-queue-snapshot: --queued-json requires a value" >&2
      usage
      exit 2
    }
    QUEUED_JSON="$2"
    shift 2
    ;;
  --in-progress-json=*)
    [[ -n ${1#*=} ]] || {
      echo "ci-queue-snapshot: --in-progress-json requires a value" >&2
      usage
      exit 2
    }
    INPROGRESS_JSON="${1#*=}"
    shift
    ;;
  --in-progress-json)
    [[ $# -ge 2 && -n ${2:-} && $2 != --* ]] || {
      echo "ci-queue-snapshot: --in-progress-json requires a value" >&2
      usage
      exit 2
    }
    INPROGRESS_JSON="$2"
    shift 2
    ;;
  --now=*)
    [[ -n ${1#*=} ]] || {
      echo "ci-queue-snapshot: --now requires a value" >&2
      usage
      exit 2
    }
    NOW="${1#*=}"
    shift
    ;;
  --now)
    [[ $# -ge 2 && -n ${2:-} && $2 != --* ]] || {
      echo "ci-queue-snapshot: --now requires a value" >&2
      usage
      exit 2
    }
    NOW="$2"
    shift 2
    ;;
  --repo=*)
    [[ -n ${1#*=} ]] || {
      echo "ci-queue-snapshot: --repo requires a value" >&2
      usage
      exit 2
    }
    REPO="${1#*=}"
    shift
    ;;
  --repo)
    [[ $# -ge 2 && -n ${2:-} && $2 != --* ]] || {
      echo "ci-queue-snapshot: --repo requires a value" >&2
      usage
      exit 2
    }
    REPO="$2"
    shift 2
    ;;
  --help | -h)
    usage
    exit 0
    ;;
  --*)
    echo "ci-queue-snapshot: unknown flag $1" >&2
    usage
    exit 2
    ;;
  *)
    echo "ci-queue-snapshot: unexpected positional argument $1" >&2
    usage
    exit 2
    ;;
  esac
done

# Empty --opt= is present-but-valueless: usage error, never the default.
# (Checked inline at each --opt=* arm above via the empty test.)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [[ -z $NOW ]]; then
  NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
fi
now_epoch="$(date -u -d "$NOW" +%s)"

if [[ -n $QUEUED_JSON ]]; then
  [[ -f $QUEUED_JSON ]] || {
    echo "ci-queue-snapshot: queued file not found: $QUEUED_JSON" >&2
    exit 1
  }
  cp "$QUEUED_JSON" "$TMP/queued.json"
else
  [[ -n $REPO ]] || REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
  gh api "repos/$REPO/actions/runs?status=queued&per_page=100" --paginate \
    --jq '{total_count: ([.workflow_runs] | length), workflow_runs: ([.workflow_runs] | add // [])}' >"$TMP/queued.json"
fi

if [[ -n $INPROGRESS_JSON ]]; then
  [[ -f $INPROGRESS_JSON ]] || {
    echo "ci-queue-snapshot: in-progress file not found: $INPROGRESS_JSON" >&2
    exit 1
  }
  cp "$INPROGRESS_JSON" "$TMP/in_progress.json"
else
  [[ -n $REPO ]] || REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
  gh api "repos/$REPO/actions/runs?status=in_progress&per_page=100" --paginate \
    --jq '{total_count: ([.workflow_runs] | length), workflow_runs: ([.workflow_runs] | add // [])}' >"$TMP/in_progress.json"
fi

# Totals come from the run arrays (paginate-safe), not the page's total_count.
queued_total="$(jq -r '.workflow_runs | length' "$TMP/queued.json")"
inprogress_total="$(jq -r '.workflow_runs | length' "$TMP/in_progress.json")"

# Queued by event (stable sort for golden output).
queued_by_event="$(jq -r '.workflow_runs | group_by(.event) | map("\(.[0].event)=\(length)") | sort | join(" ")' "$TMP/queued.json")"
[[ -n $queued_by_event ]] || queued_by_event="(none)"

oldest_line="$(jq -r '.workflow_runs | sort_by(.created_at) | .[0] | "\(.id) \(.created_at)"' "$TMP/queued.json")"
if [[ $oldest_line == "null null" || -z $oldest_line ]]; then
  oldest_line="none"
fi

fresh_in_progress=0
while IFS= read -r created; do
  [[ -n $created ]] || continue
  created_epoch="$(date -u -d "$created" +%s)"
  age=$((now_epoch - created_epoch))
  if [[ $age -lt 7200 ]]; then
    fresh_in_progress=$((fresh_in_progress + 1))
  fi
done < <(jq -r '.workflow_runs[].created_at' "$TMP/in_progress.json")

stale_queued=0
while IFS= read -r created; do
  [[ -n $created ]] || continue
  created_epoch="$(date -u -d "$created" +%s)"
  age=$((now_epoch - created_epoch))
  if [[ $age -gt 86400 ]]; then
    stale_queued=$((stale_queued + 1))
  fi
done < <(jq -r '.workflow_runs[].created_at' "$TMP/queued.json")

echo "ci-queue-snapshot: queued_total=$queued_total in_progress_total=$inprogress_total"
echo "ci-queue-snapshot: queued_by_event $queued_by_event"
if [[ $oldest_line == "none" ]]; then
  echo "ci-queue-snapshot: oldest_queued none"
else
  # shellcheck disable=SC2086
  set -- $oldest_line
  echo "ci-queue-snapshot: oldest_queued id=$1 created=$2"
fi
echo "ci-queue-snapshot: fresh_in_progress=$fresh_in_progress"
echo "ci-queue-snapshot: stale_queued_older_than_24h=$stale_queued"
