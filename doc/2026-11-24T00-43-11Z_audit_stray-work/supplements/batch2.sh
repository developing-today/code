#!/bin/bash
export PATH="$HOME/.local/share/hatch-recovery/bin:$PATH"
while ! grep -q FINISH1-DONE ~/workspace/lane7-audit7/finish1.log 2>/dev/null; do sleep 15; done
# parent|children|labels
run_batch() {
  parent=$1
  labels=$2
  shift 2
  for n in "$@"; do
    claims=$(fleet gh api "repos/developing-today/code/issues/$n/comments?per_page=100" --jq '[.[] | select(.body | test("AUDIT-7|lane claim|claimed by";"i"))] | length' 2>/dev/null | tail -1)
    if [ "$claims" != "0" ]; then
      echo "$n SKIPPED claims=$claims"
      continue
    fi
    id=$(fleet gh api repos/developing-today/code/issues/$n --jq .id 2>/dev/null | tail -1)
    r=$(fleet gh api -X POST repos/developing-today/code/issues/$parent/sub_issues -F sub_issue_id=$id --jq '.number' 2>&1 | tail -1)
    labargs=""
    for l in $labels; do labargs="$labargs -f labels[]=$l"; done
    lr=$(fleet gh api -X POST repos/developing-today/code/issues/$n/labels $labargs --jq '[.[].name]|join(",")' 2>&1 | tail -1)
    echo "$n -> parent $parent link=[$r] labels=[$lr]"
  done
}
run_batch 8979 "stray-work primary-parent/scheduled-tasks epic/scheduled-tasks" 12260 12133 11355 11166 11165 10937 10628 10538 10387 9404
run_batch 8986 "stray-work primary-parent/design-register epic/design-register" 10389 10390 10392 10393
echo BATCH2-DONE
