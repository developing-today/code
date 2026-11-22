#!/usr/bin/env bash
# Load the fleet-queue backup credentials at runtime, then run the
# repo-owned Turso -> D1 export. Credentials are fetched from
# Bitwarden by secret name and exposed only to this process tree;
# nothing is persisted in systemd unit files.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
FQ_DIR="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z "${BWS_ACCESS_TOKEN-}" && -f ~/.config/bitwarden/access-token ]]; then
  BWS_ACCESS_TOKEN="$(cat ~/.config/bitwarden/access-token)"
  export BWS_ACCESS_TOKEN
fi
if ! command -v bws >/dev/null 2>&1 && [[ -x "$HOME/tools/bin/bws" ]]; then
  export PATH="$HOME/tools/bin:$PATH"
fi
command -v bws >/dev/null 2>&1 || {
  echo "fleet-queue-backup: bws not found" >&2
  exit 1
}
secrets="$(bws secret list 2>/dev/null)" || {
  echo "fleet-queue-backup: bws secret list failed" >&2
  exit 1
}
secret() {
  printf '%s' "$secrets" | python3 -c 'import json,sys
key=sys.argv[1]
for item in json.load(sys.stdin):
    if item.get("key") == key:
        print(item.get("value", "")); break' "$1"
}
TURSO_QUEUE_TOKEN="$(secret turso.fleet-queue-rw-token)"
CLOUDFLARE_API_TOKEN="$(secret cloudflare.api-token-account-wide)"
[[ -n $TURSO_QUEUE_TOKEN && -n $CLOUDFLARE_API_TOKEN ]] || {
  echo "fleet-queue-backup: credentials incomplete in Bitwarden" >&2
  exit 1
}
export TURSO_QUEUE_TOKEN CLOUDFLARE_API_TOKEN
# The account id is configuration, not a credential: discover the one
# account the token can see rather than storing a second copy of it.
CLOUDFLARE_ACCOUNT_ID="$(python3 -c '
import json, os, urllib.request
req = urllib.request.Request("https://api.cloudflare.com/client/v4/accounts")
req.add_header("Authorization", "Bearer " + os.environ["CLOUDFLARE_API_TOKEN"])
with urllib.request.urlopen(req, timeout=60) as resp:
    accounts = json.load(resp)["result"]
assert len(accounts) == 1, f"expected exactly one account, found {len(accounts)}"
print(accounts[0]["id"])')"
export CLOUDFLARE_ACCOUNT_ID
exec /usr/bin/env python3 "$FQ_DIR/bin/export_to_d1.py"
