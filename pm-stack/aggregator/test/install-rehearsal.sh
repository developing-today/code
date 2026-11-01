#!/usr/bin/env bash
# Safe deployment rehearsal: replace every external command that could
# touch a live service or require credentials, then install with an
# isolated HOME and assert the checkout/state layout.
set -euo pipefail

PKG="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d /tmp/opencode/pm-aggregator-install.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
HOME_TEST="$TMP/home"
STATE="$HOME_TEST/.local/state/pm-aggregator"
INSTALL_SRC="$HOME_TEST/git/code/pm-stack/aggregator"
LOG="$TMP/commands.log"
mkdir -p "$BIN" "$HOME_TEST/pm-stack" "$STATE"
mkdir -p "$(dirname "$INSTALL_SRC")"
ln -s "$PKG" "$INSTALL_SRC"
ln -s "$PKG/../systemd" "$HOME_TEST/git/code/pm-stack/systemd"
ln -s "$STATE" "$HOME_TEST/pm-stack/aggregator"
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
printf 'test-only-placeholder\n'
STUB

chmod +x "$BIN/systemctl" "$BIN/bun" "$BIN/curl" "$BIN/sops"
if TEST_LOG="$LOG" HOME="$HOME_TEST" PATH="$BIN:$PATH" \
  bash "$PKG/install.sh" >"$TMP/worktree-refused.out" 2>&1; then
  echo 'install unexpectedly accepted a non-primary checkout' >&2
  exit 1
fi
TEST_LOG="$LOG" \
TEST_LIVE_VERSION="$(git -C "$PKG" rev-parse HEAD)" \
HOME="$HOME_TEST" PATH="$BIN:$PATH" \
  bash "$INSTALL_SRC/install.sh" >"$TMP/install.out"
TEST_LOG="$LOG" \
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

grep -Fx 'WorkingDirectory=%h/git/code/pm-stack/aggregator' \
  "$HOME_TEST/.config/systemd/user/pm-aggregator.service" >/dev/null
grep -Fx 'EnvironmentFile=%h/.local/state/pm-aggregator/.env' \
  "$HOME_TEST/.config/systemd/user/pm-aggregator.service" >/dev/null
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

echo 'install rehearsal passed (isolated HOME; no live service or credentials used)'
