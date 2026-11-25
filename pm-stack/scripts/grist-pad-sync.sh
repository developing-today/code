#!/usr/bin/env bash
# grist-pad-sync — the gated Grist/Pad file-store sync driver.
#
# Implements the Grist/Pad half of pm-stack/STORE-MIGRATION-PLAN-6816.md
# §3-§5 for issue #6816. File sets (from the §2 inventory):
#   grist: home.sqlite3 + docs/ (grist-sessions.db is runtime state and is
#          never synced); pad: pad.db + encryption.key + config.toml +
#          attachments/.
#
# Two modes:
#   presync  while live: copy the store's file set from source to target,
#            then require post-copy sha256 parity of the copied set.
#            Pre-copy drift is reported, not failed. May need a rerun if
#            the live writer changes a file mid-copy.
#   final    the gate before promotion. Requires the store's writer FENCED
#            (proven by fence-check.sh --store <store>; per-store so pad
#            can go final while grist still runs, per the Pad → Grist →
#            Kaneo stop order) and, for pad, a clean WAL (pad.db-wal
#            absent or zero bytes in the source). Then copies and requires
#            exact parity. Refusal copies nothing.
#
# The driver never deletes target files: target-only files under the synced
# file set (the named files and the named dir trees — not unrelated files
# elsewhere in the target, such as sync-tool metadata or grist-sessions.db)
# are preserved and counted in the receipt. presync tolerates them
# (reported); final refuses on them (exit 4) — an exact mirror has no
# unaccounted files, and only the operator removes them.
#
# Argument contract (#3352): flags only, no positionals. `--opt value`
# and `--opt=value` behave identically. Anything else is a usage error
# (exit 2) before any copy runs:
#   --store grist|pad       required
#   --mode presync|final     required
#   --source-dir DIR         required; must exist and be readable
#   --target-dir DIR         required; created when missing
#   --receipt FILE           write the receipt here (default: stdout only)
#   --fence-check PATH       fence checker (default: sibling fence-check.sh)
#   --units-dir DIR          passed to fence-check (fixture mode)
#   --containers-file FILE   passed to fence-check (fixture mode)
#   --unreachable            passed to fence-check: old host unreachable
#   -h, --help               usage, exit 0
#
# Exit codes: 0 pass, 2 usage, 3 final refused (unfenced or dirty WAL),
# 4 parity failure, 5 command failure (missing required file, copy/hash
# failure).
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

STORE=""
MODE=""
SOURCE_DIR=""
TARGET_DIR=""
RECEIPT=""
FENCE_CHECK="$SCRIPT_DIR/fence-check.sh"
FENCE_ARGS=()

usage() {
  cat <<'USAGE'
usage: grist-pad-sync.sh --store grist|pad --mode presync|final
                         --source-dir DIR --target-dir DIR
                         [--receipt FILE] [--fence-check PATH]
                         [--units-dir DIR] [--containers-file FILE]
                         [--unreachable] [-h|--help]

  presync  copy the store's file set while live, require post-copy parity.
  final    require the store fenced (and a clean pad WAL), copy, require
           exact parity with no unaccounted target files. Refusal (exit 3)
           copies nothing. Never deletes target files.
USAGE
}

fail_usage() {
  echo "grist-pad-sync: $1" >&2
  usage >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
  -h | --help)
    usage
    exit 0
    ;;
  --unreachable)
    FENCE_ARGS+=("--unreachable")
    shift
    ;;
  --store | --mode | --source-dir | --target-dir | --receipt | --fence-check | --units-dir | --containers-file)
    opt="$1"
    [ "$#" -ge 2 ] || fail_usage "$opt requires a value"
    val="$2"
    [ -n "$val" ] || fail_usage "$opt requires a non-empty value"
    case "$opt" in
    --store) STORE="$val" ;;
    --mode) MODE="$val" ;;
    --source-dir) SOURCE_DIR="$val" ;;
    --target-dir) TARGET_DIR="$val" ;;
    --receipt) RECEIPT="$val" ;;
    --fence-check) FENCE_CHECK="$val" ;;
    --units-dir) FENCE_ARGS+=("--units-dir" "$val") ;;
    --containers-file) FENCE_ARGS+=("--containers-file" "$val") ;;
    esac
    shift 2
    ;;
  --store=* | --mode=* | --source-dir=* | --target-dir=* | --receipt=* | --fence-check=* | --units-dir=* | --containers-file=*)
    opt="${1%%=*}"
    val="${1#*=}"
    [ -n "$val" ] || fail_usage "$opt requires a non-empty value"
    case "$opt" in
    --store) STORE="$val" ;;
    --mode) MODE="$val" ;;
    --source-dir) SOURCE_DIR="$val" ;;
    --target-dir) TARGET_DIR="$val" ;;
    --receipt) RECEIPT="$val" ;;
    --fence-check) FENCE_CHECK="$val" ;;
    --units-dir) FENCE_ARGS+=("--units-dir" "$val") ;;
    --containers-file) FENCE_ARGS+=("--containers-file" "$val") ;;
    esac
    shift
    ;;
  --*)
    fail_usage "unexpected argument '$1'"
    ;;
  *)
    fail_usage "unexpected argument '$1' (no positionals)"
    ;;
  esac
done

case "$STORE" in
grist | pad) ;;
"") fail_usage "--store is required (grist or pad)" ;;
*) fail_usage "--store must be grist or pad (got '$STORE')" ;;
esac

case "$MODE" in
presync | final) ;;
"") fail_usage "--mode is required (presync or final)" ;;
*) fail_usage "--mode must be presync or final (got '$MODE')" ;;
esac

[ -n "$SOURCE_DIR" ] || fail_usage "--source-dir is required"
[ -n "$TARGET_DIR" ] || fail_usage "--target-dir is required"
[ -d "$SOURCE_DIR" ] || fail_usage "source dir '$SOURCE_DIR' is not a directory"
[ -r "$SOURCE_DIR" ] || fail_usage "source dir '$SOURCE_DIR' is not readable"
[ "$SOURCE_DIR" != "$TARGET_DIR" ] || fail_usage "source and target dirs must differ"

[ -x "$FENCE_CHECK" ] || {
  echo "grist-pad-sync: fence checker '$FENCE_CHECK' is not executable — refusing to gate a sync on an absent check" >&2
  exit 5
}

if command -v sha256sum >/dev/null 2>&1; then
  HASHER="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  HASHER="shasum -a 256"
else
  echo "grist-pad-sync: no sha256 tool (sha256sum or shasum) — cannot verify parity" >&2
  exit 5
fi

case "$STORE" in
grist)
  SYNC_FILES="home.sqlite3"
  SYNC_DIRS="docs"
  ;;
pad)
  SYNC_FILES="pad.db encryption.key config.toml"
  SYNC_DIRS="attachments"
  ;;
esac

NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RECEIPT_BODY=""
write_receipt() {
  # $1 = verdict line, $2 = extra detail lines (optional)
  RECEIPT_BODY="# Grist/Pad file-store sync receipt — store $STORE — mode $MODE — $NOW

- Fence: ${FENCE_LINE:-not checked (presync)}
- Files synced: ${SYNCED_COUNT:-0}
- Pre-copy drift: ${DRIFT_LINE:-unknown}
- Target extras: ${EXTRAS_LINE:-none}
- Verdict: $1
${2:-}"
  if [ -n "$RECEIPT" ]; then
    printf '%s\n' "$RECEIPT_BODY" >"$RECEIPT" || {
      echo "grist-pad-sync: cannot write receipt '$RECEIPT'" >&2
      exit 5
    }
  fi
}

# manifest <dir>: "hash  relpath" per synced file, sorted. Only the synced
# file set is listed, so unrelated target files (e.g. grist-sessions.db)
# never count as extras.
manifest() {
  dir="$1"
  tmp="$2"
  : >"$tmp"
  for f in $SYNC_FILES; do
    if [ -f "$dir/$f" ]; then
      # shellcheck disable=SC2086
      $HASHER "$dir/$f" | sed "s|$dir/||" >>"$tmp" || return 1
    fi
  done
  for d in $SYNC_DIRS; do
    if [ -d "$dir/$d" ]; then
      while IFS= read -r f; do
        # shellcheck disable=SC2086
        $HASHER "$f" | sed "s|$dir/||" >>"$tmp" || return 1
      done < <(find "$dir/$d" -type f | sort)
    fi
  done
  sort -o "$tmp" "$tmp"
}

relpaths() { awk '{print $2}' "$1"; }

# --- final: fence first, fail closed, before anything is copied ---
if [ "$MODE" = "final" ]; then
  if ! FENCE_OUT="$("$FENCE_CHECK" --store "$STORE" ${FENCE_ARGS[@]+"${FENCE_ARGS[@]}"} 2>&1)" || [ "${FENCE_OUT%%:*}" != "FENCED" ]; then
    FENCE_LINE="UNFENCED — ${FENCE_OUT:-fence-check produced no output}"
    SYNCED_COUNT=0
    DRIFT_LINE="not run (refused)"
    EXTRAS_LINE="not run (refused)"
    write_receipt "REFUSED — final requires the $STORE writer fenced"
    echo "grist-pad-sync: REFUSED — $STORE writer not fenced; no sync, no copy. $FENCE_OUT" >&2
    exit 3
  fi
  FENCE_LINE="$FENCE_OUT"
  if [ "$STORE" = "pad" ]; then
    if [ -e "$SOURCE_DIR/pad.db-wal" ] && [ -s "$SOURCE_DIR/pad.db-wal" ]; then
      SYNCED_COUNT=0
      DRIFT_LINE="not run (refused)"
      EXTRAS_LINE="not run (refused)"
      write_receipt "REFUSED — pad WAL is not checkpointed (pad.db-wal is non-empty)"
      echo "grist-pad-sync: REFUSED — pad.db-wal is non-empty; checkpoint it before the final copy. No sync, no copy." >&2
      exit 3
    fi
  fi
fi

mkdir -p "$TARGET_DIR" || {
  echo "grist-pad-sync: cannot create target dir '$TARGET_DIR'" >&2
  exit 5
}

MANIFEST_TMP="$(mktemp "${TMPDIR:-/tmp/opencode}/grist-pad-sync.XXXXXX")"
trap 'rm -f "$MANIFEST_TMP" "$MANIFEST_TMP.src" "$MANIFEST_TMP.tgt"' EXIT
SRC_LIST="$MANIFEST_TMP.src"
TGT_LIST="$MANIFEST_TMP.tgt"

# Pre-copy drift: which synced files already match, for the receipt.
if manifest "$SOURCE_DIR" "$SRC_LIST" && manifest "$TARGET_DIR" "$TGT_LIST"; then
  drift=0
  missing=0
  while IFS= read -r line; do
    hash="${line%% *}"
    rel="${line#* }"
    rel="${rel#* }"
    if grep -Fq "$rel" "$TGT_LIST" 2>/dev/null; then
      if ! grep -Fq "$line" "$TGT_LIST" 2>/dev/null; then
        drift=$((drift + 1))
      fi
    else
      missing=$((missing + 1))
    fi
    _="$hash"
  done <"$SRC_LIST"
  DRIFT_LINE="$drift differed, $missing missing from target (landed by this sync)"
else
  DRIFT_LINE="unreadable — failing closed"
  write_receipt "FAIL — cannot hash the synced file set"
  echo "grist-pad-sync: FAIL — cannot hash the synced file set" >&2
  exit 5
fi

# Copy the file set. Required files must exist; dirs land when present.
SYNCED_COUNT=0
for f in $SYNC_FILES; do
  [ -f "$SOURCE_DIR/$f" ] || {
    write_receipt "FAIL — required source file '$f' is missing"
    echo "grist-pad-sync: required source file '$STORE/$f' is missing — failing closed" >&2
    exit 5
  }
  [ -r "$SOURCE_DIR/$f" ] || {
    write_receipt "FAIL — required source file '$f' is unreadable"
    echo "grist-pad-sync: required source file '$STORE/$f' is unreadable — failing closed" >&2
    exit 5
  }
  cp -- "$SOURCE_DIR/$f" "$TARGET_DIR/$f" || {
    echo "grist-pad-sync: copy of '$f' failed" >&2
    exit 5
  }
  SYNCED_COUNT=$((SYNCED_COUNT + 1))
done
DIR_NOTE=""
for d in $SYNC_DIRS; do
  if [ -d "$SOURCE_DIR/$d" ]; then
    mkdir -p "$TARGET_DIR/$d" || {
      echo "grist-pad-sync: cannot create target dir '$TARGET_DIR/$d'" >&2
      exit 5
    }
    cp -a -- "$SOURCE_DIR/$d/." "$TARGET_DIR/$d/" || {
      echo "grist-pad-sync: copy of '$d/' failed" >&2
      exit 5
    }
    n="$(find "$SOURCE_DIR/$d" -type f | wc -l)"
    SYNCED_COUNT=$((SYNCED_COUNT + n))
  else
    DIR_NOTE="${DIR_NOTE}dir $d/ absent in source, not synced. "
  fi
done

# Post-copy parity over the synced set, plus target extras under it.
manifest "$SOURCE_DIR" "$SRC_LIST" || {
  write_receipt "FAIL — cannot re-hash the source after copy"
  echo "grist-pad-sync: FAIL — cannot re-hash the source after copy" >&2
  exit 5
}
manifest "$TARGET_DIR" "$TGT_LIST" || {
  write_receipt "FAIL — cannot hash the target after copy"
  echo "grist-pad-sync: FAIL — cannot hash the target after copy" >&2
  exit 5
}

extras="$(comm -13 <(relpaths "$SRC_LIST" | sort) <(relpaths "$TGT_LIST" | sort) || true)"
if [ -n "$extras" ]; then
  n_extras="$(printf '%s\n' "$extras" | wc -l)"
  EXTRAS_LINE="$n_extras kept (never deleted): $(printf '%s' "$extras" | tr '\n' ' ')"
else
  n_extras=0
  EXTRAS_LINE="none"
fi

missing_or_different="$(comm -23 "$SRC_LIST" "$TGT_LIST" || true)"
if [ -n "$missing_or_different" ]; then
  write_receipt "FAIL — parity: $(printf '%s\n' "$missing_or_different" | wc -l) synced file(s) differ or are missing after copy" "- Detail: ${DIR_NOTE}extras: $EXTRAS_LINE"
  echo "grist-pad-sync: FAIL — parity: synced files differ or are missing after copy (re-run the sync)" >&2
  exit 4
fi
if [ "$MODE" = "final" ] && [ "$n_extras" -gt 0 ]; then
  write_receipt "FAIL — parity: $n_extras unaccounted target file(s); only the operator removes them" "- Detail: $EXTRAS_LINE"
  echo "grist-pad-sync: FAIL — parity: $n_extras unaccounted target file(s): $(printf '%s' "$extras" | tr '\n' ' ') (remove them and re-run)" >&2
  exit 4
fi

write_receipt "PASS — $SYNCED_COUNT file(s) synced with exact sha256 parity" "- Detail: ${DIR_NOTE}extras: $EXTRAS_LINE"
if [ "$MODE" = "final" ]; then
  echo "grist-pad-sync: FINAL OK — $FENCE_LINE; $SYNCED_COUNT file(s), exact parity"
else
  echo "grist-pad-sync: PRESYNC OK — $SYNCED_COUNT file(s) copied with exact parity"
fi
if [ -z "$RECEIPT" ]; then
  printf '\n%s\n' "$RECEIPT_BODY"
fi
exit 0
