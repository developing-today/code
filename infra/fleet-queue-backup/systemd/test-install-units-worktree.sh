#!/usr/bin/env bash
# Verify the installer refuses a linked Git worktree before touching user units.
# Oracle: the linked checkout is rejected, with no installed files or systemctl call.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(git -C "$here" rev-parse --show-toplevel)"
tmp="$(mktemp -d)"
checkout="$tmp/worktree"
home="$tmp/home"
bin="$tmp/bin"
mkdir -p "$home" "$bin"
trap 'git -C "$repo" worktree remove --force "$checkout" >/dev/null 2>&1 || true; rm -rf "$tmp"' EXIT

git -C "$repo" worktree add --quiet --detach "$checkout" HEAD
cat >"$bin/systemctl" <<'EOF'
#!/usr/bin/env bash
touch "$HOME/systemctl-was-called"
EOF
chmod +x "$bin/systemctl"

set +e
output="$(HOME="$home" PATH="$bin:$PATH" bash "$checkout/infra/fleet-queue-backup/systemd/install-units.sh" 2>&1)"
status=$?
set -e
if [[ $status -eq 0 || $output != *worktree* || -e "$home/.config/systemd/user/fleet-queue-backup-export.service" || -e "$home/systemctl-was-called" ]]; then
  printf 'FAIL: linked worktree was not refused before installation (status=%s)\n%s\n' \
    "$status" "$output" >&2
  exit 1
fi
echo 'test-install-units-worktree.sh: linked worktree rejected before installation'
