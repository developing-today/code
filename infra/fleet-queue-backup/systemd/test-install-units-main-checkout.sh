#!/usr/bin/env bash
# Verify the installer accepts a main checkout and installs units that
# point at it. Oracle: a fresh `git init` checkout (a main worktree by
# construction) installs with exit 0, the export service bakes in the
# checkout's path, and systemctl is invoked; nothing outside the
# sandbox HOME is touched.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
checkout="$tmp/main checkout"
home="$tmp/home"
bin="$tmp/bin"
mkdir -p "$home" "$bin"
trap 'rm -rf "$tmp"' EXIT

git init --quiet "$checkout"
mkdir -p "$checkout/infra/fleet-queue-backup/systemd"
cp "$here"/fleet-queue-backup-*.service "$here"/fleet-queue-backup-*.timer \
  "$here"/install-units.sh "$checkout/infra/fleet-queue-backup/systemd"
cat >"$bin/systemctl" <<'EOF'
#!/usr/bin/env bash
touch "$HOME/systemctl-was-called"
EOF
chmod +x "$bin/systemctl"

set +e
output="$(HOME="$home" PATH="$bin:$PATH" bash "$checkout/infra/fleet-queue-backup/systemd/install-units.sh" 2>&1)"
status=$?
set -e
unit="$home/.config/systemd/user/fleet-queue-backup-export.service"
if [[ $status -ne 0 ]]; then
  printf 'FAIL: main checkout was refused (status=%s)\n%s\n' \
    "$status" "$output" >&2
  exit 1
fi
if [[ ! -e $unit || ! -e "$home/systemctl-was-called" ]]; then
  printf 'FAIL: units were not installed from the main checkout (status=%s)\n%s\n' \
    "$status" "$output" >&2
  exit 1
fi
if ! grep -qF "$checkout/infra/fleet-queue-backup/systemd/run-export.sh" "$unit"; then
  printf 'FAIL: installed unit does not point at the main checkout\n' >&2
  cat "$unit" >&2
  exit 1
fi
echo 'test-install-units-main-checkout.sh: main checkout accepted and installed'
