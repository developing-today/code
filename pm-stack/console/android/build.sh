#!/usr/bin/env bash
# build.sh — build the PM console APK, signed when a keystore is given.
#
# Inputs (environment):
#   KEYSTORE_B64                  base64 of the PKCS#12 signing keystore (key alias pmconsole)
#   PM_ANDROID_KEYSTORE_PASSWORD  its password
#   GRADLE_VERSION                Gradle to fetch (default 8.9; AGP 8.5 needs 8.7 or later)
# Needs: a JDK 17 or 21, an Android SDK at ANDROID_HOME, curl, unzip.
#
# With both keystore variables set the result is app/build/outputs/apk/release/pm-console.apk,
# and the build fails unless its signing certificate is the one in
# signing-fingerprint.txt: the site lists that fingerprint in its
# asset-links file, and an APK signed by anything else would open with a
# browser bar. Without them the result is an unsigned APK, for a check
# that the project builds.
set -euo pipefail

cd "$(dirname "$0")"
GRADLE_VERSION="${GRADLE_VERSION:-8.9}"
OUT=app/build/outputs/apk/release

export JAVA_HOME="${JAVA_HOME_17_X64:-${JAVA_HOME_21_X64:-${JAVA_HOME:-}}}"
if [ -z "$JAVA_HOME" ] || [ -z "${ANDROID_HOME:-}" ]; then
  echo "build.sh: JAVA_HOME and ANDROID_HOME must be set" >&2
  exit 2
fi

if [ ! -x "gradle-dist/gradle-$GRADLE_VERSION/bin/gradle" ]; then
  mkdir -p gradle-dist
  zip="gradle-dist/gradle-$GRADLE_VERSION-bin.zip"
  curl -fsSL -o "$zip" "https://services.gradle.org/distributions/gradle-$GRADLE_VERSION-bin.zip"
  want="$(curl -fsSL "https://services.gradle.org/distributions/gradle-$GRADLE_VERSION-bin.zip.sha256")"
  echo "$want  $zip" | sha256sum -c - >/dev/null
  unzip -q -o "$zip" -d gradle-dist
fi
GRADLE="gradle-dist/gradle-$GRADLE_VERSION/bin/gradle"

if [ -n "${KEYSTORE_B64:-}" ] && [ -n "${PM_ANDROID_KEYSTORE_PASSWORD:-}" ]; then
  keystore="$(mktemp --suffix=.p12)"
  trap 'rm -f "$keystore"' EXIT
  printf '%s' "$KEYSTORE_B64" | base64 -d >"$keystore"
  export PM_ANDROID_KEYSTORE="$keystore"
fi

"$GRADLE" --no-daemon --console=plain assembleRelease

if [ -z "${PM_ANDROID_KEYSTORE:-}" ]; then
  echo "build.sh: built unsigned: $(ls $OUT/*.apk)"
  exit 0
fi

mv "$OUT/app-release.apk" "$OUT/pm-console.apk"
apksigner="$(find "$ANDROID_HOME/build-tools" -name apksigner -type f | sort -V | tail -1)"
got="$("$apksigner" verify --print-certs "$OUT/pm-console.apk" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | tr 'a-f' 'A-F' | sed 's/../&:/g; s/:$//')"
want="$(tr -d '[:space:]' <signing-fingerprint.txt)"
if [ "$got" != "$want" ]; then
  echo "build.sh: the APK is signed by $got, not by the certificate the site lists ($want)" >&2
  exit 1
fi
echo "build.sh: $OUT/pm-console.apk signed by $got"
