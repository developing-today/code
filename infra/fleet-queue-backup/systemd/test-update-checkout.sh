#!/usr/bin/env bash
# Verify update-checkout.sh brings a service-owned checkout to
# origin/main and refuses a dirty one. Oracle: against a local bare
# remote, a clone one commit behind is advanced to the remote's tip
# (exit 0, HEAD == tip, new file content present); with an uncommitted
# change in the tree the script exits non-zero and HEAD does not move.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
remote="$tmp/remote.git"
seed="$tmp/seed"
checkout="$tmp/checkout"

git init --quiet --bare "$remote"
git -C "$remote" symbolic-ref HEAD refs/heads/main
git init --quiet -b main "$seed"
(
  cd "$seed"
  git config user.email test@example.invalid
  git config user.name test
  mkdir -p infra/fleet-queue-backup/systemd
  cp "$here/update-checkout.sh" infra/fleet-queue-backup/systemd/
  echo v1 >infra/fleet-queue-backup/version.txt
  git add -A
  git commit --quiet -m v1
  git push --quiet "$remote" main
)
git clone --quiet "$remote" "$checkout"

# Advance the remote by one commit after the clone was made.
(
  cd "$seed"
  echo v2 >infra/fleet-queue-backup/version.txt
  git commit --quiet -am v2
  git push --quiet "$remote" main
)
tip="$(git -C "$seed" rev-parse HEAD)"

set +e
output="$(bash "$checkout/infra/fleet-queue-backup/systemd/update-checkout.sh" 2>&1)"
status=$?
set -e
if [[ $status -ne 0 ]]; then
  printf 'FAIL: update of a clean behind-checkout exited %s\n%s\n' "$status" "$output" >&2
  exit 1
fi
if [[ "$(git -C "$checkout" rev-parse HEAD)" != "$tip" ]]; then
  printf 'FAIL: checkout did not advance to the remote tip\n%s\n' "$output" >&2
  exit 1
fi
if [[ "$(cat "$checkout/infra/fleet-queue-backup/version.txt")" != "v2" ]]; then
  printf 'FAIL: checkout content is not v2 after the update\n' >&2
  exit 1
fi

# Advance the remote again, then dirty the checkout: the update must
# refuse and HEAD must stay where it was.
(
  cd "$seed"
  echo v3 >infra/fleet-queue-backup/version.txt
  git commit --quiet -am v3
  git push --quiet "$remote" main
)
echo dirty >>"$checkout/infra/fleet-queue-backup/version.txt"
set +e
output="$(bash "$checkout/infra/fleet-queue-backup/systemd/update-checkout.sh" 2>&1)"
status=$?
set -e
if [[ $status -eq 0 ]]; then
  printf 'FAIL: a dirty checkout was updated instead of refused\n%s\n' "$output" >&2
  exit 1
fi
if [[ "$(git -C "$checkout" rev-parse HEAD)" != "$tip" ]]; then
  printf 'FAIL: a dirty checkout moved despite the refusal\n' >&2
  exit 1
fi
echo 'test-update-checkout.sh: checkout follows origin/main and refuses a dirty tree'
