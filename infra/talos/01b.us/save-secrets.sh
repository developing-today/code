#!/usr/bin/env bash
set -Eeuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

# Encrypt to temp files in this directory and atomically rename over
# secrets.enc only after sops exits 0 with non-empty output, so a failed
# encryption leaves the previous secrets.enc byte-identical (#6554).
plaintext_tmp="$(mktemp secrets.tmp.XXXXXX)"
cipher_tmp="$(mktemp secrets.enc.tmp.XXXXXX)"
cleanup() {
  rm -f "$plaintext_tmp" "$cipher_tmp" secrets.tmp
}
trap cleanup EXIT

tar czf - ./secrets | base64 >"$plaintext_tmp"
sops --encrypt "$plaintext_tmp" >"$cipher_tmp"
if [ ! -s "$cipher_tmp" ]; then
  echo "save-secrets: sops produced empty output; leaving secrets.enc untouched" >&2
  exit 1
fi
mv -- "$cipher_tmp" secrets.enc
