#!/usr/bin/env bash
# Record a cf-worker versions release in pm-stack/platform/workers.json
# and push the record commit to main (#14941 pipeline; retry per #15041).
#
# The record job of cf-worker-versions-release.yml used to write the
# record, commit, and push exactly once. Parallel releases of different
# Workers (an expected use of the parameterized workflow) raced on that
# push: the loser was rejected with a stale-ref error ("cannot lock ref
# 'refs/heads/main': is at X but expected Y") and its record commit was
# lost, leaving workers.json without that Worker's deploy record until
# a manual re-dispatch. The per-Worker concurrency group serializes
# releases of the same Worker, never the record pushes across Workers.
#
# This script keeps the deploy itself untouched and makes only the
# record-commit path race-safe: on a rejected push it fetches current
# main, resets this job's disposable checkout onto it, re-applies the
# (idempotent) record write, re-commits, and retries — bounded attempts
# with jittered backoff. If every attempt fails it exits 1 with the
# last push error on stderr, so the job fails visibly instead of
# silently losing the record.
#
# Command-line contract (#3352): this script takes NO arguments; any
# argument is a usage error (exit 2) raised before any git, network, or
# file operation. All configuration arrives by environment:
#   WORKER             (required) workers.json entry name to update
#   DEPLOYED_COMMIT    (required) full commit SHA the release deployed
#   WORKERS_JSON       (default pm-stack/platform/workers.json)
#   REMOTE             (default origin)
#   TARGET_BRANCH      (default main)
#   MAX_ATTEMPTS       (default 8) bounded push attempts, integer >= 1
#   RETRY_BASE_SECONDS (default 2) backoff base; per-attempt delay is
#                      attempt * base plus up to 5s of jitter
#
# The caller must run this in a disposable checkout dedicated to the
# record commit: a retry performs `git reset --hard` onto the fetched
# target branch.
set -euo pipefail

usage() {
  echo "usage: record-worker-deploy.sh (no arguments; configure via WORKER and DEPLOYED_COMMIT env)" >&2
}

if [ "$#" -ne 0 ]; then
  echo "record-worker-deploy: unexpected argument: $1" >&2
  usage
  exit 2
fi

WORKER="${WORKER:-}"
DEPLOYED_COMMIT="${DEPLOYED_COMMIT:-}"
WORKERS_JSON="${WORKERS_JSON:-pm-stack/platform/workers.json}"
REMOTE="${REMOTE:-origin}"
TARGET_BRANCH="${TARGET_BRANCH:-main}"
MAX_ATTEMPTS="${MAX_ATTEMPTS:-8}"
RETRY_BASE_SECONDS="${RETRY_BASE_SECONDS:-2}"

if [ -z "$WORKER" ]; then
  echo "record-worker-deploy: WORKER is required" >&2
  usage
  exit 2
fi
if ! [[ "$DEPLOYED_COMMIT" =~ ^[0-9a-f]{7,64}$ ]]; then
  echo "record-worker-deploy: DEPLOYED_COMMIT must be a hex commit SHA (got '${DEPLOYED_COMMIT}')" >&2
  usage
  exit 2
fi
if ! [[ "$MAX_ATTEMPTS" =~ ^[0-9]+$ ]] || [ "$MAX_ATTEMPTS" -lt 1 ]; then
  echo "record-worker-deploy: MAX_ATTEMPTS must be an integer >= 1 (got '${MAX_ATTEMPTS}')" >&2
  usage
  exit 2
fi
if ! [[ "$RETRY_BASE_SECONDS" =~ ^[0-9]+$ ]]; then
  echo "record-worker-deploy: RETRY_BASE_SECONDS must be an integer >= 0 (got '${RETRY_BASE_SECONDS}')" >&2
  usage
  exit 2
fi

# The record write itself: identical transformation to the one the
# workflow ran inline before #15041. Idempotent — re-applying it onto a
# newer workers.json sets the same three fields for this Worker.
write_record() {
  WORKER="$WORKER" DEPLOYED_COMMIT="$DEPLOYED_COMMIT" WORKERS_JSON="$WORKERS_JSON" \
    python3 - <<'PY'
import json, os
path = os.environ["WORKERS_JSON"]
with open(path) as fh:
    data = json.load(fh)
for entry in data["workers"]:
    if entry["name"] == os.environ["WORKER"]:
        entry["deploy"] = "actions"
        entry["workflow"] = "cf-worker-versions-release.yml"
        entry["lastDeployedCommit"] = os.environ["DEPLOYED_COMMIT"]
        break
else:
    raise SystemExit(f"worker not in workers.json: {os.environ['WORKER']}")
with open(path, "w") as fh:
    json.dump(data, fh, indent=2)
    fh.write("\n")
PY
}

git config user.name >/dev/null 2>&1 ||
  git config user.name "github-actions[bot]"
git config user.email >/dev/null 2>&1 ||
  git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

last_push_error=""
attempt=1
while [ "$attempt" -le "$MAX_ATTEMPTS" ]; do
  if [ "$attempt" -gt 1 ]; then
    # A competing record push landed first. Rebase by construction:
    # reset this disposable checkout onto current main and re-apply
    # the record write there, so the retry commit is a fast-forward.
    git fetch "$REMOTE" "+refs/heads/${TARGET_BRANCH}:refs/record-worker-deploy/base"
    git reset --hard refs/record-worker-deploy/base >/dev/null
  fi
  write_record
  if git diff --quiet -- "$WORKERS_JSON"; then
    echo "workers.json already records this deploy." >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
    echo "record-worker-deploy: $WORKERS_JSON already records $WORKER at $DEPLOYED_COMMIT; nothing to push."
    exit 0
  fi
  git add "$WORKERS_JSON"
  git commit -q -m "cf-worker release: record $WORKER at ${DEPLOYED_COMMIT:0:12} (#14941)"
  push_log="$(mktemp)"
  if git push "$REMOTE" "HEAD:${TARGET_BRANCH}" 2>"$push_log"; then
    cat "$push_log" >&2 || true
    rm -f "$push_log"
    echo "Recorded $WORKER at ${DEPLOYED_COMMIT:0:12} in workers.json (attempt $attempt)." >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
    echo "record-worker-deploy: recorded $WORKER at $DEPLOYED_COMMIT on attempt $attempt."
    exit 0
  fi
  last_push_error="$(cat "$push_log")"
  rm -f "$push_log"
  if [ "$attempt" -lt "$MAX_ATTEMPTS" ]; then
    delay=$((attempt * RETRY_BASE_SECONDS + RANDOM % 6))
    echo "record-worker-deploy: push attempt $attempt for $WORKER rejected; retrying in ${delay}s after fetching current $TARGET_BRANCH." >&2
    if [ "$delay" -gt 0 ]; then
      sleep "$delay"
    fi
  fi
  attempt=$((attempt + 1))
done

echo "record-worker-deploy: FAILED to record $WORKER at $DEPLOYED_COMMIT after $MAX_ATTEMPTS attempts; the deploy itself succeeded but workers.json was not updated." >&2
echo "record-worker-deploy: last push error:" >&2
printf '%s\n' "$last_push_error" >&2
echo "Record push for $WORKER at ${DEPLOYED_COMMIT:0:12} failed after $MAX_ATTEMPTS attempts; workers.json was not updated. Last push error: $last_push_error" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
exit 1
