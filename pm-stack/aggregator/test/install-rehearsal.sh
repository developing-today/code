#!/usr/bin/env bash
# Safe deployment rehearsal: replace every external command that could
# touch a live service or require credentials, then install with an
# isolated HOME and assert the checkout/state layout.
set -euo pipefail

PKG="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d /tmp/opencode/pm-aggregator-install.XXXXXX)"
# The legacy-store seed below writes through the INSTALL_SRC symlink
# into the real package dir, so a pre-existing real store is copied
# aside first and restored on every exit path (Codex P1 on PR #8765):
# the rehearsal must never truncate or lose the checkout's own
# roadmap-annotations.json. (A package copy is not used for the
# fixture because install.sh derives its build version from
# `git rev-parse` at $SRC, which only resolves inside the real repo.)
LEGACY_REAL="$PKG/roadmap-annotations.json"
LEGACY_BACKUP="$TMP/real-roadmap-annotations.json"
SEEDED_LEGACY=0
cleanup() {
  if [ -f "$LEGACY_BACKUP" ]; then
    cp -a "$LEGACY_BACKUP" "$LEGACY_REAL"
  elif [ "$SEEDED_LEGACY" = 1 ]; then
    rm -f "$LEGACY_REAL"
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
BIN="$TMP/bin"
HOME_TEST="$TMP/home"
STATE="$HOME_TEST/.local/state/pm-aggregator"
INSTALL_SRC="$HOME_TEST/git/code/pm-stack/aggregator"
LOG="$TMP/commands.log"
REAL_CP="$(command -v cp)"
REAL_MV="$(command -v mv)"
mkdir -p "$BIN" "$HOME_TEST/pm-stack" "$STATE"
mkdir -p "$(dirname "$INSTALL_SRC")"
ln -s "$PKG" "$INSTALL_SRC"
ln -s "$PKG/../systemd" "$HOME_TEST/git/code/pm-stack/systemd"
ln -s "$STATE" "$HOME_TEST/pm-stack/aggregator"
UNIT_DST="$HOME_TEST/.config/systemd/user/pm-aggregator.service"
STORE_UNIT="$TMP/nix/store/home-manager-files/.config/systemd/user/pm-aggregator.service"
mkdir -p "$(dirname "$UNIT_DST")" "$(dirname "$STORE_UNIT")"
cp "$PKG/../systemd/pm-aggregator.service" "$STORE_UNIT"
printf '%s\n' '# preserved Home Manager unit' >>"$STORE_UNIT"
chmod 444 "$STORE_UNIT"
ln -s "$STORE_UNIT" "$UNIT_DST"
printf '{"kept":"existing feed"}\n' >"$STATE/fleet-feed.json"

cat >"$BIN/systemctl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'systemctl %s\n' "$*" >>"$TEST_LOG"
if [[ " $* " == *" is-active "* ]]; then
  if [[ " $* " == *" --quiet "* ]]; then exit 0; fi
  echo active
fi
STUB

cat >"$BIN/bun" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  -e)
    cat >/dev/null
    printf '%s\n' "$TEST_LIVE_VERSION"
    ;;
  install)
    printf 'bun-install cwd=%s\n' "$PWD" >>"$TEST_LOG"
    ;;
  *) exit 0 ;;
esac
STUB

cat >"$BIN/curl" <<'STUB'
#!/usr/bin/env bash
printf '{"version":"%s","status":"ok"}\n' "$TEST_LIVE_VERSION"
STUB

cat >"$BIN/sops" <<'STUB'
#!/usr/bin/env bash
# Test-only stand-in; never reads a real secret file.
if [ "${TEST_SOPS_FAIL:-0}" = 1 ]; then exit 1; fi
printf 'test-only-placeholder\n'
STUB

cat >"$BIN/cp" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
dest="${@: -1}"
if [[ "$dest" == "${TEST_UNIT_DST:?}" ]]; then
  echo "unexpected attempt to copy over Home Manager unit symlink: $dest" >&2
  exit 1
fi
exec "${TEST_REAL_CP:?}" "$@"
STUB

cat >"$BIN/mv" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
dest="${@: -1}"
if [ "${TEST_MV_FAIL:-0}" = 1 ] && [[ "$dest" == *"git/code/pm-stack/aggregator/roadmap-annotations.json" ]]; then
  echo "injected mv failure for rollback" >&2
  exit 1
fi
exec "${TEST_REAL_MV:?}" "$@"
STUB

chmod +x "$BIN/systemctl" "$BIN/bun" "$BIN/curl" "$BIN/sops" "$BIN/cp" "$BIN/mv"
if TEST_LOG="$LOG" TEST_UNIT_DST="$UNIT_DST" TEST_REAL_CP="$REAL_CP" HOME="$HOME_TEST" PATH="$BIN:$PATH" \
  bash "$PKG/install.sh" >"$TMP/worktree-refused.out" 2>&1; then
  echo 'install unexpectedly accepted a non-primary checkout' >&2
  exit 1
fi
if [ -f "$LEGACY_REAL" ]; then cp -a "$LEGACY_REAL" "$LEGACY_BACKUP"; fi
printf '%s\n' '{"kept":"legacy annotations"}' >"$INSTALL_SRC/roadmap-annotations.json"
SEEDED_LEGACY=1
TEST_LOG="$LOG" \
  TEST_UNIT_DST="$UNIT_DST" \
  TEST_REAL_CP="$REAL_CP" TEST_REAL_MV="$REAL_MV" \
  TEST_LIVE_VERSION="$(git -C "$PKG" rev-parse HEAD)" \
  HOME="$HOME_TEST" PATH="$BIN:$PATH" \
  bash "$INSTALL_SRC/install.sh" >"$TMP/install.out"
TEST_LOG="$LOG" \
  TEST_UNIT_DST="$UNIT_DST" \
  TEST_REAL_CP="$REAL_CP" TEST_REAL_MV="$REAL_MV" \
  TEST_LIVE_VERSION="$(git -C "$PKG" rev-parse HEAD)" \
  HOME="$HOME_TEST" PATH="$BIN:$PATH" \
  bash "$INSTALL_SRC/install.sh" >"$TMP/install-second.out"

test -f "$STATE/.env"
test "$(stat -c '%a' "$STATE")" = 700
test "$(stat -c '%a' "$STATE/.env")" = 600
test -d "$STATE/snapshots"
test -f "$STATE/.deploy-tree-hash"
grep -Fx "PM_AGG_FEED_FILE=$STATE/fleet-feed.json" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_COMMENTS_FILE=$STATE/comments.json" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_ACTIONS_FILE=$STATE/actions.json" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_REGISTRY_FILE=$STATE/registry.json" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_SNAPSHOTS_DIR=$STATE/snapshots" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_QUOTA_CURRENT_FILE=$STATE/quota-current.json" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_COVERAGE_SNAPSHOT_FILE=$STATE/coverage-snapshot.json" "$STATE/.env" >/dev/null
grep -Fx "PM_AGG_ROADMAP_ANNOTATIONS_FILE=$STATE/roadmap-annotations.json" "$STATE/.env" >/dev/null

# The legacy checkout store is migrated, not orphaned (#8763).
grep -Fx '{"kept":"legacy annotations"}' "$STATE/roadmap-annotations.json" >/dev/null
test ! -e "$INSTALL_SRC/roadmap-annotations.json"

grep -Fx 'WorkingDirectory=%h/git/code/pm-stack/aggregator' \
  "$HOME_TEST/.config/systemd/user/pm-aggregator.service" >/dev/null
grep -Fx 'EnvironmentFile=%h/.local/state/pm-aggregator/.env' \
  "$UNIT_DST" >/dev/null
grep -Fx '# preserved Home Manager unit' "$UNIT_DST" >/dev/null
test -L "$UNIT_DST"
test "$(readlink "$UNIT_DST")" = "$STORE_UNIT"
grep -Fx 'WorkingDirectory=%h/git/code/pm-stack/aggregator' \
  "$HOME_TEST/.config/systemd/user/quota-gatherer.service" >/dev/null
grep -Fx 'Environment=QUOTA_CURRENT_PATH=%h/.local/state/pm-aggregator/quota-current.json' \
  "$HOME_TEST/.config/systemd/user/quota-gatherer.service" >/dev/null

# The old feed bridge survives; the package itself is no longer copied
# into the state directory.
test -L "$HOME_TEST/pm-stack/aggregator"
test "$(readlink "$HOME_TEST/pm-stack/aggregator")" = "$STATE"
test ! -e "$STATE/src"
grep -Fx '{"kept":"existing feed"}' "$STATE/fleet-feed.json" >/dev/null
test "$(grep -c -Fx "bun-install cwd=$INSTALL_SRC" "$LOG")" = 1
test "$(grep -c -F 'systemctl --user restart pm-aggregator.service' "$LOG")" = 1
grep -F 'DEST_DIR="~/.local/state/pm-aggregator"' "$PKG/scripts/push-fleet-feed.sh" >/dev/null

# Failure recovery (CodeRabbit on PR #8765): with the annotations
# destination already populated, a changed-inputs install must stop
# the writer before the both-exist warning too, and a sops failure
# after that stop must restart the service on the previous
# configuration -- never leave it stopped with a half-rendered env.
printf '%s\n' '{"kept":"legacy annotations"}' >"$INSTALL_SRC/roadmap-annotations.json"
SEEDED_LEGACY=1
mkdir -p "$HOME_TEST/secrets"
printf 'dummy: not-a-real-secret\n' >"$HOME_TEST/secrets/pm-stack.sops.yaml"
if TEST_LOG="$LOG" TEST_UNIT_DST="$UNIT_DST" TEST_REAL_CP="$REAL_CP" TEST_REAL_MV="$REAL_MV" TEST_SOPS_FAIL=1 TEST_LIVE_VERSION="$(git -C "$PKG" rev-parse HEAD)" HOME="$HOME_TEST" PATH="$BIN:$PATH" bash "$INSTALL_SRC/install.sh" >"$TMP/install-failed.out" 2>&1; then
  echo 'install unexpectedly succeeded with a failing sops' >&2
  exit 1
fi
test "$(grep -c -F 'systemctl --user stop pm-aggregator.service' "$LOG")" = 2
test "$(grep -c -F 'systemctl --user start pm-aggregator.service' "$LOG")" = 1
test "$(grep -c -Fx 'systemctl --user restart pm-aggregator.service' "$LOG")" = 1
grep -Fx '{"kept":"legacy annotations"}' "$INSTALL_SRC/roadmap-annotations.json" >/dev/null
grep -Fx '{"kept":"legacy annotations"}' "$STATE/roadmap-annotations.json" >/dev/null
test ! -e "$STATE/.env.tmp"
grep -Fx "PM_AGG_ROADMAP_ANNOTATIONS_FILE=$STATE/roadmap-annotations.json" "$STATE/.env" >/dev/null

# Failed-rollback coverage (CodeRabbit on PR #8765): the migration
# succeeds, the env render fails, and the rollback move itself fails.
# The service must be left stopped -- starting it would point the
# previous configuration at a missing store -- the failure must be
# reported, and the migrated store stays in the state directory.
printf '%s\n' '{"kept":"legacy annotations"}' >"$INSTALL_SRC/roadmap-annotations.json"
SEEDED_LEGACY=1
mv "$STATE/roadmap-annotations.json" "$TMP/state-annotations-aside.json"
printf 'dummy: changed-again\n' >"$HOME_TEST/secrets/pm-stack.sops.yaml"
if TEST_LOG="$LOG" TEST_UNIT_DST="$UNIT_DST" TEST_REAL_CP="$REAL_CP" TEST_REAL_MV="$REAL_MV" TEST_MV_FAIL=1 TEST_SOPS_FAIL=1 TEST_LIVE_VERSION="$(git -C "$PKG" rev-parse HEAD)" HOME="$HOME_TEST" PATH="$BIN:$PATH" bash "$INSTALL_SRC/install.sh" >"$TMP/install-rollback-failed.out" 2>&1; then
  echo 'install unexpectedly succeeded with failing sops and mv' >&2
  exit 1
fi
grep -F 'could not move the migrated roadmap annotations' "$TMP/install-rollback-failed.out" >/dev/null
test "$(grep -c -F 'systemctl --user start pm-aggregator.service' "$LOG")" = 1
grep -Fx '{"kept":"legacy annotations"}' "$STATE/roadmap-annotations.json" >/dev/null
test ! -e "$INSTALL_SRC/roadmap-annotations.json"
test ! -e "$STATE/.env.tmp"

echo 'install rehearsal passed (isolated HOME; no live service or credentials used)'
