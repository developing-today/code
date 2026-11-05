#!/usr/bin/env bash
# Tests for pm-dashboard-backup.sh's pmp_links export (code#1744 P0):
#   pmp_links is the only non-rebuildable table in the projector DB, so
#   the dashboard archive must carry a pmp_links-only export on all
#   three legs (local dated archive, R2, Drive — the archive tarball is
#   the single artefact all three legs copy, so membership in the
#   verified tarball + the legs' sha256 verification is the coverage).
#   The script resolves the ACTUAL configured projector DB (the
#   pm-projector env file's database URL, else the deployed default
#   ~/.local/state/pm-projector/projector.db), stages ONLY the
#   pmp_links schema+rows as projector/pmp_links.sql, and records it
#   in the manifest as a conditionally-required member: required
#   whenever a projector DB was present, optional when there is none.
#   A present but unreadable/corrupt DB fails the run before any
#   upload — a missing or corrupt export can never report success.
# This drives the REAL script with stubbed tools (the
# pm-dashboard-backup-manifest.test.sh harness shape): scratch HOME,
# stub bws, stub journalctl, stub rclone backed by a local store.
# Scenarios:
#   * with-db — fixture DB (pmp_links rows + a pmp_items decoy table)
#     at the default state path: run completes and uploads; the
#     archive's projector/pmp_links.sql restores into a scratch DB
#     with content identical to the source, contains NO pmp_items
#     data, and the manifest records the member required + present.
#   * env-url — the env file's database URL points at a fixture DB
#     elsewhere: the export comes from THAT database.
#   * no-db — no projector DB anywhere: run completes, manifest
#     records the member optional + missing.
#   * corrupt-db — the configured DB file is garbage: run fails,
#     nothing is uploaded, the journal marker is unchanged, and the
#     log names pmp_links.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SELF_DIR/pm-dashboard-backup.sh" ]]; then
  SCRIPT="$SELF_DIR/pm-dashboard-backup.sh"
else
  SCRIPT="$SELF_DIR/modules/pm-dashboard-backup.sh"
fi

command -v sqlite3 >/dev/null 2>&1 || {
  echo "pm-dashboard-backup pmp_links tests: sqlite3 required" >&2
  exit 1
}

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}
ok() { echo "ok: $1"; }

BIN="$TMP_ROOT/bin"
mkdir -p "$BIN"

{ printf '#!%s\n' "$BASH"; cat <<'STUB'
if [[ "${1:-} ${2:-}" == "secret list" ]]; then
  printf '%s\n' '[{"key":"cloudflare.r2-backups-access-key-id","value":"fake-access-key-id"},{"key":"cloudflare.r2-backups-secret-access-key","value":"fake-secret-access-key"},{"key":"cloudflare.r2-endpoint","value":"https://example.invalid"}]'
  exit 0
fi
echo "stub bws: unsupported args: $*" >&2
exit 1
STUB
} >"$BIN/bws"

{ printf '#!%s\n' "$BASH"; cat <<'STUB'
echo "stub journal line"
exit 0
STUB
} >"$BIN/journalctl"

{ printf '#!%s\n' "$BASH"; cat <<'STUB'
store="${RCLONE_STUB_DIR:?RCLONE_STUB_DIR unset}"
calls="${RCLONE_CALLS_LOG:?RCLONE_CALLS_LOG unset}"
mkdir -p "$store"
echo "$*" >>"$calls"
key() { printf '%s' "$1" | tr '/:' '__'; }
cmd="${1:-}"
shift || true
case "$cmd" in
  copyto) cp "$1" "$store/$(key "$2")" ;;
  cat)
    f="$store/$(key "$1")"
    [[ -f "$f" ]] || exit 1
    cat "$f"
    ;;
  lsf) exit 0 ;;
  purge) exit 0 ;;
  *)
    echo "stub rclone: unsupported command: $cmd" >&2
    exit 1
    ;;
esac
STUB
} >"$BIN/rclone"
chmod +x "$BIN/bws" "$BIN/journalctl" "$BIN/rclone"

PRESET=1000000000
RC=0
MARKER_AFTER=""
RUN_HOME=""

setup_home() {
  local name="$1"
  RUN_HOME="$TMP_ROOT/$name/home"
  mkdir -p "$RUN_HOME/backups/dashboard" "$RUN_HOME/.local/state/pm-aggregator" \
    "$RUN_HOME/atlas/meta-quota-export"
  local f
  for f in actions.json comments.json fleet-feed.json quota-current.json \
    registry.json quota-gatherer-state.json quota-gatherer-heartbeat.json \
    events.json coverage-snapshot.json aa-last-good.json; do
    printf '{}\n' >"$RUN_HOME/.local/state/pm-aggregator/$f"
  done
  printf '{}\n' >"$RUN_HOME/atlas/meta-quota-export/latest.json"
  printf 'history\n' >"$RUN_HOME/atlas/meta-quota-export/history.jsonl"
  echo "$PRESET" >"$RUN_HOME/backups/dashboard/.last-run-epoch"
}

# make_fixture_db <path> <marker-note>
make_fixture_db() {
  mkdir -p "$(dirname "$1")"
  sqlite3 "$1" <<SQL
CREATE TABLE pmp_items (ref TEXT PRIMARY KEY, title TEXT);
INSERT INTO pmp_items VALUES ('gh:secret-projection', 'DECOY-PROJECTION-ROW');
CREATE TABLE pmp_links (
  subject TEXT NOT NULL, ref TEXT NOT NULL,
  relation TEXT NOT NULL DEFAULT 'relates', note TEXT,
  created_by TEXT, created_at TEXT NOT NULL,
  PRIMARY KEY (subject, ref, relation)
);
CREATE INDEX pmp_links_subject ON pmp_links(subject);
INSERT INTO pmp_links VALUES ('issue:1', 'issue:2', 'blocks', '$2', 'tester', '2026-11-04T00:00:00Z');
INSERT INTO pmp_links VALUES ('issue:3', 'pr:9', 'relates', NULL, 'tester', '2026-11-04T00:01:00Z');
SQL
}

run_backup() {
  local name="$1"
  HOME="$RUN_HOME" PATH="$BIN:$PATH" BWS_ACCESS_TOKEN="test-token" \
    RCLONE_STUB_DIR="$TMP_ROOT/$name/rclone-store" \
    RCLONE_CALLS_LOG="$TMP_ROOT/$name/rclone-calls.log" \
    PM_DB_DUMP_SCRIPT="$RUN_HOME/no-such-dump-script.sh" \
    bash "$SCRIPT" 2026-10-28 >"$TMP_ROOT/$name.log" 2>&1
  RC=$?
  MARKER_AFTER="$(cat "$RUN_HOME/backups/dashboard/.last-run-epoch" 2>/dev/null || echo missing)"
}

archive_dir() { # $1 = scenario name; extracts the run archive to a dir, prints it
  local arc="$RUN_HOME/backups/dashboard/2026-10-28/dashboard-2026-10-28.tar.zst"
  local d="$TMP_ROOT/$1-extract"
  mkdir -p "$d"
  tar -I zstd -xf "$arc" -C "$d" 2>/dev/null
  printf '%s' "$d"
}

assert_member() { # $1 scenario, $2 class, $3 status
  local d
  d="$(archive_dir "$1")"
  if python3 -c '
import json, sys
m = json.load(open(sys.argv[1]))
for e in m.get("expected", []):
    if e["path"] == "projector/pmp_links.sql":
        sys.exit(0 if (e["class"] == sys.argv[2] and e["status"] == sys.argv[3]) else 1)
sys.exit(1)
' "$d/manifest.json" "$2" "$3"; then
    ok "$1: manifest records projector/pmp_links.sql as $2 + $3"
  else
    fail "$1: manifest does not record projector/pmp_links.sql as $2 + $3"
  fi
}

no_upload() { # $1 scenario name
  [[ ! -f "$TMP_ROOT/$1/rclone-calls.log" ]] \
    || ! grep -q '^copyto' "$TMP_ROOT/$1/rclone-calls.log"
}

canonical_links() { # $1 = sqlite db path; canonical pmp_links content
  sqlite3 "file:$1?mode=ro" '.dump pmp_links'
}

# --- with-db: export staged, restorable, links-only ---------------------------
setup_home with-db
make_fixture_db "$RUN_HOME/.local/state/pm-projector/projector.db" 'first-link-note'
run_backup with-db
if [[ "$RC" -eq 0 ]]; then
  ok "with-db: script exited 0"
else
  fail "with-db: expected exit 0, got $RC; log: $(cat "$TMP_ROOT/with-db.log")"
fi
D="$(archive_dir with-db)"
if [[ -f "$D/projector/pmp_links.sql" ]]; then
  ok "with-db: archive contains projector/pmp_links.sql"
else
  fail "with-db: archive lacks projector/pmp_links.sql"
fi
if grep -q 'CREATE INDEX pmp_links_subject' "$D/projector/pmp_links.sql" 2>/dev/null; then
  ok "with-db: export carries the pmp_links index definitions"
else
  fail "with-db: export lacks the pmp_links index definitions"
fi
if grep -q 'DECOY-PROJECTION-ROW' "$D/projector/pmp_links.sql" 2>/dev/null \
  || grep -q 'pmp_items' "$D/projector/pmp_links.sql" 2>/dev/null; then
  fail "with-db: export leaks non-pmp_links content (projection tables must not be archived)"
else
  ok "with-db: export is pmp_links-only (no projection tables)"
fi
SCRATCH="$TMP_ROOT/with-db-scratch.db"
rm -f "$SCRATCH"
if sqlite3 "$SCRATCH" <"$D/projector/pmp_links.sql" 2>/dev/null \
  && [[ "$(canonical_links "$SCRATCH")" == "$(canonical_links "$RUN_HOME/.local/state/pm-projector/projector.db")" ]] \
  && [[ "$(canonical_links "$SCRATCH")" == *"first-link-note"* ]]; then
  ok "with-db: isolated restore reproduces pmp_links content exactly"
else
  fail "with-db: restored pmp_links content differs from source"
fi
assert_member with-db required present
if ! no_upload with-db; then
  ok "with-db: archive uploaded to the offsite legs (stub rclone copyto recorded)"
else
  fail "with-db: expected uploads to the offsite legs"
fi

# --- env-url: the env file's database URL selects the source DB ----------------
setup_home env-url
mkdir -p "$RUN_HOME/elsewhere" "$RUN_HOME/.local/state/pm-projector"
make_fixture_db "$RUN_HOME/elsewhere/custom.db" 'env-url-link-note'
printf '%s=file:%s\n' 'DATABASE_URL' "$RUN_HOME/elsewhere/custom.db" \
  >"$RUN_HOME/.local/state/pm-projector/.env"
run_backup env-url
D="$(archive_dir env-url)"
if [[ "$RC" -eq 0 ]] && grep -q 'env-url-link-note' "$D/projector/pmp_links.sql" 2>/dev/null; then
  ok "env-url: export sourced from the env-configured database"
else
  fail "env-url: export did not come from the env-configured database (rc=$RC); log: $(cat "$TMP_ROOT/env-url.log")"
fi
assert_member env-url required present

# --- no-db: not applicable, recorded optional + missing ------------------------
setup_home no-db
run_backup no-db
if [[ "$RC" -eq 0 ]]; then
  ok "no-db: script exited 0"
else
  fail "no-db: expected exit 0, got $RC; log: $(cat "$TMP_ROOT/no-db.log")"
fi
assert_member no-db optional missing

# --- corrupt-db: fail before upload, marker unchanged --------------------------
setup_home corrupt-db
mkdir -p "$RUN_HOME/.local/state/pm-projector"
printf 'this is not a sqlite database\n' >"$RUN_HOME/.local/state/pm-projector/projector.db"
run_backup corrupt-db
if [[ "$RC" -ne 0 ]]; then
  ok "corrupt-db: script exited non-zero ($RC)"
else
  fail "corrupt-db: expected non-zero exit, got 0; log: $(cat "$TMP_ROOT/corrupt-db.log")"
fi
if grep -q 'pmp_links' "$TMP_ROOT/corrupt-db.log"; then
  ok "corrupt-db: failure output names pmp_links"
else
  fail "corrupt-db: failure output does not name pmp_links: $(cat "$TMP_ROOT/corrupt-db.log")"
fi
if no_upload corrupt-db; then
  ok "corrupt-db: nothing uploaded"
else
  fail "corrupt-db: an upload was attempted"
fi
if [[ "$MARKER_AFTER" == "$PRESET" ]]; then
  ok "corrupt-db: journal marker unchanged"
else
  fail "corrupt-db: marker advanced — was $PRESET, now $MARKER_AFTER"
fi

if (( failures > 0 )); then
  echo "pm-dashboard-backup pmp_links tests: $failures failure(s)" >&2
  exit 1
fi
echo "pm-dashboard-backup pmp_links tests: all passed"
