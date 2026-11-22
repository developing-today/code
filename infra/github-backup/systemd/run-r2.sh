#!/usr/bin/env bash
# Load the established runtime-only R2 credentials, then run the repo-owned
# all-mirror uploader. Never persist credentials in systemd unit files.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
GB_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# Match atlas-backup.sh: fetch secrets at runtime by key name and expose them
# only to this process tree via rclone's environment-based remote config.
if [[ -z ${BWS_ACCESS_TOKEN:-} && -r "$HOME/.config/bws/access-token" ]]; then
  BWS_ACCESS_TOKEN="$(cat "$HOME/.config/bws/access-token")"
  export BWS_ACCESS_TOKEN
fi
if ! command -v bws >/dev/null 2>&1 && [[ -x "$HOME/tools/bin/bws" ]]; then
  export PATH="$HOME/tools/bin:$PATH"
fi
command -v bws >/dev/null 2>&1 || {
  echo "github-backup-r2: bws not found" >&2
  exit 1
}
if ! command -v rclone >/dev/null 2>&1; then
  rclone_bin="$(ls -d /nix/store/*-rclone-[0-9]*/bin/rclone 2>/dev/null | sort -V | tail -1)"
  [[ -n $rclone_bin && -x $rclone_bin ]] || {
    echo "github-backup-r2: rclone not found in PATH or nix store" >&2
    exit 1
  }
  export PATH="$(dirname "$rclone_bin"):$PATH"
fi
secrets="$(bws secret list 2>/dev/null)" || {
  echo "github-backup-r2: bws secret list failed" >&2
  exit 1
}
secret() {
  printf '%s' "$secrets" | python3 -c 'import json,sys
key=sys.argv[1]
for item in json.load(sys.stdin):
    if item.get("key") == key:
        print(item.get("value", "")); break' "$1"
}
access_key="$(secret cloudflare.r2-backups-access-key-id)"
secret_key="$(secret cloudflare.r2-backups-secret-access-key)"
endpoint="$(secret cloudflare.r2-endpoint)"
[[ -n $access_key && -n $secret_key && -n $endpoint ]] || {
  echo "github-backup-r2: R2 credentials incomplete in Bitwarden" >&2
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
exec /usr/bin/env python3 "$GB_DIR/bin/bundle_r2_all.py"
