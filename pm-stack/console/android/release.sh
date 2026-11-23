#!/usr/bin/env bash
# release.sh — publish a signed build of the PM console APK.
#
# Usage: bash release.sh <signed pm-console.apk>
#
# Inputs (environment):
#   VERSION_NAME, VERSION_CODE   the build's version (build.sh writes both)
#   GITHUB_SHA, GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_SERVER_URL
#                                as a workflow run has them
# Needs gh, signed in with leave to write releases (GH_TOKEN), and python3.
#
# Two releases come of it:
#   pm-console-android-v<code>  this build. Once made it is never changed.
#   pm-console-android          one address that always holds the newest
#                               build; the site hands out
#                               releases/download/pm-console-android/pm-console.apk
# Each carries pm-console.apk, pm-console.apk.sha256 and version.json
# (name, code, commit, sha256, size, when, and the signing certificate).
# Neither is marked as the repository's latest release: other things in
# this repository have releases of their own.
#
# Running it again for a build already released changes nothing in the
# numbered release; the fixed address is brought up to that release if
# it was behind, and is never taken back to an older build.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: bash release.sh <signed pm-console.apk>" >&2
  exit 2
fi
here="$(cd "$(dirname "$0")" && pwd)"
apk="$1"
name="${VERSION_NAME:-}"
code="${VERSION_CODE:-}"
sha="${GITHUB_SHA:-}"
repo="${GITHUB_REPOSITORY:-}"

[ -f "$apk" ] || {
  echo "release.sh: not found: $apk" >&2
  exit 1
}
if ! printf '%s' "$code" | grep -Eq '^[1-9][0-9]{0,9}$'; then
  echo "release.sh: VERSION_CODE must be a whole number" >&2
  exit 2
fi
if ! printf '%s' "$name" | grep -Eq '^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$'; then
  echo "release.sh: VERSION_NAME is not a version name" >&2
  exit 2
fi
if ! printf '%s' "$sha" | grep -Eq '^[0-9a-f]{40}$'; then
  echo "release.sh: GITHUB_SHA must be a full commit id" >&2
  exit 2
fi
if ! printf '%s' "$repo" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'; then
  echo "release.sh: GITHUB_REPOSITORY must be owner/name" >&2
  exit 2
fi

numbered="pm-console-android-v$code"
fixed="pm-console-android"
run_url="${GITHUB_SERVER_URL:-https://github.com}/$repo/actions/runs/${GITHUB_RUN_ID:-0}"
certificate="$(tr -d '[:space:]' <"$here/signing-fingerprint.txt")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/new" "$work/was"

exists() {
  gh release view "$1" --repo "$repo" >/dev/null 2>&1
}

# The code a release's version.json gives, or 0 if it has none.
code_of() {
  rm -rf "$work/was" && mkdir "$work/was"
  if ! gh release download "$1" --repo "$repo" --dir "$work/was" --pattern version.json --clobber >/dev/null 2>&1; then
    echo 0
    return
  fi
  python3 -c 'import json, sys
try:
    code = json.load(open(sys.argv[1]))["code"]
    print(code if isinstance(code, int) and code > 0 else 0)
except Exception:
    print(0)' "$work/was/version.json"
}

if exists "$numbered"; then
  # Released before (the run was started again). What was released then
  # is the build of this version; this run's own file is set aside.
  gh release download "$numbered" --repo "$repo" --dir "$work/new" \
    --pattern pm-console.apk --pattern pm-console.apk.sha256 --pattern version.json --clobber
  for f in pm-console.apk pm-console.apk.sha256 version.json; do
    [ -f "$work/new/$f" ] || {
      echo "release.sh: release $numbered exists without $f; not touching it" >&2
      exit 1
    }
  done
  # Its own account of itself is what the fixed address is told.
  name="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["name"])' "$work/new/version.json")"
  echo "release.sh: $numbered was released before; left as it is"
else
  cp "$apk" "$work/new/pm-console.apk"
  digest="$(sha256sum "$work/new/pm-console.apk" | cut -d' ' -f1)"
  printf '%s  pm-console.apk\n' "$digest" >"$work/new/pm-console.apk.sha256"
  python3 -c 'import json, os, sys, time
name, code, commit, digest, certificate, path, out = sys.argv[1:8]
json.dump(
    {
        "name": name,
        "code": int(code),
        "commit": commit,
        "sha256": digest,
        "size": os.path.getsize(path),
        "built": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "certificate": certificate,
    },
    open(out, "w"),
    indent=2,
)
open(out, "a").write("\n")' "$name" "$code" "$sha" "$digest" "$certificate" "$work/new/pm-console.apk" "$work/new/version.json"
  gh release create "$numbered" --repo "$repo" --target "$sha" --latest=false \
    --title "PM console for Android $name" \
    --notes "The PM console as an Android app, build $name (version code $code), from commit $sha.

Built, signed and released by $run_url. Signed by the certificate $certificate, the one the site lists, so it installs over earlier builds.

The newest build is always at https://github.com/$repo/releases/download/$fixed/pm-console.apk" \
    "$work/new/pm-console.apk" "$work/new/pm-console.apk.sha256" "$work/new/version.json"
  echo "release.sh: released $numbered"
fi

notes="The PM console as an Android app: always the newest build, now $name (version code $code). It opens https://pm.security.cab/console/ and signs in the way the site does. Install pm-console.apk; each build installs over the one before.

Every build is also kept under its own release, pm-console-android-v<code>. This one: https://github.com/$repo/releases/tag/$numbered"

if exists "$fixed"; then
  was="$(code_of "$fixed")"
  if [ "$was" -gt "$code" ]; then
    echo "release.sh: $fixed already holds a newer build (code $was); left as it is"
    exit 0
  fi
  # The build first and its description last: someone reading in between
  # finds a description that does not match the file, and waits.
  gh release upload "$fixed" --repo "$repo" --clobber "$work/new/pm-console.apk" "$work/new/pm-console.apk.sha256"
  gh release upload "$fixed" --repo "$repo" --clobber "$work/new/version.json"
  gh release edit "$fixed" --repo "$repo" --draft=false --latest=false --title "PM console for Android" --notes "$notes"
  # The tag follows the build it now holds. If it cannot be moved the
  # release still holds the right files, which is what is read.
  gh api -X PATCH "repos/$repo/git/refs/tags/$fixed" -f sha="$sha" -F force=true >/dev/null 2>&1 ||
    echo "release.sh: could not move the tag $fixed to $sha; the release holds the new build all the same"
else
  gh release create "$fixed" --repo "$repo" --target "$sha" --latest=false \
    --title "PM console for Android" --notes "$notes" \
    "$work/new/pm-console.apk" "$work/new/pm-console.apk.sha256" "$work/new/version.json"
fi
echo "release.sh: $fixed holds $name (code $code)"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "Released $name (code $code) as \`$numbered\`; \`$fixed\` holds it." >>"$GITHUB_STEP_SUMMARY"
fi
