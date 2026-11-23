#!/usr/bin/env bash
# release-built.sh — sign the newest build GitHub made of the app, on
# this machine, and release it.
#
# GitHub builds the app on every change to it (the pm-console android
# workflow) but does not hold the app's signing key, so what it builds
# is unsigned and nothing is released there. The key is on this machine
# and stays on it. pm-console-android-release.timer runs this script,
# which takes the newest unsigned build made on main, signs it here
# (sign.sh), releases it (release.sh) and puts it where the console
# serves the app from. Nothing of the key goes anywhere: what leaves is
# the signed APK, which is public.
#
# When the newest build is already released it asks GitHub one question
# and does nothing. If GitHub's environment is one day given the key,
# the workflow signs and releases by itself and this finds nothing to do.
#
# Usage: bash release-built.sh
#
# Inputs (environment), all optional:
#   PM_GH                    how gh is run here (default "fleet gh" where
#                            fleet is installed, else "gh")
#   PM_ANDROID_REPO          owner/name (default developing-today/code)
#   PM_CONSOLE_ANDROID_APK   where the console serves the app from
#                            (default ~/.local/state/pm-console/android/pm-console.apk)
#   PM_ANDROID_KEYSTORE, PM_ANDROID_KEYSTORE_PASSWORD_FILE
#                            as sign.sh reads them
# Needs python3, flock, and apksigner; where apksigner is not on PATH it
# is run through nix.
set -euo pipefail

if [ "$#" -ne 0 ]; then
  echo "usage: bash release-built.sh   (it takes no arguments)" >&2
  exit 2
fi
here="$(cd "$(dirname "$0")" && pwd)"
repo="${PM_ANDROID_REPO:-developing-today/code}"
serve="${PM_CONSOLE_ANDROID_APK:-$HOME/.local/state/pm-console/android/pm-console.apk}"
state="$(dirname "$serve")"
workflow="pm-console-android.yml"

if ! printf '%s' "$repo" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'; then
  echo "release-built: PM_ANDROID_REPO must be owner/name" >&2
  exit 2
fi
if [ -z "${PM_GH:-}" ]; then
  if command -v fleet >/dev/null 2>&1; then PM_GH="fleet gh"; else PM_GH="gh"; fi
fi
export PM_GH
read -r -a gh_cmd <<<"$PM_GH"
# "command": the program of that name, not this function, which is
# also called gh (PM_GH may be plain "gh").
gh() { command "${gh_cmd[@]}" "$@"; }

mkdir -p "$state"
# One at a time: a second start while the first is still signing leaves
# it to the first.
exec 9>"$state/release.lock"
if ! flock -n 9; then
  echo "release-built: another run is at work; left to it"
  exit 0
fi

# The newest build that is this repository's own, made on main by a push
# or by hand. A run of a pull request, of another branch or of a fork is
# never signed, whatever it is called.
runs="$(gh api "repos/$repo/actions/workflows/$workflow/runs?branch=main&status=success&per_page=30")"
newest="$(printf '%s' "$runs" | python3 -c 'import json, sys
repo = sys.argv[1]
runs = json.load(sys.stdin).get("workflow_runs", [])
mine = [
    r
    for r in runs
    if r.get("conclusion") == "success"
    and r.get("head_branch") == "main"
    and r.get("event") in ("push", "workflow_dispatch")
    and (r.get("head_repository") or {}).get("full_name") == repo
    and (r.get("repository") or {}).get("full_name") == repo
    and isinstance(r.get("id"), int)
    and isinstance(r.get("head_sha"), str)
]
mine.sort(key=lambda r: r["id"], reverse=True)
if mine:
    print(mine[0]["id"], mine[0]["head_sha"])' "$repo")"
if [ -z "$newest" ]; then
  echo "release-built: GitHub has no build of the app made on main yet"
  exit 0
fi
run="${newest%% *}"
sha="${newest##* }"
if ! printf '%s' "$sha" | grep -Eq '^[0-9a-f]{40}$'; then
  echo "release-built: run $run names no commit" >&2
  exit 1
fi
if [ "$(cat "$state/released-run" 2>/dev/null || true)" = "$run" ]; then
  exit 0
fi

artifacts="$(gh api "repos/$repo/actions/runs/$run/artifacts?per_page=100" | python3 -c 'import json, sys
print(" ".join(a["name"] for a in json.load(sys.stdin).get("artifacts", []) if not a.get("expired")))')"
case " $artifacts " in
*" pm-console-apk "*)
  # GitHub held the key for this one: it signed and released it itself.
  printf '%s\n' "$run" >"$state/released-run"
  echo "release-built: run $run was signed and released by GitHub; nothing to do here"
  exit 0
  ;;
*" pm-console-apk-unsigned "*) ;;
*)
  echo "release-built: run $run kept no build to sign (its artifacts: ${artifacts:-none}); waiting for the next build"
  exit 0
  ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/in"
gh run download "$run" --repo "$repo" --name pm-console-apk-unsigned --dir "$work/in"
mapfile -t apks < <(find "$work/in" -type f -name '*.apk')
if [ "${#apks[@]}" -ne 1 ]; then
  echo "release-built: run $run holds ${#apks[@]} APKs, not one; nothing is signed" >&2
  exit 1
fi
version="$(find "$work/in" -type f -name version | head -n 1)"
if [ -z "$version" ]; then
  # A build from before builds carried their version. It will never
  # carry one, so this is not a failure to repeat every ten minutes.
  echo "release-built: run $run does not say which version it built; nothing is signed, waiting for the next build"
  exit 0
fi
name="$(sed -n 1p "$version")"
code="$(sed -n 2p "$version")"

sign=(bash "$here/sign.sh" "${apks[0]}" "$work/pm-console.apk")
if command -v apksigner >/dev/null 2>&1; then
  "${sign[@]}"
else
  nix --extra-experimental-features "nix-command flakes" shell nixpkgs#apksigner -c "${sign[@]}"
fi

VERSION_NAME="$name" VERSION_CODE="$code" GITHUB_SHA="$sha" GITHUB_REPOSITORY="$repo" GITHUB_RUN_ID="$run" \
  PM_SIGNED_HERE=1 bash "$here/release.sh" "$work/pm-console.apk"

# Released: the console serves the same file, and this run is done with.
install -D -m 644 "$work/pm-console.apk" "$serve.new"
mv -f "$serve.new" "$serve"
printf '%s\n' "$run" >"$state/released-run"
echo "release-built: $name (code $code), built by run $run, is signed, released and served"
