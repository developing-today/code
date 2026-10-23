#!/usr/bin/env bash
# bws-to-sops -- mirror EVERY Bitwarden Secrets Manager secret into one
# sops/age-encrypted YAML file, keyed by the bws secret key (e.g.
# `google.jules-api-key`, `cloudflare.api-token-ai-inference`).
#
#   lib/bws-to-sops.sh              sync all secrets -> secrets/sops/common/bws.yaml
#   lib/bws-to-sops.sh --dry-run    list what would be written (key names only)
#   lib/bws-to-sops.sh --project ID only secrets in one bws project
#   lib/bws-to-sops.sh --out FILE   write somewhere else
#
# Idempotent: rewrites the file only when the decrypted contents change, so it
# is safe to re-run after adding/rotating secrets in bws. The file is a full
# mirror -- secrets deleted from bws disappear from it on the next sync.
#
# Auth (first hit wins): $BWS_ACCESS_TOKEN (set in every shell by the
# agent-shell-env hook), /run/secrets/agent-auth/bws-access-token,
# ~/.config/bws/access-token, then secrets/sops/common/bitwarden.yaml.
#
# Use a mirrored value from NixOS with sops-nix, e.g.:
#   sops.secrets.foo = { sopsFile = lib.from-root "secrets/sops/common/bws.yaml"; key = "google.jules-api-key"; };
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || dirname "$SCRIPT_DIR")"

OUT="${REPO_ROOT}/secrets/sops/common/bws.yaml"
PROJECT=""
DRY_RUN=false

die() {
  echo "bws-to-sops: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
  --out)
    OUT="${2:?--out needs a path}"
    shift 2
    ;;
  --project)
    PROJECT="${2:?--project needs an id}"
    shift 2
    ;;
  --dry-run | -n)
    DRY_RUN=true
    shift
    ;;
  -h | --help)
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *) die "unknown argument: $1" ;;
  esac
done

for bin in bws sops jq; do
  command -v "$bin" >/dev/null 2>&1 || die "$bin not found in PATH"
done

if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
  for f in /run/secrets/agent-auth/bws-access-token "${XDG_CONFIG_HOME:-$HOME/.config}/bws/access-token"; do
    if [ -r "$f" ]; then
      IFS= read -r BWS_ACCESS_TOKEN <"$f" || :
      [ -n "${BWS_ACCESS_TOKEN:-}" ] && break
    fi
  done
fi
if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
  BWS_ACCESS_TOKEN="$(sops -d --extract '["bws_access_token"]' "${REPO_ROOT}/secrets/sops/common/bitwarden.yaml" 2>/dev/null)" ||
    die "no BWS_ACCESS_TOKEN and could not decrypt secrets/sops/common/bitwarden.yaml"
fi
export BWS_ACCESS_TOKEN

ARGS=(secret list --output json)
[ -n "$PROJECT" ] && ARGS=(secret list "$PROJECT" --output json)
RAW="$(bws "${ARGS[@]}")" || die "bws secret list failed"

DUPES="$(jq -r 'group_by(.key)[] | select(length > 1) | .[0].key' <<<"$RAW")"
[ -z "$DUPES" ] || die "duplicate bws keys, refusing to guess which wins: $DUPES"

DOC="$(jq -S 'map({(.key): .value}) | add // {}' <<<"$RAW")"
COUNT="$(jq 'length' <<<"$DOC")"
[ "$COUNT" -gt 0 ] || die "bws returned no secrets (wrong project / token scope?)"

if [ "$DRY_RUN" = true ]; then
  echo "Would write $COUNT secret(s) to ${OUT#"$REPO_ROOT"/}:"
  jq -r 'keys[] | "  " + .' <<<"$DOC"
  exit 0
fi

if [ -f "$OUT" ]; then
  CUR="$(sops -d --output-type json "$OUT" | jq -S .)" || die "cannot decrypt existing $OUT"
  if [ "$CUR" = "$DOC" ]; then
    echo "Up to date: ${OUT#"$REPO_ROOT"/} ($COUNT secrets)"
    exit 0
  fi
  echo "Changes vs ${OUT#"$REPO_ROOT"/} (key names only):"
  jq -rn --argjson a "$CUR" --argjson b "$DOC" '
    ($b | keys) as $bk | ($a | keys) as $ak |
    (($bk - $ak)[] | "  + \(.)"),
    (($ak - $bk)[] | "  - \(.)"),
    (($ak - ($ak - $bk))[] | select($a[.] != $b[.]) | "  ~ \(.)")'
fi

mkdir -p "$(dirname "$OUT")"
TMP="$(mktemp "${OUT}.XXXXXX")"
trap 'rm -f "$TMP"' EXIT
sops -e --input-type json --output-type yaml --filename-override "$OUT" <(printf '%s\n' "$DOC") >"$TMP"
# Round-trip check before replacing the old file.
[ "$(sops -d --input-type yaml --output-type json "$TMP" | jq -S .)" = "$DOC" ] || die "round-trip check failed; $OUT left untouched"
mv "$TMP" "$OUT"
trap - EXIT
echo "Wrote $COUNT secret(s) to ${OUT#"$REPO_ROOT"/}"
