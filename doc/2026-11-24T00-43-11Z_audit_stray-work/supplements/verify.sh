#!/bin/bash
export PATH="$HOME/.local/share/hatch-recovery/bin:$PATH"
for p in 8976 12462 12460 13504 8979; do
  echo "== parent $p sub_issues:"
  fleet gh api "repos/developing-today/code/issues/$p/sub_issues?per_page=100" --jq '.[].number' 2>&1 | tail -n +1 | tr '\n' ' '; echo
done
for n in 13497 13496 13495 12664 12467 12249 12250; do
  echo "== child $n labels:"
  fleet gh api "repos/developing-today/code/issues/$n/labels" --jq '[.[].name]|join(",")' 2>&1 | tail -1
done
echo VERIFY-DONE
