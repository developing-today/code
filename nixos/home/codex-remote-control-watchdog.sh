# codex-remote-control-watchdog — liveness check for the Codex
# app-server daemon (developing-today/code#1845). Run by the
# codex-remote-control-watchdog user timer from
# codex-remote-control.nix.
#
# The daemon is owned by the vendor CLI (see the module header), so
# the oneshot start unit cannot tell systemd when it dies. This
# check probes the CLI's own liveness surface
# (`codex app-server daemon version`): exit 0 means the daemon
# answered. Anything else means it is down, and the check runs
# `remote-control start` to bring it back. Every verdict is echoed
# to stdout, which lands in the journal under the check's unit, so
# a dead daemon is visible after the fact.
set -u

codex="$1"

if "$codex" app-server daemon version >/dev/null 2>&1; then
  echo "codex-remote-control-watchdog: daemon up"
  exit 0
fi

echo "codex-remote-control-watchdog: daemon down, running remote-control start"
if "$codex" remote-control start; then
  echo "codex-remote-control-watchdog: daemon restarted"
else
  echo "codex-remote-control-watchdog: restart FAILED" >&2
  exit 1
fi
