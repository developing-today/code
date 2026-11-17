#!/bin/bash
# lane-scratch: any-d laptop worktree task 16153 issue 9609 remediation loop
# usage: unblock-9609.sh <start-line> <count>
cd /home/user/git/fleet/16153-9609-quarantine-triage || exit 1
START=${1:-1}
COUNT=${2:-40}
REASON="#9609 remediation: max_attempts validator ceiling raised 100 to 1000 by PR #9621 (broker rev a2ebee7a65); packet conforms again - releasing claim-path quarantine, attempts preserved"
sed -n "${START},$((START+COUNT-1))p" unblock-ids-9609.txt | while read -r id; do
  [ -z "$id" ] && continue
  out=$(fleet queue unblock "$id" --worker repo-worker-code --reason "$REASON" 2>&1)
  if printf '%s' "$out" | grep -qF '"status": "queued"'; then
    echo "$id OK" >> unblock-9609-results.log
  else
    echo "$id FAIL: $(printf '%s' "$out" | head -c 200)" >> unblock-9609-results.log
  fi
done
echo "CHUNK ${START} done" >> unblock-9609-results.log
