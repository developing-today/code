#!/usr/bin/env bash
# Sources the Worker URL + bearer from Bitwarden BY NAME at runtime
# (names: github-app.backup-worker-url / github-app.backup-worker-bearer;
# see ../config.json). Values pass through the environment only and are
# never written to disk.
set -euo pipefail
# Bitwarden access: the laptop's bws reads ~/.config/bws/access-token
# itself (verified: bare `bws secret list` works there). Export it when
# present; only if a bare bws call would fail, fall back to sourcing a
# bws-env.sh (sandbox convention) — non-fatally.
if [ -z "${BWS_ACCESS_TOKEN:-}" ] && [ -f ~/.config/bws/access-token ]; then
  export BWS_ACCESS_TOKEN="$(cat ~/.config/bws/access-token)"
fi
if ! bws secret list >/dev/null 2>&1; then
  for f in ~/tools/bws-env.sh ~/hatch/tools/bws-env.sh; do
    if [ -f "$f" ]; then source "$f" || true; fi
  done
fi
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
