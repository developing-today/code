#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

if [ -n "${SKIP_SAVE:-}" ]; then
  echo "skipping tfstate save"
  exit 0
fi

echo "\$0=$0"
script_name="$0"
while [[ $script_name == -* ]]; do
  script_name="${script_name#-}"
done

dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
touch "$dir/.lock"
tmp_enc=""
function cleanup() {
  echo "cleaning up"
  echo "deleting lock file"
  rm -f "$dir/.lock"
  if [ -n "$tmp_enc" ]; then
    echo "deleting temporary encrypted state file"
    rm -f "$tmp_enc"
  fi
  # echo "saving tfstate"
  # "$dir/save.sh"
  # echo "successfully saved tfstate"
  echo "done cleaning up"
}
trap cleanup EXIT

if [ ! -f "$dir/terraform.tfstate" ] || [ ! -s "$dir/terraform.tfstate" ]; then
  echo "Error: terraform.tfstate does not exist or is empty" >&2
  exit 1
fi
echo "terraform.tfstate exists and is not empty, continuing... (dir=$dir)"

# If terraform.tfstate.enc already decrypts to exactly this state, the
# encrypted state of record is already current: skip re-encryption so
# read-only runs (e.g. plan) leave terraform.tfstate.enc byte-identical
# instead of dirtying it with a fresh sops ciphertext of the same plaintext.
if [ -f "$dir/terraform.tfstate.enc" ] && sops -d "$dir/terraform.tfstate.enc" 2>/dev/null | cmp -s - "$dir/terraform.tfstate"; then
  echo "terraform.tfstate.enc already contains this exact state; skipping re-encryption"
  exit 0
fi

echo "Encrypting with sops: terraform.tfstate -> terraform.tfstate.enc (dir=$dir)"
# Encrypt to a temporary file in the same directory and atomically rename
# it over terraform.tfstate.enc only after the ciphertext is verified.
# Writing directly (sops -e ... > terraform.tfstate.enc) truncates the
# state of record before sops runs, so any encryption failure — missing
# or expired sops key, sops error, disk full, interruption — would leave
# the only encrypted copy empty or partial (#6536). A failed run here
# leaves the previous terraform.tfstate.enc byte-identical; the EXIT
# trap removes the temp file and the lock, exactly as before.
tmp_enc="$(mktemp "$dir/.terraform.tfstate.enc.tmp.XXXXXX")"
sops -e "$dir/terraform.tfstate" >"$tmp_enc"
if [ ! -s "$tmp_enc" ]; then
  echo "Error: sops encryption produced an empty file; leaving terraform.tfstate.enc untouched" >&2
  exit 1
fi
# Post-write sanity check on the temp file, before it becomes the record:
# the new ciphertext must decrypt back to exactly this plaintext state
# (which also proves its lineage matches, mirroring plan.sh's check).
if ! sops -d "$tmp_enc" 2>/dev/null | cmp -s - "$dir/terraform.tfstate"; then
  echo "Error: encrypted temp file does not decrypt to terraform.tfstate; leaving terraform.tfstate.enc untouched" >&2
  exit 1
fi
if [ -f "$dir/terraform.tfstate.enc" ]; then
  chmod --reference="$dir/terraform.tfstate.enc" "$tmp_enc" || true
fi
mv -f "$tmp_enc" "$dir/terraform.tfstate.enc"
tmp_enc=""
echo "Encrypted with sops: terraform.tfstate -> terraform.tfstate.enc (dir=$dir)"
