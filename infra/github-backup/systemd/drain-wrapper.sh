#!/usr/bin/env bash
# Sources the Worker URL + bearer from Bitwarden BY NAME at runtime
# (names: github-app.backup-worker-url / github-app.backup-worker-bearer;
# see ../config.json). Values pass through the environment only and are
# never written to disk.
set -euo pipefail
source ~/tools/bws-env.sh
eval "$(python3 - <<'PY'
import json, subprocess
lst = json.loads(subprocess.run(["bws", "secret", "list"],
                                capture_output=True, text=True).stdout or "[]")
ids = {s["key"]: s["id"] for s in lst}
def get(name):
    r = subprocess.run(["bws", "secret", "get", ids[name]],
                       capture_output=True, text=True)
    return json.loads(r.stdout)["value"]
print(f"export GHB_WORKER_URL={get('github-app.backup-worker-url')!r}")
print(f"export GHB_WORKER_BEARER={get('github-app.backup-worker-bearer')!r}")
PY
)"
exec python3 "$(dirname "$0")/../bin/drain_queue.py"
