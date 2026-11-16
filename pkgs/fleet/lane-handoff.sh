#!/bin/bash
# usage: lane-handoff.sh TASK_ID ISSUE_NUMBER "kind description"
set -u
T=$1; I=$2; DESC=$3
python3 - "$T" <<'PY'
import json,pathlib,sys
p=pathlib.Path.home()/"hooks/state/repo-queue-code-worker-g.lock"
d=json.loads(p.read_text()); d["task_id"]=int(sys.argv[1]); p.write_text(json.dumps(d,indent=2))
PY
LANE=$(fleet queue lane "$T" --worker repo-worker-code 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('assigned_lane',''))")
echo "LANE=$LANE"
fleet queue checkpoint "$T" --worker repo-worker-code --stage claimed --progress "{\"next\": \"lane assignment: $LANE\", \"artifact\": \"issue #$I\"}" >/dev/null && echo CLAIMED_CKPT_OK
gh issue view "$I" --repo developing-today/code --json state,labels --jq '"state="+.state+" labels="+([.labels[].name]|join(","))'
