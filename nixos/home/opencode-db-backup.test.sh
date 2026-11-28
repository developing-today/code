#!/usr/bin/env bash
# Test suite for opencode-db-backup.sh (code#10297; zstd leg #14515).
# Drives the REAL script (no pasted copy) with a scratch HOME holding a
# fake ~/.local/share/opencode/opencode.db, a stub bws printing fake
# credential JSON, a stub sqlite3 that performs .backup as a file copy
# and answers integrity_check with "ok", and a stub restic that records
# its invocations and maintains a fake local repo (config + snapshots).
# rclone is a FUNCTIONAL fake: a "drive:" remote backed by a local
# directory ($RCLONE_STUB_ROOT), so the zstd leg's copyto/cat/lsf/purge
# run for real against files the suite can inspect; the Drive restic
# leg runs restic against an rclone: URL, which the stub restic only
# records. Failure injection: RCLONE_STUB_FAIL_COPYTO=1 makes copyto
# fail; RCLONE_STUB_CORRUPT_CAT=1 makes cat return corrupted bytes.
# Scenarios:
#   * happy path — exit 0, one restic backup per backend (local + r2 +
#     drive) with --tag opencode-db-daily over the staged snapshots,
#     check + forget+prune (with keep-monthly 12) per backend, staging
#     removed, success marker + run record written; plus the zstd leg:
#     dated .db.zst + manifest in the local archive and the fake Drive
#     remote (sha256-identical), the artifact decompresses to the
#     staged image, zstd success marker written, run record carries
#     the zstd block.
#   * zstd retention — a fixed run date (2026-11-24) over a 22-date
#     fixture prunes exactly {2025-05-10, 2025-03-15, 2024-06-15} on
#     BOTH targets (GFS: keep-daily 7 / keep-weekly 4 / keep-monthly
#     12; the fixture makes the weekly-only saves 2026-11-09 and
#     2026-10-31, the monthly-only save 2026-09-30, and the
#     latest-in-month prune 2025-05-10 each decisive).
#   * Drive upload failure — exit non-zero; the restic marker still
#     advances (a zstd failure must not mask restic success), the
#     zstd marker does not, and the local artifact stays published.
#   * Drive copy corruption — cat returns wrong bytes: exit non-zero
#     naming the sha256 mismatch; zstd marker not written.
#   * corrupt compression — a stub zstd whose output does not
#     decompress to the staged image: exit non-zero before publish,
#     no artifact, no .tmp- file left behind, no zstd marker.
#   * missing DB — an HOME without opencode.db exits non-zero before
#     any restic invocation.
#   * lock skip — a second instance holding the flock prints SKIP and
#     exits 0, invoking restic zero times.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/opencode-db-backup.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

failures=0
note_fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

make_home() { # $1 dir; fake opencode share dir + bws token
  local h="$1"
  mkdir -p "$h/.local/share/opencode" "$h/.config/bws"
  printf 'fake-sqlite-db-bytes' >"$h/.local/share/opencode/opencode.db"
  printf 'fake-local-db-bytes' >"$h/.local/share/opencode/opencode-local.db"
  echo "fake-token" >"$h/.config/bws/access-token"
}

make_bin() { # $1 bin dir, $2 restic calls file
  local b="$1" calls="$2"
  mkdir -p "$b"
  cat >"$b/bws" <<'EOF'
#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "list" ]; then
  printf '[{"key":"restic.repo-archive-password","value":"pw"},{"key":"cloudflare.r2-backups-access-key-id","value":"id"},{"key":"cloudflare.r2-backups-secret-access-key","value":"secret"},{"key":"cloudflare.r2-endpoint","value":"https://example.r2.test"}]'
  exit 0
fi
exit 1
EOF
  cat >"$b/sqlite3" <<'EOF'
#!/bin/sh
db="$1"; sql="$2"
case "$sql" in
  "VACUUM INTO "*)
    dest="$(printf '%s' "$sql" | sed "s/^VACUUM INTO '//; s/';$//")"
    cp "$db" "$dest" ;;
  "PRAGMA integrity_check;") echo "ok" ;;
  *) echo "stub sqlite3: unexpected sql: $sql" >&2; exit 1 ;;
esac
exit 0
EOF
  cat >"$b/restic" <<EOF
#!/bin/sh
echo "\$*" >> "$calls"
repo="\$(printf '%s\n' "\$@" | grep -A1 '^-r\$' | tail -1)"
if [ "\$1" = "-r" ]; then shift 2; fi
case "\$repo" in
  *:* ) :;; # remote URL (s3:/rclone:) — record invocation only
  *)
    case "\$1" in
      cat|init) mkdir -p "\$repo"; touch "\$repo/config";;
      backup) mkdir -p "\$repo"; echo "\$*" >> "\$repo/snapshots"; echo "Added to the repository: 1.000 KiB";;
      check|forget) :;;
    esac;;
esac
exit 0
EOF
  cat >"$b/rclone" <<'EOF'
#!/bin/sh
# Functional fake rclone: the "drive:" remote is the local directory
# $RCLONE_STUB_ROOT, so copyto/cat/lsf/purge operate on real files.
root="${RCLONE_STUB_ROOT:?RCLONE_STUB_ROOT not set}"
cmd="$1"; shift
case "$cmd" in
  copyto)
    [ "${RCLONE_STUB_FAIL_COPYTO:-0}" = "1" ] && exit 1
    dest="${2#drive:}"
    mkdir -p "$root/$(dirname "$dest")"
    cp "$1" "$root/$dest" ;;
  cat)
    p="${1#drive:}"
    cat "$root/$p" || exit 1
    [ "${RCLONE_STUB_CORRUPT_CAT:-0}" = "1" ] && printf 'x' ;;
  lsf)
    for last; do :; done
    p="${last#drive:}"
    if [ -d "$root/$p" ]; then
      for d in "$root/$p"/*/; do
        [ -d "$d" ] && printf '%s/\n' "$(basename "$d")"
      done
    fi ;;
  purge)
    p="${1#drive:}"
    rm -rf "${root:?}/$p" ;;
  *) echo "stub rclone: unexpected command: $cmd $*" >&2; exit 1 ;;
esac
exit 0
EOF
  chmod +x "$b/bws" "$b/sqlite3" "$b/restic" "$b/rclone"
}

run_script() { # $1 home, $2 bin, $3 remote root, $4 run date, $5 fail copyto, $6 corrupt cat
  env HOME="$1" PATH="$2:$PATH" RCLONE_STUB_ROOT="$3" \
    OPENCODE_DB_BACKUP_DATE="${4:-}" \
    RCLONE_STUB_FAIL_COPYTO="${5:-0}" RCLONE_STUB_CORRUPT_CAT="${6:-0}" \
    bash "$SCRIPT"
}

ZSTD_BIN="$(command -v zstd 2>/dev/null || ls -d /nix/store/*-zstd-[0-9]*/bin/zstd 2>/dev/null | sort -V | tail -1)"

# --- happy path -------------------------------------------------------------
H1="$TMP_ROOT/h1" B1="$TMP_ROOT/b1" C1="$TMP_ROOT/calls1" R1="$TMP_ROOT/r1"
make_home "$H1"
make_bin "$B1" "$C1"
out="$(run_script "$H1" "$B1" "$R1" "2026-11-24" 2>&1)" || note_fail "happy path exited non-zero: $out"
echo "$out" | grep -q "OK opencode session DB backed up" || note_fail "happy path missing OK line: $out"
backups="$(grep -c ' backup ' "$C1" 2>/dev/null || true)"
[ "$backups" = "3" ] || note_fail "expected 3 restic backup invocations, got $backups"
grep -q -- '--tag opencode-db-daily' "$C1" || note_fail "backup invocations missing --tag opencode-db-daily"
grep -q -- '--keep-monthly 12' "$C1" || note_fail "forget invocations missing --keep-monthly 12"
grep -q 'opencode.db' "$C1" || note_fail "backup invocations missing staged opencode.db"
[ -f "$H1/backups/restic/.opencode-db-last-success-epoch" ] || note_fail "success marker not written"
[ -f "$H1/backups/restic/.opencode-db-last-run.json" ] || note_fail "run record not written"
grep -q '"result": "complete"' "$H1/backups/restic/.opencode-db-last-run.json" 2>/dev/null ||
  note_fail "run record does not say complete"
[ ! -e "$H1/backups/opencode-db-staging" ] || note_fail "staging dir not cleaned after success"

# --- zstd leg (happy path, fixed run date 2026-11-24) ---
ZDATE="2026-11-24"
ZDIR="$H1/backups/opencode-db-zstd/$ZDATE"
ZART="$ZDIR/opencode-db-$ZDATE.db.zst"
[ -f "$ZART" ] || note_fail "zstd artifact missing at $ZART"
[ -f "$ZDIR/manifest.json" ] || note_fail "zstd manifest missing at $ZDIR/manifest.json"
zst_sha="$(sha256sum "$ZART" | cut -d' ' -f1)"
python3 - "$ZDIR/manifest.json" "$zst_sha" <<'PYEOF' || note_fail "zstd manifest does not match the artifact"
import json, sys
m = json.load(open(sys.argv[1]))
assert m["source_db"] == "opencode.db", "manifest source_db"
assert m["files"][0]["path"] == "opencode-db-2026-11-24.db.zst", "manifest file path"
assert m["files"][0]["sha256"] == sys.argv[2], "manifest sha256 != artifact sha256"
PYEOF
dec_sha="$("$ZSTD_BIN" -dc "$ZART" | sha256sum | cut -d' ' -f1)"
src_sha="$(printf 'fake-sqlite-db-bytes' | sha256sum | cut -d' ' -f1)"
[ "$dec_sha" = "$src_sha" ] || note_fail "zstd artifact does not decompress to the staged image"
RDIR="$R1/Cloud/cold-store/db-backups/opencode/$ZDATE"
[ -f "$RDIR/opencode-db-$ZDATE.db.zst" ] || note_fail "zstd Drive artifact missing"
[ -f "$RDIR/manifest.json" ] || note_fail "zstd Drive manifest missing"
rsha="$(sha256sum "$RDIR/opencode-db-$ZDATE.db.zst" 2>/dev/null | cut -d' ' -f1)"
[ "$rsha" = "$zst_sha" ] || note_fail "zstd Drive artifact sha256 mismatch"
[ -f "$H1/backups/restic/.opencode-db-zstd-last-success-epoch" ] || note_fail "zstd success marker not written"
grep -q '"zstd"' "$H1/backups/restic/.opencode-db-last-run.json" 2>/dev/null ||
  note_fail "run record missing the zstd block"

# --- zstd retention (GFS over the dated dirs, both targets) ------------------
H4="$TMP_ROOT/h4" B4="$TMP_ROOT/b4" C4="$TMP_ROOT/calls4" R4="$TMP_ROOT/r4"
make_home "$H4"
make_bin "$B4" "$C4"
FIXTURE_DATES="2026-11-23 2026-11-22 2026-11-21 2026-11-20 2026-11-19 2026-11-18 2026-11-09 2026-10-31 2026-09-30 2025-12-20 2025-11-15 2025-10-15 2025-09-15 2025-08-15 2025-07-15 2025-06-15 2025-05-15 2025-05-10 2025-04-15 2025-03-15 2024-06-15"
for d in $FIXTURE_DATES; do
  mkdir -p "$H4/backups/opencode-db-zstd/$d"
  echo dummy >"$H4/backups/opencode-db-zstd/$d/opencode-db-$d.db.zst"
  mkdir -p "$R4/Cloud/cold-store/db-backups/opencode/$d"
  echo dummy >"$R4/Cloud/cold-store/db-backups/opencode/$d/opencode-db-$d.db.zst"
done
out="$(run_script "$H4" "$B4" "$R4" "2026-11-24" 2>&1)" || note_fail "retention run exited non-zero: $out"
for d in 2025-05-10 2025-03-15 2024-06-15; do
  [ ! -e "$H4/backups/opencode-db-zstd/$d" ] || note_fail "retention kept pruned local date $d"
  [ ! -e "$R4/Cloud/cold-store/db-backups/opencode/$d" ] || note_fail "retention kept pruned Drive date $d"
done
for d in 2026-11-24 2026-11-18 2026-11-09 2026-10-31 2026-09-30 2025-12-20 2025-05-15 2025-04-15; do
  [ -d "$H4/backups/opencode-db-zstd/$d" ] || note_fail "retention pruned kept local date $d"
  [ -d "$R4/Cloud/cold-store/db-backups/opencode/$d" ] || note_fail "retention pruned kept Drive date $d"
done

# --- zstd Drive upload failure ------------------------------------------------
H5="$TMP_ROOT/h5" B5="$TMP_ROOT/b5" C5="$TMP_ROOT/calls5" R5="$TMP_ROOT/r5"
make_home "$H5"
make_bin "$B5" "$C5"
if run_script "$H5" "$B5" "$R5" "2026-11-24" 1 >/dev/null 2>&1; then
  note_fail "Drive upload failure should exit non-zero"
fi
[ -f "$H5/backups/restic/.opencode-db-last-success-epoch" ] ||
  note_fail "restic marker missing after a zstd-only failure (restic success must stay visible)"
[ ! -f "$H5/backups/restic/.opencode-db-zstd-last-success-epoch" ] ||
  note_fail "zstd marker written despite the Drive upload failure"
[ -f "$H5/backups/opencode-db-zstd/2026-11-24/opencode-db-2026-11-24.db.zst" ] ||
  note_fail "local zstd artifact should stay published when the Drive leg fails"

# --- zstd Drive copy corruption ------------------------------------------------
H6="$TMP_ROOT/h6" B6="$TMP_ROOT/b6" C6="$TMP_ROOT/calls6" R6="$TMP_ROOT/r6"
make_home "$H6"
make_bin "$B6" "$C6"
out="$(run_script "$H6" "$B6" "$R6" "2026-11-24" 0 1 2>&1)" &&
  note_fail "corrupt Drive copy should exit non-zero"
echo "$out" | grep -q "zstd Drive copy sha256 mismatch" ||
  note_fail "corrupt Drive copy not named as a sha256 mismatch: $out"
[ ! -f "$H6/backups/restic/.opencode-db-zstd-last-success-epoch" ] ||
  note_fail "zstd marker written despite the Drive copy corruption"

# --- corrupt compression: verify-before-publish + temp cleanup ------------------
H7="$TMP_ROOT/h7" B7="$TMP_ROOT/b7" C7="$TMP_ROOT/calls7" R7="$TMP_ROOT/r7"
make_home "$H7"
make_bin "$B7" "$C7"
cat >"$B7/zstd" <<'EOF'
#!/bin/sh
# Corrupt zstd: compression "succeeds" but the bytes do not
# decompress to the source.
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-o" ]; then out="$a"; fi
  prev="$a"
done
case " $* " in
*" -dc "*) printf 'corrupted-decompressed-bytes'; exit 0 ;;
esac
[ -n "$out" ] && printf 'corrupted-compressed-bytes' >"$out"
exit 0
EOF
chmod +x "$B7/zstd"
if run_script "$H7" "$B7" "$R7" "2026-11-24" >/dev/null 2>&1; then
  note_fail "corrupt compression should exit non-zero"
fi
[ ! -e "$H7/backups/opencode-db-zstd/2026-11-24/opencode-db-2026-11-24.db.zst" ] ||
  note_fail "corrupt artifact was published"
tmp_left="$(find "$H7/backups/opencode-db-zstd" -name '.tmp-*' 2>/dev/null)"
[ -z "$tmp_left" ] || note_fail "temp file left behind after corrupt compression: $tmp_left"
[ ! -f "$H7/backups/restic/.opencode-db-zstd-last-success-epoch" ] ||
  note_fail "zstd marker written despite corrupt compression"

# --- missing DB ---------------------------------------------------------------
H2="$TMP_ROOT/h2" B2="$TMP_ROOT/b2" C2="$TMP_ROOT/calls2" R2="$TMP_ROOT/r2"
mkdir -p "$H2/.config/bws"
echo "fake-token" >"$H2/.config/bws/access-token"
make_bin "$B2" "$C2"
if run_script "$H2" "$B2" "$R2" >/dev/null 2>&1; then
  note_fail "missing DB should exit non-zero"
fi
[ ! -f "$C2" ] || note_fail "missing DB invoked restic"

# --- lock skip ------------------------------------------------------------------
H3="$TMP_ROOT/h3" B3="$TMP_ROOT/b3" C3="$TMP_ROOT/calls3" R3="$TMP_ROOT/r3"
make_home "$H3"
make_bin "$B3" "$C3"
mkdir -p "$H3/backups/restic"
flock "$H3/backups/restic/.opencode-db-backup.lock" sleep 10 &
lock_pid=$!
sleep 0.5
out="$(run_script "$H3" "$B3" "$R3" 2>&1)" || note_fail "lock skip exited non-zero: $out"
echo "$out" | grep -q "SKIP" || note_fail "lock skip missing SKIP line: $out"
[ ! -f "$C3" ] || note_fail "lock skip invoked restic"
kill "$lock_pid" 2>/dev/null || true
wait "$lock_pid" 2>/dev/null || true

if [ "$failures" -gt 0 ]; then
  echo "opencode-db-backup.test: $failures failure(s)" >&2
  exit 1
fi
echo "opencode-db-backup.test: all scenarios passed"
