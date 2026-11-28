#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

if [ -n "${SKIP_INIT:-}" ]; then
  echo "skipping tf init"
  exit 0
fi

echo "\$0=$0"
script_name="$0"
while [[ $script_name == -* ]]; do
  script_name="${script_name#-}"
done

dir="$(dirname -- "$(readlink -f -- "$script_name")")"
echo "dir: $dir"

echo "loading tfstate"
"$dir/load.sh"
echo "successfully loaded tfstate"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
touch "$dir/.lock"
function cleanup() {
  echo "cleaning up"
  echo "deleting lock file"
  rm -f "$dir/.lock"
  # echo "saving tfstate"
  # "$dir/save.sh"
  # echo "successfully saved tfstate"
  echo "done cleaning up"
}
trap cleanup EXIT

# --- tfstate guard (code#1943) ---------------------------------------------
# load.sh decrypts the state of record to $dir/terraform.tfstate. That path
# is both the local backend's state path and the working directory's DEFAULT
# state path. When the backend has never been initialized in this directory
# (no .terraform/terraform.tfstate record: fresh checkout/worktree, or after
# cleanup_terraform.sh removed .terraform while the decrypted state file
# stayed), `tofu init` treats that file as a legacy pre-backend state to
# migrate: it moves it to terraform.tfstate.backup and initializes the
# backend with an EMPTY state, so the next plan runs against a void
# (observed: "151 to add, 0 to destroy" instead of the real diff).
# Snapshot the decrypted state before init; afterwards restore it if init
# replaced it, and fail loudly if it is not intact byte-for-byte.
state_file="$dir/terraform.tfstate"
state_guard="$dir/terraform.tfstate.loaded"
if [ ! -s "$state_file" ]; then
  echo "Error: $state_file is missing or empty after load; refusing to init against a void state" >&2
  exit 1
fi
rm -f "$state_guard"
cp "$state_file" "$state_guard"
state_sha_before="$(sha256sum "$state_file" | awk '{print $1}')"
echo "snapshotted decrypted tfstate (sha256: $state_sha_before)"

echo "initializing tofu..."
tofu -chdir="$dir" init -backend-config="path=$dir/terraform.tfstate" "$@"
echo "tofu initialized."

if [ ! -s "$state_file" ] || [ "$(sha256sum "$state_file" | awk '{print $1}')" != "$state_sha_before" ]; then
  echo "WARNING: tofu init replaced the loaded tfstate (backend state migration); restoring the decrypted state of record" >&2
  cp "$state_guard" "$state_file"
fi
if [ ! -s "$state_file" ] || [ "$(sha256sum "$state_file" | awk '{print $1}')" != "$state_sha_before" ]; then
  echo "Error: tfstate at the backend path does not match the decrypted terraform.tfstate.enc after init; refusing to continue against a mismatched state" >&2
  exit 1
fi
rm -f "$state_guard"
echo "verified tfstate intact after init (sha256: $state_sha_before)"
