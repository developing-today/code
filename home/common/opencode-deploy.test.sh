#!/usr/bin/env bash
# Regression test for same-build OpenCode deployment. Oracle: a fake
# systemctl invocation log must stay empty for an unchanged store-path key.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

systemctl="$tmp/systemctl"
log="$tmp/systemctl.log"
state="$tmp/deployed"
proc_root="$tmp/proc"
cat >"$systemctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$SYSTEMCTL_LOG"
case "$*" in
  '--user show --property=MainPID --value opencode.service') printf '4242\n' ;;
  '--user is-system-running')
    printf '%s\n' "${SYSTEMD_STATE:-running}"
    [[ "${SYSTEMD_STATE:-running}" == running ]] ;;
  '--user is-active --quiet opencode.socket') [[ -f "$SOCKET_ACTIVE" ]]; exit $? ;;
  '--user is-enabled --quiet opencode.socket') exit "${SOCKET_ENABLED:-1}" ;;
  '--user start opencode.socket') : >"$SOCKET_ACTIVE" ; exit 0 ;;
esac
exit 0
EOF
chmod +x "$systemctl"
export SYSTEMCTL_LOG="$log"
export SOCKET_ACTIVE="$tmp/socket-active"

printf '%s\n' '/nix/store/opencode-same-build' >"$state"

"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-same-build' "$state" "$systemctl" "$proc_root"

if [[ -e $log ]]; then
  echo "FAIL: unchanged OpenCode build called systemctl:" >&2
  cat "$log" >&2
  exit 1
fi

echo 'ok - unchanged store path does not invoke systemctl'

# Bootstrap case: the marker does not exist yet, but the active process
# command line proves it already runs this store path.
rm -f "$state"
mkdir -p "$proc_root/4242"
printf '%s\0' '/nix/store/opencode-same-build/bin/opencode' 'serve' >"$proc_root/4242/cmdline"
: >"$log"
"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-same-build' "$state" "$systemctl" "$proc_root"
if grep -Fxq -- '--user restart opencode.service' "$log"; then
  echo 'FAIL: first activation restarted the already-matching build' >&2
  cat "$log" >&2
  exit 1
fi
if [[ "$(cat "$state")" != '/nix/store/opencode-same-build' ]]; then
  echo 'FAIL: first activation did not record the matching live build' >&2
  exit 1
fi
echo 'ok - first activation recognizes the already-running build'

: >"$log"
"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-new-build' "$state" "$systemctl" "$proc_root"

if ! grep -Fxq -- '--user restart opencode.service' "$log"; then
  echo 'FAIL: changed OpenCode build did not restart the active service' >&2
  cat "$log" >&2
  exit 1
fi
if [[ "$(cat "$state")" != '/nix/store/opencode-new-build' ]]; then
  echo 'FAIL: changed OpenCode build key was not recorded' >&2
  exit 1
fi

echo 'ok - changed store path restarts and records the active service build'

# Socket handover: the port moves from the service to the socket unit before
# the service starts again, so the new process adopts the socket's fd 3.
rm -f "$SOCKET_ACTIVE"
: >"$log"
export SOCKET_ENABLED=0
"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-socket-build' "$state" "$systemctl" "$proc_root"
stop_line="$(grep -nFx -- '--user stop opencode.service' "$log" | cut -d: -f1 || true)"
socket_line="$(grep -nFx -- '--user start opencode.socket' "$log" | cut -d: -f1 || true)"
start_line="$(grep -nE -- '--user (start|restart) opencode\.service' "$log" | tail -n1 | cut -d: -f1 || true)"
if [[ -z $stop_line || -z $socket_line || -z $start_line ]] ||
  ((stop_line >= socket_line || socket_line >= start_line)); then
  echo 'FAIL: socket handover did not stop service, start socket, then start service' >&2
  cat "$log" >&2
  exit 1
fi
if [[ ! -f $SOCKET_ACTIVE ]]; then
  echo 'FAIL: socket was not started before the service' >&2
  exit 1
fi
echo 'ok - enabled socket takes the port before the service restarts'
unset SOCKET_ENABLED

# A degraded manager is still available for service control, even though
# systemctl reports a nonzero exit status for is-system-running.
: >"$log"
export SYSTEMD_STATE=degraded
"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-degraded-manager-build' "$state" "$systemctl" "$proc_root"
if ! grep -Fxq -- '--user restart opencode.service' "$log"; then
  echo 'FAIL: degraded manager did not deploy the changed build' >&2
  cat "$log" >&2
  exit 1
fi
if [[ "$(cat "$state")" != '/nix/store/opencode-degraded-manager-build' ]]; then
  echo 'FAIL: degraded-manager deployment key was not recorded' >&2
  exit 1
fi
echo 'ok - degraded manager deploys and records the changed build'

# An unavailable manager must not record the new key, so a later activation
# can retry the deployment.
: >"$log"
export SYSTEMD_STATE=offline
"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-offline-manager-build' "$state" "$systemctl" "$proc_root"
if [[ "$(cat "$state")" != '/nix/store/opencode-degraded-manager-build' ]]; then
  echo 'FAIL: unavailable manager recorded a deployment key' >&2
  exit 1
fi
if grep -Fxq -- '--user restart opencode.service' "$log"; then
  echo 'FAIL: unavailable manager attempted a service restart' >&2
  cat "$log" >&2
  exit 1
fi
echo 'ok - unavailable manager skips without recording the build'
