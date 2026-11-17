# keyring-password-stash — PAM auth helper (code#10724).
#
# Called by pam_exec (expose_authtok) from the graphical-login PAM stack.
# Reads the just-typed login password from stdin and stashes it in the
# kernel user keyring, where keyring-auto-unlock can reach it after an
# out-of-session gnome-keyring-daemon restart. The password is NEVER
# written to disk: kernel keyrings are RAM-only, UID-scoped, and die
# with the user (reboot clears them; the next graphical login
# re-stashes). This satisfies the issue's "does NOT store the keyring
# password in plaintext" constraint — there is no file to steal, only
# a live in-memory key readable by this UID.
#
# Command line: none. Reads stdin (PAM authtok + one trailing newline),
# configured via env: KEYRING_CACHE_KEY (default below),
# KEYRING_STASH_USER (default below).
# Exits 0 always: a stash failure must never fail a login (the PAM
# rule is `optional`, and this is belt-and-braces).
KEYRING_CACHE_KEY="${KEYRING_CACHE_KEY:-keyring:login-unlock}"
# pam_exec exports PAM_USER for the authenticating user; honor an
# explicit override (tests) the same way.
KEYRING_STASH_USER="${KEYRING_STASH_USER:-${PAM_USER:-user}}"

if [ $# -ne 0 ]; then
  echo "usage: keyring-password-stash (no arguments; reads password from stdin)" >&2
  exit 2
fi

# Run a command as the target user when we are root (PAM auth runs as
# root); otherwise run it directly (tests, manual runs).
as_target() {
  if [ "$(id -u)" -eq 0 ]; then
    runuser -u "${KEYRING_STASH_USER}" -- "$@"
  else
    "$@"
  fi
}

# PAM appends a single trailing newline to the authtok; command
# substitution already strips all trailing newlines, the extra trim is
# explicit documentation. Empty input (fingerprint login, autologin,
# no password) means there is nothing to cache — exit quietly.
password="$(cat)"
password="${password%$'\n'}"
if [ -z "${password}" ]; then
  exit 0
fi

# Replace any previous entry (e.g. from an earlier login) so a changed
# password cannot linger beside the current one.
old_id="$(as_target keyctl search @u user "${KEYRING_CACHE_KEY}" 2>/dev/null)" || old_id=""
if [ -n "${old_id}" ]; then
  as_target keyctl unlink "${old_id}" @u 2>/dev/null || true
fi

if ! printf '%s' "${password}" | as_target keyctl padd user "${KEYRING_CACHE_KEY}" @u >/dev/null 2>&1; then
  echo "keyring-password-stash: failed to cache login password in kernel keyring" >&2
fi

# Drop the secret from this shell's memory as far as bash allows.
password=""
exit 0
