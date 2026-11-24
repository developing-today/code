#!/bin/bash
export PATH="$HOME/.local/share/hatch-recovery/bin:$PATH"
for n in 12249 12250; do
  id=$(fleet gh api repos/developing-today/code/issues/$n --jq .id 2>/dev/null | tail -1)
  echo "$n id=$id"
  fleet gh api -X POST repos/developing-today/code/issues/8979/sub_issues -F sub_issue_id=$id --jq '.number' 2>&1 | tail -1
done
for n in 12467 12249 12250; do
  echo "labels $n:"
  fleet gh api -X POST repos/developing-today/code/issues/$n/labels -f 'labels[]=stray-work' -f 'labels[]=primary-parent/scheduled-tasks' -f 'labels[]=epic/scheduled-tasks' --jq '[.[].name]|join(",")' 2>&1 | tail -1
done
echo FINISH1-DONE
