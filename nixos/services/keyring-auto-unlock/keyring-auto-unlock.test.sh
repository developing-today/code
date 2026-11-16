#!/usr/bin/env bash
# keyring-auto-unlock.test.sh — isolated acceptance for code#10724.
#
# Spins a fully isolated Secret Service (private bus via
# dbus-run-session, scratch HOME + XDG_RUNTIME_DIR, scratch kernel-key
# name) and proves every leg of the mechanism that a headless session
# can prove:
#   1. addressing: the default collection resolves over the real
#      Secret Service bus name/interface and reports Locked=true in
#      the fresh state (a restarted daemon presenting a locked
#      default collection is the reported bug state). The draft bug
#      (interface org.freedesktop.secrets.* instead of
#      org.freedesktop.Secret.*) made this resolution return empty.
#   2. stash: the packaged keyring-password-stash caches the password
#      in the kernel user keyring, byte-exact, and replaces it on a
#      second run; empty input caches nothing.
#   3. real attempt, no silent success: with the collection locked
#      and a cache present, the packaged keyring-auto-unlock feeds
#      the cache to gnome-keyring-daemon --unlock and reports
#      still-locked (exit 1) when the daemon does not unlock — the
#      draft exited 0 silently here without ever reaching the daemon.
#      (In this scratch HOME there is no login.keyring file for
#      --unlock to decrypt; on the deployed system the file exists
#      from the real PAM login, which is the state --unlock acts on.)
#   4. no cache: with the cache dropped, the unlocker degrades to
#      today's behavior — exit 0, keyring left locked, named reason.
#   5. already unlocked: with the default alias pointed at the
#      (always-unlocked) session collection, the unlocker is a
#      silent no-op, exit 0.
#
# Boundary, established by experiment on 2026-11-16 (task 17508): a
# headless session cannot mint a login.keyring file — the daemon's
# --login/--start/--unlock paths create none without a real PAM
# session, and Secret Service CreateCollection requires an
# interactive prompter. The file-backed unlock leg is therefore
# exercised on the deployed system (post-switch live check recorded
# on issue #10724), not here. Nothing in this test touches the live
# daemon, the live collections, or the real cache key; the scratch
# kernel key is unlinked on exit (trap).
#
# Usage: keyring-auto-unlock.test.sh (no arguments). Needs the
# deployed package paths (post-switch): /run/current-system/sw/bin,
# and keyctl on PATH (the module installs keyutils for the user).
# Env knobs (tests/CI only): STASH_BIN, UNLOCK_BIN.
set -euo pipefail

if [ $# -ne 0 ]; then
  echo "usage: keyring-auto-unlock.test.sh (no arguments)" >&2
  exit 2
fi

STASH_BIN="${STASH_BIN:-/run/current-system/sw/bin/keyring-password-stash}"
UNLOCK_BIN="${UNLOCK_BIN:-/run/current-system/sw/bin/keyring-auto-unlock}"
TEST_KEY="keyring-auto-unlock-test-v1"
TEST_PW="correct horse battery staple $(date +%s)"

command -v dbus-run-session >/dev/null || { echo "need dbus-run-session"; exit 2; }
command -v keyctl >/dev/null || { echo "need keyctl (keyutils)"; exit 2; }
[ -x "${STASH_BIN}" ] || { echo "missing ${STASH_BIN} (switch first?)"; exit 2; }
[ -x "${UNLOCK_BIN}" ] || { echo "missing ${UNLOCK_BIN} (switch first?)"; exit 2; }

export KEYRING_CACHE_KEY="${TEST_KEY}"
export KEYRING_UNLOCK_ATTEMPTS=3
cleanup_key() { keyctl unlink "$(keyctl search @u user "${TEST_KEY}" 2>/dev/null)" @u 2>/dev/null || true; }
cleanup_key

T="$(mktemp -d "${TMPDIR:-/tmp}/kringtest.XXXXXX")"
mkdir -p "${T}/home" "${T}/run"
trap 'cleanup_key; rm -rf "${T}"' EXIT

cat >"${T}/body.sh" <<'BODY'
set -euo pipefail
NAME="org.freedesktop.secrets"
IFACE="org.freedesktop.Secret"
SP="/org/freedesktop/secrets"

default_col() {
  busctl --user call "${NAME}" "${SP}" "${IFACE}.Service" ReadAlias s default 2>/dev/null | sed -n 's/^o "\(.*\)"$/\1/p'
}
locked_of() {
  busctl --user get-property "${NAME}" "$1" "${IFACE}.Collection" Locked 2>/dev/null | awk '{print $2}'
}

start_daemon() {
  gnome-keyring-daemon --foreground --components=secrets &>"${T}/daemon-$1.log" &
  echo $! >"${T}/daemon.pid"
  for _ in $(seq 1 50); do
    [ -S "${XDG_RUNTIME_DIR}/keyring/control" ] && break
    sleep 0.2
  done
  for _ in $(seq 1 50); do
    busctl --user list 2>/dev/null | grep -q "^org.freedesktop.secrets" && break
    sleep 0.2
  done
  [ -S "${XDG_RUNTIME_DIR}/keyring/control" ] || { echo "daemon $1 never created control socket" >&2; return 1; }
}

stop_daemon() {
  kill -9 "$(cat "${T}/daemon.pid")" 2>/dev/null || true
  sleep 1
  rm -f "${XDG_RUNTIME_DIR}/keyring/control"
}

echo "--- 1. addressing: fresh daemon, default collection locked"
start_daemon 1
COL="$(default_col)"
[ -n "${COL}" ] || { echo "FAIL: default collection did not resolve (D-Bus addressing broken)"; exit 1; }
[ "$(locked_of "${COL}")" = "true" ] || { echo "FAIL: expected locked default collection in fresh state"; exit 1; }
echo "ok: default collection ${COL} resolves and is locked"

echo "--- 2. stash caches the password byte-exact and replaces it"
printf "%s\n" "${TEST_PW}" | KEYRING_STASH_USER="$(id -un)" "${STASH_BIN}" >/dev/null
KID="$(keyctl search @u user "${KEYRING_CACHE_KEY}" 2>/dev/null)" || KID=""
[ -n "${KID}" ] || { echo "FAIL: stash did not create kernel key"; exit 1; }
[ "$(keyctl pipe "${KID}")" = "${TEST_PW}" ] || { echo "FAIL: cached value mismatch"; exit 1; }
printf "%s\n" "${TEST_PW}-v2" | KEYRING_STASH_USER="$(id -un)" "${STASH_BIN}" >/dev/null
[ "$(keyctl pipe "$(keyctl search @u user "${KEYRING_CACHE_KEY}")")" = "${TEST_PW}-v2" ] || { echo "FAIL: stash did not replace cached value"; exit 1; }
printf "%s\n" "${TEST_PW}" | KEYRING_STASH_USER="$(id -un)" "${STASH_BIN}" >/dev/null
echo "ok: stash round-trip + replace"

echo "--- 3. unlocker attempts against the locked collection (no silent success)"
if "${UNLOCK_BIN}" >"${T}/unlock3.out" 2>"${T}/unlock3.err"; then
  echo "FAIL: unlocker exited 0 although the collection stayed locked (silent no-op regression)"
  exit 1
fi
grep -q "still locked" "${T}/unlock3.err" || { echo "FAIL: unlocker did not report the still-locked outcome"; cat "${T}/unlock3.err"; exit 1; }
[ "$(locked_of "${COL}")" = "true" ] || { echo "FAIL: collection state changed unexpectedly"; exit 1; }
echo "ok: unlocker reached the daemon, attempted, and reported still-locked"

echo "--- 4. no cache: graceful degradation, exit 0"
keyctl unlink "$(keyctl search @u user "${KEYRING_CACHE_KEY}")" @u
"${UNLOCK_BIN}" >"${T}/unlock4.out" 2>"${T}/unlock4.err"
grep -q "no cached password" "${T}/unlock4.err" || { echo "FAIL: no-cache path did not name its reason"; cat "${T}/unlock4.err"; exit 1; }
echo "ok: no-cache degradation"

echo "--- 5. already unlocked: silent no-op"
busctl --user call "${NAME}" "${SP}" "${IFACE}.Service" SetAlias so default /org/freedesktop/secrets/collection/session >/dev/null
OUT="$("${UNLOCK_BIN}" 2>&1)"
[ -z "${OUT}" ] || { echo "FAIL: unlocker spoke on the already-unlocked path: ${OUT}"; exit 1; }
busctl --user call "${NAME}" "${SP}" "${IFACE}.Service" SetAlias so default "${COL}" >/dev/null
echo "ok: already-unlocked no-op"

stop_daemon
echo "ALL PASS"
BODY

env HOME="${T}/home" XDG_RUNTIME_DIR="${T}/run" \
  TEST_PW="${TEST_PW}" TEST_KEY="${TEST_KEY}" T="${T}" \
  STASH_BIN="${STASH_BIN}" UNLOCK_BIN="${UNLOCK_BIN}" \
  dbus-run-session -- bash "${T}/body.sh"
