#!/bin/bash
set -u
OUT=$(fleet queue claim --queue repo-health-code-pilot --worker repo-worker-code --lane code-g --lease-seconds 2700)
echo "$OUT" | python3 -c "
import json,sys
d=json.load(sys.stdin)
if not d or d.get('id') is None: print('NO_TASK'); sys.exit(0)
p=d.get('payload') or {}
print('ID',d['id']); print('TITLE',d.get('title')); print('KIND',p.get('kind')); print('ISSUE',(p.get('issue') or {}).get('number')); print('ACCEPT', json.dumps(p.get('acceptance'))[:800]); print('SCOPE', json.dumps(p.get('scope'))[:800])
" > /tmp/claim-summary.txt
cat /tmp/claim-summary.txt
grep -q NO_TASK /tmp/claim-summary.txt && exit 0
TID=$(grep '^ID' /tmp/claim-summary.txt | awk '{print $2}')
ISS=$(grep '^ISSUE' /tmp/claim-summary.txt | awk '{print $2}')
~/tools/lane-handoff.sh "$TID" "$ISS" x
