#!/usr/bin/env bash
# Load the runtime-only credentials the freshness probe needs, then run
# the repo-owned five-copy probe. Credentials are fetched from
# Bitwarden by secret name and exposed only to this process tree;
# nothing is persisted in systemd unit files.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
GB_DIR="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z ${BWS_ACCESS_TOKEN-} && -f ~/.config/bitwarden/access-token ]]; then
  BWS_ACCESS_TOKEN="$(<~/.config/bitwarden/access-token)"
  export BWS_ACCESS_TOKEN
fi
if ! command -v bws >/dev/null 2>&1 && [[ -x "$HOME/tools/bin/bws" ]]; then
  export PATH="$HOME/tools/bin:$PATH"
fi
command -v bws >/dev/null 2>&1 || {
  echo "github-backup-freshness: bws not found" >&2
  exit 1
}
if ! command -v rclone >/dev/null 2>&1; then
  rclone_bin="$(ls -d /nix/store/*-rclone-[0-9]*/bin/rclone 2>/dev/null | sort -V | tail -1)"
  [[ -n $rclone_bin && -x $rclone_bin ]] || {
    echo "github-backup-freshness: rclone not found in PATH or nix store" >&2
    exit 1
  }
  export PATH="$(dirname "$rclone_bin"):$PATH"
fi
secrets="$(bws secret list 2>/dev/null)" || {
  echo "github-backup-freshness: bws secret list failed" >&2
  exit 1
}
secret() {
  printf '%s' "$secrets" | python3 -c 'import json,sys
key=sys.argv[1]
for item in json.load(sys.stdin):
    if item.get("key") == key:
        print(item.get("value", "")); break' "$1"
}
# Cloudflare token: the D1 meta check (same named secret the queue
# export wrapper uses).
CLOUDFLARE_API_TOKEN="$(secret cloudflare.api-token-account-wide)"
[[ -n $CLOUDFLARE_API_TOKEN ]] || {
  echo "github-backup-freshness: Cloudflare token incomplete in Bitwarden" >&2
  exit 1
}
export CLOUDFLARE_API_TOKEN
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
# R2 read access for the bundle check: the same named secrets and
# environment-based rclone remote the R2 uploader wrapper builds.
access_key="$(secret cloudflare.r2-backups-access-key-id)"
secret_key="$(secret cloudflare.r2-backups-secret-access-key)"
endpoint="$(secret cloudflare.r2-endpoint)"
[[ -n $access_key && -n $secret_key && -n $endpoint ]] || {
  echo "github-backup-freshness: R2 credentials incomplete in Bitwarden" >&2
  exit 1
}
case "$endpoint" in http://* | https://*) ;; *) endpoint="https://$endpoint" ;; esac
export RCLONE_CONFIG_R2BAK_TYPE=s3
export RCLONE_CONFIG_R2BAK_PROVIDER=Cloudflare
export RCLONE_CONFIG_R2BAK_ACCESS_KEY_ID="$access_key"
export RCLONE_CONFIG_R2BAK_SECRET_ACCESS_KEY="$secret_key"
export RCLONE_CONFIG_R2BAK_ENDPOINT="${endpoint%/}"
export RCLONE_CONFIG_R2BAK_REGION=auto
export RCLONE_CONFIG_R2BAK_NO_CHECK_BUCKET=true
exec /usr/bin/env python3 "$GB_DIR/bin/freshness_probe.py" --file-issues
