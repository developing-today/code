#!/usr/bin/env bash

sops_encrypt() {
  if [ $# -ne 1 ]; then
    echo "Usage: <command> | sops_encrypt <output_file>"
    return 1
  fi

  local output_file="$1"

  if ! command -v sops &>/dev/null; then
    echo "Error: SOPS is not installed or not in PATH."
    return 1
  fi

  # Encrypt to a temp file in the target's directory and rename it over the
  # output only after sops exits 0 with non-empty output (#6555). Writing
  # with `>"$output_file"` truncated the previous ciphertext before sops
  # ran, so a failed encryption destroyed it — and the unconditional
  # success message below still reported success with status 0. The
  # temp-then-rename shape follows lib/bws-to-sops.sh (mktemp beside the
  # target, mv only on success).
  local output_dir tmp_file
  output_dir="$(dirname "$output_file")"
  if ! tmp_file="$(mktemp "$output_dir/.sops_encrypt.XXXXXX")"; then
    echo "Error: could not create a temp file in $output_dir" >&2
    return 1
  fi

  if ! sops --encrypt <(cat -) >"$tmp_file" || [ ! -s "$tmp_file" ]; then
    rm -f "$tmp_file"
    echo "Error: SOPS encryption failed; $output_file left unchanged." >&2
    return 1
  fi

  if ! mv "$tmp_file" "$output_file"; then
    rm -f "$tmp_file"
    echo "Error: could not replace $output_file with the encrypted output" >&2
    return 1
  fi

  echo "Encrypted output saved to $output_file"
}

if [[ ${BASH_SOURCE[0]} != "${0}" ]]; then
  echo "SOPS encryption function 'sops_encrypt' has been loaded."
  echo "Usage: <command> | sops_encrypt <output_file>"
else
  if [ $# -ne 1 ]; then
    echo "Usage: $0 <output_file>"
    exit 1
  fi
  sops_encrypt "$1"
fi
