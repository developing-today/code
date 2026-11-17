# keyring-auto-unlock — re-unlock gnome-keyring after daemon restarts (code#10724).
#
# Triggered by the keyring-auto-unlock.path unit whenever the daemon's
# control socket changes (i.e. the daemon (re)started), and safe to run
# any time: it exits 0 without doing anything when the default
# collection is already unlocked or when no cached password exists
# (e.g. no graphical login has happened since boot — the pre-login
# state keeps today's behavior, per the repo doctrine that
# boot-independent services must use file-based secrets).
#
# The password comes from the kernel user keyring entry written by
# keyring-password-stash at graphical login — never from disk.
# Nothing secret is printed: stdout/stderr carry fixed strings only.
#
# Command line: none. Env: KEYRING_CACHE_KEY overrides the kernel key
# name (tests use a scratch key); KEYRING_UNLOCK_ATTEMPTS overrides
# the unlock retry count (tests use a small number).
set -euo pipefail

KEYRING_CACHE_KEY="${KEYRING_CACHE_KEY:-keyring:login-unlock}"
KEYRING_UNLOCK_ATTEMPTS="${KEYRING_UNLOCK_ATTEMPTS:-30}"

if [ $# -ne 0 ]; then
  echo "usage: keyring-auto-unlock (no arguments)" >&2
  exit 2
fi

# D-Bus addressing (verified against a live daemon, task 17508):
# the well-known bus NAME is org.freedesktop.secrets (plural), the
# object path is /org/freedesktop/secrets, but the INTERFACES are
# org.freedesktop.Secret.Service / .Collection (singular "Secret").
# Confusing the name with the interface prefix makes every call fail
# and the unlocker exit 0 silently — the draft bug this fixes.
NAME="org.freedesktop.secrets"
IFACE="org.freedesktop.Secret"
SERVICE_PATH="/org/freedesktop/secrets"

# Resolve the default collection (the "login" keyring PAM manages).
collection="$(busctl --user call "${NAME}" "${SERVICE_PATH}" \
  "${IFACE}.Service" ReadAlias s default 2>/dev/null | sed -n 's/^o "\(.*\)"$/\1/p')" || collection=""
if [ -z "${collection}" ]; then
  echo "keyring-auto-unlock: Secret Service not reachable; daemon may still be starting" >&2
  exit 0
fi

locked="$(busctl --user get-property "${NAME}" "${collection}" \
  "${IFACE}.Collection" Locked 2>/dev/null | awk '{print $2}')" || locked=""
if [ "${locked}" != "true" ]; then
  exit 0
fi

key_id="$(keyctl search @u user "${KEYRING_CACHE_KEY}" 2>/dev/null)" || key_id=""
if [ -z "${key_id}" ]; then
  echo "keyring-auto-unlock: collection locked and no cached password (login since boot?); leaving locked" >&2
  exit 0
fi

# The daemon may still be initializing (control socket exists before
# the bus name is claimable), so retry the unlock for a while.
attempt=0
while [ "${attempt}" -lt "${KEYRING_UNLOCK_ATTEMPTS}" ]; do
  if keyctl pipe "${key_id}" 2>/dev/null | gnome-keyring-daemon --unlock >/dev/null 2>&1; then
    sleep 1
    locked="$(busctl --user get-property "${NAME}" "${collection}" \
      "${IFACE}.Collection" Locked 2>/dev/null | awk '{print $2}')" || locked="true"
    if [ "${locked}" != "true" ]; then
      echo "keyring-auto-unlock: default collection re-unlocked after daemon restart"
      exit 0
    fi
  fi
  attempt=$((attempt + 1))
  sleep 1
done

echo "keyring-auto-unlock: unlock attempted but collection still locked (cached password may predate a password change; next graphical login refreshes it)" >&2
exit 1
