#!/usr/bin/env bash
# main-broken-budget (#1782): reconcile the main-breakage chain's
# concurrent-remediation budget (3) over ALL open main-broken
# issues, as one serialized decision.
#
# Why a reconcile, not a per-filing check: the watcher used to
# count the `dispatch/remediate` labels and then apply the label
# inline, inside a workflow whose concurrency group was keyed on
# the run id -- so two firings for the same breakage were never
# serialized, and two create-path firings could both count 2 and
# both apply, exceeding the cap (acceptance gap recorded on #1782,
# 2026-11-05). The watcher now runs this script from a job whose
# concurrency group is the budget scope itself
# (`main-broken-budget`, cancel-in-progress: false), and the daily
# release-check job runs it again under the same group, so a
# budget decision lost with a cancelled pending job is re-made by
# the next invocation instead of being lost.
#
# Rules (the chain's long-standing semantics, computed over
# global state):
#   - A candidate is an OPEN issue labelled `main-broken` +
#     `queue/queued` (queue/queued asserts a fix task was queued,
#     so scout-routed issues filed without one never enter the
#     budget). A candidate is DECIDED once it carries the
#     `dispatch/remediate` label or an agent-note recording
#     `holder: dispatch`, `remediation budget exhausted`, or
#     `released by chain`; decided candidates are never revisited.
#   - While fewer than BUDGET issues carry the label, grant it to
#     the oldest undecided candidates first (lowest issue number),
#     posting the `holder: dispatch` agent-note.
#   - Once the budget is full, every remaining undecided candidate
#     gets the `holder: chain (remediation budget exhausted)`
#     agent-note -- the same terminal decision the inline step
#     made at filing time; the issue already carries
#     queue/queued, so the fleet monitor enqueues the priority
#     packet from the issue itself.
#   - If more than BUDGET issues carry the label (a pre-serialized
#     race, or a manual label), the label is removed from the
#     highest-numbered holders beyond the first BUDGET, with the
#     budget-exhausted note, so the cap holds after every run.
#
# Env in:  GH_REPO (required), DRY_RUN ("true" prints decisions
#          and mutates nothing)
# Deps:    gh, jq.
set -euo pipefail
: "${GH_REPO:?GH_REPO required}"
DRY_RUN="${DRY_RUN:-false}"
BUDGET=3

ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# One list call serves both views; label filtering happens
# locally (the find-existing helper's pattern, #9546) so the
# script is testable against a canned issue list.
issues=$(gh issue list --repo "$GH_REPO" --state open \
  --label main-broken --limit 200 --json number,labels)

labeled=$(jq -r '[.[] | select([.labels[].name] | index("dispatch/remediate")) | .number] | sort | .[]' <<<"$issues")
labeled_count=$(jq -r '[.[] | select([.labels[].name] | index("dispatch/remediate"))] | length' <<<"$issues")

grant_note() { # $1 = count before this grant
  printf 'agent-note:v1 — holder: dispatch, since %s (dispatch/remediate label applied; the issue-labeled job in pm-dispatch.yml starts remediation via pm-dispatch). Remediation budget at filing: %s/3 concurrent remediations in flight.' \
    "$ts" "$1"
}

exhausted_note() { # $1 = count of labelled issues at decision time
  printf 'agent-note:v1 — holder: chain (remediation budget exhausted) at %s: %s open main-broken issues already carry the dispatch/remediate label. This issue carries queue/queued, so the fleet monitor enqueues the priority packet from the issue.' \
    "$ts" "$1"
}

# Cap enforcement: more holders than the budget allows. The
# first BUDGET (lowest-numbered) holders keep the label.
if [ "$labeled_count" -gt "$BUDGET" ]; then
  excess=$(tail -n +"$((BUDGET + 1))" <<<"$labeled")
  for n in $excess; do
    if [ "$DRY_RUN" = "true" ]; then
      echo "DRY-RUN: would remove dispatch/remediate from #$n and note budget exhausted (cap $BUDGET, $labeled_count labelled)"
      continue
    fi
    gh issue edit "$n" --repo "$GH_REPO" --remove-label dispatch/remediate
    gh issue comment "$n" --repo "$GH_REPO" --body "$(exhausted_note "$labeled_count")"
    echo "#$n: dispatch/remediate removed by cap enforcement ($labeled_count labelled, cap $BUDGET)."
    labeled_count=$((labeled_count - 1))
  done
fi

# Candidates: open, queue/queued (a fix task exists), not already
# holding the label, oldest first.
candidates=$(jq -r '[.[] | select([.labels[].name] | index("queue/queued"))
  | select(([.labels[].name] | index("dispatch/remediate")) | not)
  | .number] | sort | .[]' <<<"$issues")

for n in $candidates; do
  # Decided? A previous grant/exhaust/release leaves an
  # agent-note naming the holder transition.
  bodies=$(gh api "repos/$GH_REPO/issues/$n/comments?per_page=100" 2>/dev/null |
    jq -r '.[].body // empty' || true)
  if grep -q 'holder: dispatch' <<<"$bodies" ||
    grep -q 'remediation budget exhausted' <<<"$bodies" ||
    grep -q 'released by chain' <<<"$bodies"; then
    echo "#$n: already decided (holder note on record); skipping."
    continue
  fi
  if [ "$labeled_count" -lt "$BUDGET" ]; then
    if [ "$DRY_RUN" = "true" ]; then
      echo "DRY-RUN: would apply dispatch/remediate to #$n and note holder: dispatch (budget $labeled_count/$BUDGET)"
      labeled_count=$((labeled_count + 1))
      continue
    fi
    gh issue edit "$n" --repo "$GH_REPO" --add-label dispatch/remediate
    gh issue comment "$n" --repo "$GH_REPO" --body "$(grant_note "$labeled_count")"
    echo "#$n: dispatch/remediate applied (budget $labeled_count/$BUDGET before grant)."
    labeled_count=$((labeled_count + 1))
  else
    if [ "$DRY_RUN" = "true" ]; then
      echo "DRY-RUN: would note holder: chain (remediation budget exhausted) on #$n (budget $labeled_count/$BUDGET)"
      continue
    fi
    gh issue comment "$n" --repo "$GH_REPO" --body "$(exhausted_note "$labeled_count")"
    echo "#$n: remediation budget exhausted; holder recorded as chain (budget $labeled_count/$BUDGET)."
  fi
done

echo "main-broken budget reconciled at $ts: $labeled_count/$BUDGET remediations labelled."
