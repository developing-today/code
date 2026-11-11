#!/bin/bash
# Back up on-disk secrets into Bitwarden Secrets Manager ("developing today").
#
# bws has NO stdin/file input for values: `bws secret create KEY - PROJECT`
# stores the literal string "-". Values therefore go through $(cat file) —
# the literal command text (what the exec harness keeps in environ) contains
# only the file path, never the value. The value appears only transiently in
# the bws child process's argv while it runs.
#
# Gotchas (hit 2026-10-07):
# - `bws secret get <id> -o json` returns an OBJECT for one id but the shape
#   has varied; always normalize with: jq 'if type=="array" then .[0].value else .value end'
# - `jq -r` appends a trailing newline to the value; use `jq -j` when hashing
#   for comparison, or every verification fails.
# - Values starting with `-` (e.g. "-----BEGIN OPENSSH PRIVATE KEY-----")
#   are parsed as flags: put options FIRST, then `--`, then positionals:
#   `bws secret create --note ... -o none -- KEY VALUE PROJECT`.
#   (Without `--`, clap errors and ECHOES THE VALUE in the error message.)
#
# Verification compares sha256 of the round-tripped value against the file
# (trailing newline stripped, since $(...) strips it) — values are never printed.
set -u
source ~/tools/bws-env.sh >/dev/null 2>&1

PROJECT=d370cd10-3d67-4b34-b61a-b4dc00753f97
DATE=$(date -u +%F)

declare -A SECRETS=(
  [nix-cache.upload-secret]="$HOME/.config/nix-cache/upload-secret"
  [nix-cache.signing-key]="$HOME/.config/nix-cache/signing.sec"
  [cachix.cachix-dhall]="$HOME/.config/cachix/cachix.dhall"
  [github-cli.hosts-yml]="$HOME/.config/gh/hosts.yml"
  [ssh.id-ed25519]="$HOME/.ssh/id_ed25519"
)
declare -A NOTES=(
  [nix-cache.upload-secret]="Source: ~/.config/nix-cache/upload-secret. Backed up $DATE. Same value is set as Cloudflare Worker secret UPLOAD_SECRET (nix-cache worker)."
  [nix-cache.signing-key]="Source: ~/.config/nix-cache/signing.sec (nix store signing private key). Backed up $DATE."
  [cachix.cachix-dhall]="Source: ~/.config/cachix/cachix.dhall (contains Cachix auth token). Backed up $DATE."
  [github-cli.hosts-yml]="Source: ~/.config/gh/hosts.yml (gh CLI OAuth tokens). Backed up $DATE."
  [ssh.id-ed25519]="Source: ~/.ssh/id_ed25519 (SSH private key). Backed up $DATE."
)

strip_nl() { perl -0777 -pe 's/\n\z//'; }
bw_value() { # $1 = secret id -> prints value on stdout, no trailing newline
  bws secret get "$1" -o json 2>/dev/null \
    | jq -j 'if type=="array" then .[0].value else .value end'
}

fail=0
for key in "${!SECRETS[@]}"; do
  file="${SECRETS[$key]}"
  [ -f "$file" ] || { echo "SKIP $key: $file missing"; fail=1; continue; }
  id=$(bws secret list 2>/dev/null | jq -r --arg k "$key" '.[] | select(.key==$k) | .id' | head -1)
  want=$(strip_nl < "$file" | sha256sum | cut -d' ' -f1)
  if [ -n "$id" ]; then
    got=$(bw_value "$id" | sha256sum | cut -d' ' -f1)
    if [ "$got" = "$want" ]; then
      echo "OK $key (already correct)"
      continue
    else
      echo "REWRITE $key (existing value wrong, id $id)"
      bws secret delete "$id" -o none 2>/dev/null
    fi
  fi
  # shellcheck disable=SC2046
  id=$(bws secret create --note "${NOTES[$key]}" -o json -- \
    "$key" "$(cat "$file")" "$PROJECT" 2>/dev/null | jq -r '.id')
  if [ -z "$id" ] || [ "$id" = "null" ]; then
    echo "FAIL $key: create error"; fail=1; continue
  fi
  got=$(bw_value "$id" | sha256sum | cut -d' ' -f1)
  if [ "$got" = "$want" ]; then
    echo "OK $key (created, verified)"
  else
    echo "FAIL $key (round-trip mismatch)"; fail=1
  fi
done
echo "---"; echo "fail=$fail"
