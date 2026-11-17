#!/usr/bin/env bash
# Tests for pm-dashboard-backup.sh's monthly tier (#10299, Drew's
# 2026-11-03 ruling on #1258 Q3/Q6: daily generations 30 days PLUS
# monthly generations kept 12 months off-machine):
#   * a month with no generation gets the run's archive pinned under
#     monthly/<YYYY-MM>/ on BOTH offsite legs, sha256-verified by the
#     script itself;
#   * a month that already has a generation keeps its first one
#     (later same-month runs do not overwrite it);
#   * monthly generations older than 12 months are pruned on both
#     legs; a 6-month-old generation is kept; the run's own month is
#     never pruned.
# Drives the REAL script with stubbed bws / journalctl / rclone
# against a scratch HOME and a fake remote tree (modeled on
# pm-dashboard-backup-prune.test.sh; the rclone stub's lsf honors
# --dirs-only vs plain listing like real rclone).
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/pm-dashboard-backup.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}
check() {
  local desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "ok: $desc"
  else
    fail "$desc"
  fi
}

HOME_DIR="$TMP_ROOT/home"
BIN="$TMP_ROOT/bin"
REMOTE="$TMP_ROOT/remote"
mkdir -p "$HOME_DIR/.local/state/pm-aggregator" "$HOME_DIR/atlas/meta-quota-export" "$BIN" "$REMOTE"

for f in actions.json comments.json fleet-feed.json quota-current.json \
  registry.json quota-gatherer-state.json quota-gatherer-heartbeat.json \
  events.json coverage-snapshot.json aa-last-good.json; do
  echo "{\"fixture\": \"$f\"}" >"$HOME_DIR/.local/state/pm-aggregator/$f"
done
echo '{"fixture": "latest"}' >"$HOME_DIR/atlas/meta-quota-export/latest.json"
echo '{"fixture": "history"}' >"$HOME_DIR/atlas/meta-quota-export/history.jsonl"

cat >"$BIN/bws" <<'EOF'
#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "list" ]; then
  cat <<'JSON'
[{"key":"cloudflare.r2-backups-access-key-id","value":"test-access-key-id"},
 {"key":"cloudflare.r2-backups-secret-access-key","value":"test-secret-access-key"},
 {"key":"cloudflare.r2-endpoint","value":"https://r2.example.invalid"}]
JSON
  exit 0
fi
echo "bws stub: unexpected args: $*" >&2
exit 1
EOF

cat >"$BIN/journalctl" <<'EOF'
#!/bin/sh
echo "fixture journal line"
exit 0
EOF

cat >"$BIN/rclone" <<'EOF'
#!/bin/sh
set -u
map_path() {
  remote="${1%%:*}"
  rest="${1#*:}"
  printf '%s/%s/%s' "$FAKE_REMOTE" "$remote" "$rest"
}
cmd="$1"; shift
case "$cmd" in
  copyto)
    dest="$(map_path "$2")"
    mkdir -p "$(dirname "$dest")"
    cp "$1" "$dest"
    ;;
  cat)
    cat "$(map_path "$1")"
    ;;
  lsf)
    dirs_only=0
    for a in "$@"; do
      [ "$a" = "--dirs-only" ] && dirs_only=1
      last="$a"
    done
    dir="$(map_path "$last")"
    [ -d "$dir" ] || exit 0
    if [ "$dirs_only" = "1" ]; then
      for d in "$dir"/*/; do
        [ -d "$d" ] && basename "$d" | sed 's|$|/|'
      done
    else
      for e in "$dir"/*; do
        [ -e "$e" ] && basename "$e"
      done
    fi
    ;;
  purge)
    rm -rf "$(map_path "$1")"
    ;;
  *)
    echo "rclone stub: unexpected command: $cmd" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$BIN/bws" "$BIN/journalctl" "$BIN/rclone"

TODAY="$(date +%F)"
MONTH="$(date +%Y-%m)"
FIRST="$(date +%Y-%m-01)"
OLD_MONTH="2020-01"
RECENT_MONTH="$(date -d "6 months ago" +%Y-%m)"
R2_MONTHLY="$REMOTE/r2bak/supabase-backups/dashboard/monthly"
DRIVE_MONTHLY="$REMOTE/drive/Cloud/cold-store/db-backups/pm-aggregator/monthly"

# Seed: current month already pinned (sentinel = first-of-month
# generation), an expired month, and a 6-month-old month, both legs.
for base in "$R2_MONTHLY" "$DRIVE_MONTHLY"; do
  mkdir -p "$base/$MONTH" "$base/$OLD_MONTH" "$base/$RECENT_MONTH"
  echo sentinel >"$base/$MONTH/dashboard-$FIRST.tar.zst"
  echo old >"$base/$OLD_MONTH/dashboard-$OLD_MONTH-01.tar.zst"
  echo recent >"$base/$RECENT_MONTH/dashboard-$RECENT_MONTH-01.tar.zst"
done

run_backup() {
  HOME="$HOME_DIR" PATH="$BIN:$PATH" FAKE_REMOTE="$REMOTE" \
    BWS_ACCESS_TOKEN=dummy-test-token \
    bash "$SCRIPT" "$1" >"$TMP_ROOT/run-$1.log" 2>&1
  RC=$?
}

# --- scenario 1: same-month run keeps the first generation -------------------
run_backup "$TODAY"
if [ "$RC" -eq 0 ]; then
  echo "ok: run exits 0"
else
  fail "run exits 0 - got $RC; log: $(cat "$TMP_ROOT/run-$TODAY.log")"
fi
for base in "$R2_MONTHLY" "$DRIVE_MONTHLY"; do
  check "first-of-month generation kept on $base" \
    grep -q sentinel "$base/$MONTH/dashboard-$FIRST.tar.zst"
  check "expired monthly generation pruned on $base" \
    test ! -e "$base/$OLD_MONTH"
  check "6-month-old monthly generation kept on $base" \
    test -f "$base/$RECENT_MONTH/dashboard-$RECENT_MONTH-01.tar.zst"
done
if [ "$TODAY" != "$FIRST" ]; then
  for base in "$R2_MONTHLY" "$DRIVE_MONTHLY"; do
    check "later same-month archive not pinned over the first on $base" \
      test ! -e "$base/$MONTH/dashboard-$TODAY.tar.zst"
  done
fi
check "log names the kept monthly generation (R2)" \
  grep -q "monthly R2 $MONTH already pinned" "$TMP_ROOT/run-$TODAY.log"
check "log names the monthly prune (R2)" \
  grep -q "pruned monthly R2 $OLD_MONTH" "$TMP_ROOT/run-$TODAY.log"

# --- scenario 2: a month with no generation gets pinned ----------------------
for base in "$R2_MONTHLY" "$DRIVE_MONTHLY"; do
  rm -rf "$base/$MONTH"
done
run_backup "$TODAY"
if [ "$RC" -eq 0 ]; then
  echo "ok: pin run exits 0"
else
  fail "pin run exits 0 - got $RC; log: $(cat "$TMP_ROOT/run-$TODAY.log")"
fi
for base in "$R2_MONTHLY" "$DRIVE_MONTHLY"; do
  check "monthly generation pinned on $base" \
    test -f "$base/$MONTH/dashboard-$TODAY.tar.zst"
  check "pinned monthly copy matches the daily archive on $base" \
    cmp "$base/$MONTH/dashboard-$TODAY.tar.zst" \
    "$HOME_DIR/backups/dashboard/$TODAY/dashboard-$TODAY.tar.zst"
done
check "log names the monthly pin (Drive)" \
  grep -q "monthly Drive $MONTH pinned from $TODAY" "$TMP_ROOT/run-$TODAY.log"

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all pm-dashboard-backup monthly checks passed"
