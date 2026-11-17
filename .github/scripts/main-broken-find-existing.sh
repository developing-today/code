#!/usr/bin/env bash
# main-broken-find-existing (#9546): find the main-broken issue that
# already tracks a failing main run, searching OPEN AND CLOSED issues.
#
# Why this exists: the watcher used to search OPEN issues by
# Breakage-ID only, through the GitHub search index. The index lags a
# just-filed issue, and a run whose issue had already closed was
# invisible, so one failing run (43392267205) was filed three times
# (#9523, #9528, #9531) and each filing spawned its own fix session.
# This helper lists main-broken issues and matches their bodies
# locally, which does not depend on search-index freshness.
#
# Match priority (first match wins):
#   0. OPEN issue whose body carries this Breakage-ID (recurrence of
#      an open breakage: append, the chain's long-standing rule)
#   1. OPEN issue whose body carries this run's URL (same run,
#      already filed: append)
#   2. CLOSED issue whose body carries this run's URL (same run,
#      already filed and closed: comment there, file nothing)
# A CLOSED issue is never matched by Breakage-ID alone: a new run with
# the same failing jobs is a genuine recurrence and must file fresh.
#
# Env in:  GH_REPO, BID (Breakage-ID), RUN_ID
# Stdout:  "<number>\t<state>" (state lower-case) or nothing.
set -euo pipefail
: "${GH_REPO:?GH_REPO required}" "${BID:?BID required}" "${RUN_ID:?RUN_ID required}"

issues=$(gh issue list --repo "$GH_REPO" --state all --label main-broken \
  --limit 200 --json number,state,body)
printf '%s' "$issues" | jq -r --arg bid "$BID" --arg run "/actions/runs/$RUN_ID" '
  def rank:
    if .state == "OPEN" and ((.body // "") | contains($bid)) then 0
    elif .state == "OPEN" and ((.body // "") | contains($run)) then 1
    elif .state == "CLOSED" and ((.body // "") | contains($run)) then 2
    else empty end;
  [ .[] | {number, state, r: rank} ]
  | sort_by(.r) | .[0] // empty
  | "\(.number)\t\(.state | ascii_downcase)"'
