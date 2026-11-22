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
if [[ "$*" == '--user show --property=MainPID --value opencode.service' ]]; then
  printf '4242\n'
fi
EOF
chmod +x "$systemctl"
export SYSTEMCTL_LOG="$log"

printf '%s\n' '/nix/store/opencode-same-build' >"$state"

"$repo_root/home/common/opencode-deploy.sh" \
  '/nix/store/opencode-same-build' "$state" "$systemctl" "$proc_root"

if [[ -e "$log" ]]; then
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
