#!/usr/bin/env bash
# Print the number of the workflow-owned open bws freshness drift issue,
# or nothing when none is verified open. Used by both the drift (comment)
# and fresh (close) paths of .github/workflows/bws-freshness.yml (#6644).
#
# Identity is deterministic, never search rank: a candidate must carry
# the exact drift title AND the hidden body marker the workflow writes
# at creation (the issue also gets the `bws-drift` label at creation).
# An unrelated issue whose title merely contains the same words — which
# is all `in:title` search guarantees — is ignored, so the fresh path
# can never close a human's issue and the drift path never comments the
# drift report onto one. If several candidates ever verify, the oldest
# (the episode's original issue) wins.
set -euo pipefail

title="bws.yaml mirror drift detected"
marker="<!-- bws-freshness-drift-issue v1 -->"

gh issue list --search "$title in:title state:open" --limit 50 \
  --json number,title,body,createdAt |
  jq -r --arg title "$title" --arg marker "$marker" \
    '[.[] | select(.title == $title and (.body // "" | contains($marker)))]
     | sort_by(.createdAt) | .[0].number // empty'
