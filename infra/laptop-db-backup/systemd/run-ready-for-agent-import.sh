#!/usr/bin/env bash
# Load the ready-for-agent backup home's credentials at runtime, then
# run the repo-owned local-DB -> Turso home import (mesh class E,
# #13459/#13474). Credentials are fetched from Bitwarden by secret name and
# exposed only to this process tree; nothing is persisted in systemd
# unit files. The home's token
# (turso.laptop-backup-ready-for-agent-rw-token) is minted when the home
# database is provisioned; until that vault entry exists this
# wrapper fails with a clear message and the timer stays visibly
# unprovisioned rather than half-working.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
LDB_DIR="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z ${BWS_ACCESS_TOKEN-} && -f ~/.config/bitwarden/access-token ]]; then
  BWS_ACCESS_TOKEN="$(cat ~/.config/bitwarden/access-token)"
  export BWS_ACCESS_TOKEN
fi
if ! command -v bws >/dev/null 2>&1 && [[ -x "$HOME/tools/bin/bws" ]]; then
  export PATH="$HOME/tools/bin:$PATH"
fi
command -v bws >/dev/null 2>&1 || {
  echo "laptop-db-backup: bws not found" >&2
  exit 1
}
secrets="$(bws secret list 2>/dev/null)" || {
  echo "laptop-db-backup: bws secret list failed" >&2
  exit 1
}
secret() {
  printf '%s' "$secrets" | python3 -c 'import json,sys
key=sys.argv[1]
for item in json.load(sys.stdin):
    if item.get("key") == key:
        print(item.get("value", "")); break' "$1"
}
TURSO_HOME_TOKEN="$(secret turso.laptop-backup-ready-for-agent-rw-token)"
[[ -n $TURSO_HOME_TOKEN ]] || {
  echo "laptop-db-backup: credentials incomplete in Bitwarden (need turso.laptop-backup-ready-for-agent-rw-token)" >&2
  exit 1
}
export TURSO_HOME_TOKEN
exec /usr/bin/env python3 "$LDB_DIR/bin/import_to_turso.py" \
  --source "$HOME/.local/share/ready-for-agent/ready-for-agent.db" \
  --home-url "https://laptop-backup-ready-for-agent-dezren39.aws-us-east-1.turso.io" \
  --home-name "laptop-backup-ready-for-agent"
