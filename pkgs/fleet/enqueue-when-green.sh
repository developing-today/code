#!/bin/bash
# Poll a PR's required basic check; enqueue it in the merge queue the
# moment the check completes SUCCESS (enqueuePullRequest requires a
# COMPLETED required check on the head). Usage: enqueue-when-green.sh <PR>
PR=$1
for i in $(seq 1 55); do
  S=$(gh pr view $PR --repo developing-today/code --json statusCheckRollup --jq '[.statusCheckRollup[] | select(.name=="basic: proxy port fallbacks")][0] | "\(.status)/\(.conclusion)"' 2>/dev/null)
  echo "poll $i: pr=$PR basic=$S"
  if [ "$S" = "COMPLETED/SUCCESS" ]; then
    ID=$(gh pr view $PR --repo developing-today/code --json id --jq .id)
    gh api graphql -f query='mutation($id: ID!) { enqueuePullRequest(input: {pullRequestId: $id}) { mergeQueueEntry { position } } }' -f id="$ID" 2>&1 | head -c 300
    echo
    exit 0
  fi
  case "$S" in COMPLETED/FAILURE|COMPLETED/CANCELLED) echo "BASIC CHECK NOT GREEN: $S"; exit 1;; esac
  sleep 60
done
echo "TIMEOUT waiting for basic check on $PR"
