#!/usr/bin/env bash
# sign.sh — sign an unsigned build of the PM console APK with the key on
# this machine, and check the result carries the certificate the site
# lists. For when the build ran somewhere that does not hold the key.
#
# Usage: bash sign.sh <unsigned.apk> [<signed.apk>]
#
# Inputs (environment):
#   PM_ANDROID_KEYSTORE                PKCS#12 keystore, key alias pmconsole
#                                      (default ~/.local/state/pm-console/android/release.p12)
#   PM_ANDROID_KEYSTORE_PASSWORD_FILE  file holding its password
#                                      (default: keystore-password beside the keystore)
# Needs apksigner on PATH (on NixOS: nix shell nixpkgs#apksigner).
#
# The password is read by apksigner from the file. It is never an
# argument, an environment value or output.
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: bash sign.sh <unsigned.apk> [<signed.apk>]" >&2
  exit 2
fi
here="$(cd "$(dirname "$0")" && pwd)"
in="$1"
out="${2:-$(dirname "$in")/pm-console.apk}"
keystore="${PM_ANDROID_KEYSTORE:-$HOME/.local/state/pm-console/android/release.p12}"
password_file="${PM_ANDROID_KEYSTORE_PASSWORD_FILE:-$(dirname "$keystore")/keystore-password}"

for f in "$in" "$keystore" "$password_file"; do
  if [ ! -f "$f" ]; then
    echo "sign.sh: not found: $f" >&2
    exit 1
  fi
done
if ! command -v apksigner >/dev/null 2>&1; then
  echo "sign.sh: apksigner is not on PATH (on NixOS: nix shell nixpkgs#apksigner)" >&2
  exit 1
fi

apksigner sign --ks "$keystore" --ks-type pkcs12 --ks-key-alias pmconsole \
  --ks-pass "file:$password_file" --key-pass "file:$password_file" \
  --out "$out" "$in"

got="$(apksigner verify --print-certs "$out" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | tr 'a-f' 'A-F' | sed 's/../&:/g; s/:$//')"
want="$(tr -d '[:space:]' <"$here/signing-fingerprint.txt")"
if [ "$got" != "$want" ]; then
  rm -f "$out"
  echo "sign.sh: signed by $got, not by the certificate the site lists ($want)" >&2
  exit 1
fi
echo "sign.sh: $out signed by $got"
