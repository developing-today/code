#!/usr/bin/env bash
# fleet-binary-keeper.sh — keep a seat's installed fleet binary current
# with the newest fleet-static-* release on developing-today/code.
#
#   scripts/muse/fleet-binary-keeper.sh [--target <path>]
#                                        [--repo <owner/name>]
#                                        [--log-file <path>]
#                                        [--check-only]
#
# Compares the sha256 of the installed binary against the .sha256
# asset published with the newest fleet-static-* release (the oracle:
# the published sum is what the installed binary must hash to). On a
# match it logs and exits 0. On drift it downloads the release asset,
# verifies its sha256 against the published sum, and only then
# replaces the target atomically — a corrupt or partial download never
# touches the installed binary. --check-only reports drift (exit 3)
# without restoring. Every run appends one timestamped line to the log
# file, including no-op matches, so a silent keeper is detectable.
# Run it on the alignment cadence (every 15 minutes) from cron or a
# systemd timer; see scripts/muse/README.md.
#
# Command line: the four options above, each at most once, in either
# --opt value or --opt=value form; no positionals. Checked before any
# subprocess (doc/agents/operator-script-argument-contract.md, #3352).
set -euo pipefail

usage() {
  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
}
die_usage() {
  echo "fleet-binary-keeper: $1" >&2
  exit 2
}

TARGET=""
REPO=""
LOG_FILE=""
CHECK_ONLY=0
while [ $# -gt 0 ]; do
  arg=$1
  shift
  case "$arg" in
  -h | --help)
    [ $# -eq 0 ] || die_usage "--help takes no further arguments"
    usage
    exit 0
    ;;
  --target | --repo | --log-file)
    [ $# -ge 1 ] || die_usage "$arg requires a non-empty value"
    val=$1
    shift
    [ -n "$val" ] || die_usage "$arg requires a non-empty value"
    case "$arg" in
    --target)
      [ -z "$TARGET" ] || die_usage "duplicate --target"
      TARGET="$val"
      ;;
    --repo)
      [ -z "$REPO" ] || die_usage "duplicate --repo"
      REPO="$val"
      ;;
    --log-file)
      [ -z "$LOG_FILE" ] || die_usage "duplicate --log-file"
      LOG_FILE="$val"
      ;;
    esac
    ;;
  --target=* | --repo=* | --log-file=*)
    opt="${arg%%=*}"
    val="${arg#*=}"
    [ -n "$val" ] || die_usage "$opt requires a non-empty value"
    case "$opt" in
    --target)
      [ -z "$TARGET" ] || die_usage "duplicate --target"
      TARGET="$val"
      ;;
    --repo)
      [ -z "$REPO" ] || die_usage "duplicate --repo"
      REPO="$val"
      ;;
    --log-file)
      [ -z "$LOG_FILE" ] || die_usage "duplicate --log-file"
      LOG_FILE="$val"
      ;;
    esac
    ;;
  --check-only)
    [ "$CHECK_ONLY" -eq 0 ] || die_usage "duplicate --check-only"
    CHECK_ONLY=1
    ;;
  -*)
    die_usage "unknown flag $arg"
    ;;
  *)
    die_usage "unexpected positional argument: $arg"
    ;;
  esac
done

TARGET="${TARGET:-$HOME/tools/fleet}"
REPO="${REPO:-developing-today/code}"
LOG_FILE="${LOG_FILE:-$HOME/.local/state/fleet/fleet-binary-keeper.log}"
ASSET="fleet-x86_64-linux-musl"

mkdir -p "$(dirname "$LOG_FILE")"
log() {
  local line
  line="$(date -u +%Y-%m-%dT%H:%M:%SZ) $1"
  echo "$line"
  echo "$line" >>"$LOG_FILE"
}

# The newest fleet-static-* release is the reference build of current
# main: the fleet-static-release workflow publishes one from main on a
# daily cadence (#14999), and publish-fleet.sh remains the manual path.
tag="$(gh api "repos/$REPO/releases?per_page=100" --jq \
  '[.[] | select(.tag_name | startswith("fleet-static-"))] | sort_by(.published_at) | last | .tag_name // empty')"
if [ -z "$tag" ]; then
  log "error: no fleet-static-* release found on $REPO; installed binary left untouched"
  exit 1
fi

expected="$(gh release download "$tag" -R "$REPO" -p "$ASSET.sha256" -O - | awk 'NR == 1 {print $1}')"
if ! printf '%s' "$expected" | grep -qE '^[0-9a-f]{64}$'; then
  log "error: $tag published a malformed sha256 ($expected); installed binary left untouched"
  exit 1
fi

installed="none"
if [ -f "$TARGET" ]; then
  installed="$(sha256sum "$TARGET" | awk '{print $1}')"
fi

if [ "$installed" = "$expected" ]; then
  log "match tag=$tag sha256=${expected:0:12} target=$TARGET"
  exit 0
fi

if [ "$CHECK_ONLY" -eq 1 ]; then
  log "drift tag=$tag installed=${installed:0:12} expected=${expected:0:12} target=$TARGET (check-only; not restored)"
  exit 3
fi

target_dir="$(dirname "$TARGET")"
mkdir -p "$target_dir"
tmp="$(mktemp "$target_dir/.fleet-binary-keeper.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
gh release download "$tag" -R "$REPO" -p "$ASSET" -O "$tmp"
actual="$(sha256sum "$tmp" | awk '{print $1}')"
if [ "$actual" != "$expected" ]; then
  log "error: downloaded $tag asset failed sha256 verification (got ${actual:0:12}, expected ${expected:0:12}); installed binary left untouched"
  exit 1
fi
chmod 755 "$tmp"
mv "$tmp" "$TARGET"
trap - EXIT
log "restored tag=$tag from=${installed:0:12} to=${expected:0:12} target=$TARGET"
