#!/usr/bin/env bash
# release-built.test.sh — pins what release-built.sh does on the machine
# that holds the key: which of GitHub's builds it signs, that nothing is
# released or served unless it carries the right certificate, that a
# build is handled once, and that every failure leaves things as they
# were so the next run tries again.
#
# The scripts under test are the real release-built.sh, sign.sh and
# release.sh. Stand-ins: gh keeps releases in a directory and answers
# the two Actions questions from files; apksigner "signs" by copying and
# reports the certificate a file names; fleet passes gh through. Whether
# apksigner signs correctly is apksigner's business, and sign.sh's check
# of the certificate is what is relied on here.
set -euo pipefail

if [ "$#" -gt 0 ]; then
  echo "usage: release-built.test.sh (it takes no arguments)" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gh_state="$tmp/github"
home="$tmp/home"
mkdir -p "$tmp/bin" "$tmp/fleet-bin" "$gh_state" "$home"

# ---- stand-ins --------------------------------------------------------------
cat >"$tmp/bin/gh" <<'GH'
#!/usr/bin/env bash
# Releases live under $GH_STATE/<tag>/ (as in release.test.sh). The runs
# of the workflow are $GH_STATE/.runs.json; a run's artifacts are
# $GH_STATE/.artifacts-<id>.json and their files $GH_STATE/.files-<id>/.
set -euo pipefail
echo "$*" >>"$GH_STATE/.log"
if [ "$1" = "api" ]; then
  path="" sha=""
  for arg in "$@"; do
    case "$arg" in
    repos/*) path="$arg" ;;
    sha=*) sha="${arg#sha=}" ;;
    esac
  done
  case "$path" in
  repos/*/actions/workflows/pm-console-android.yml/runs\?*)
    [ -e "$GH_STATE/.github-is-down" ] && exit 1
    # What is asked for is part of what is pinned: only finished builds on main.
    case "$path" in *branch=main*status=success*) ;; *) exit 64 ;; esac
    cat "$GH_STATE/.runs.json"
    ;;
  repos/*/actions/runs/*/artifacts\?*)
    id="${path#repos/*/actions/runs/}"
    cat "$GH_STATE/.artifacts-${id%%/*}.json"
    ;;
  repos/*/git/refs/tags/*)
    mkdir -p "$GH_STATE/.tags"
    echo "$sha" >"$GH_STATE/.tags/${path##*/}"
    ;;
  *) exit 64 ;;
  esac
  exit 0
fi
if [ "$1" = "run" ] && [ "$2" = "download" ]; then
  id="$3" name="" out=""
  shift 3
  while [ "$#" -gt 0 ]; do
    case "$1" in
    --repo) shift ;;
    --name) name="$2" && shift ;;
    --dir) out="$2" && shift ;;
    *) exit 64 ;;
    esac
    shift
  done
  [ "$name" = "pm-console-apk-unsigned" ] || exit 64
  mkdir -p "$out"
  cp -r "$GH_STATE/.files-$id/." "$out/"
  exit 0
fi
[ "$1" = "release" ] || exit 64
verb="$2" tag="$3"
shift 3
dir="$GH_STATE/$tag"
files=() patterns=() out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
  --repo) shift ;;
  --target) target="$2" && shift ;;
  --title) title="$2" && shift ;;
  --notes) notes="$2" && shift ;;
  --dir) out="$2" && shift ;;
  --pattern) patterns+=("$2") && shift ;;
  --latest=false) latest=false ;;
  --draft=false) draft=false ;;
  --clobber) ;;
  --*) exit 64 ;;
  *) files+=("$1") ;;
  esac
  shift
done
case "$verb" in
view) [ -d "$dir" ] ;;
create)
  [ -e "$GH_STATE/.cannot-release" ] && exit 1
  [ ! -d "$dir" ] || exit 1
  mkdir -p "$dir/assets"
  echo false >"$dir/draft"
  echo "${target:-}" >"$dir/target"
  echo "${notes:-}" >"$dir/notes"
  cp "${files[@]}" "$dir/assets/"
  ;;
upload)
  [ -d "$dir" ] || exit 1
  cp "${files[@]}" "$dir/assets/"
  ;;
download)
  [ -d "$dir" ] || exit 1
  found=0
  for pattern in "${patterns[@]}"; do
    if [ -f "$dir/assets/$pattern" ]; then
      cp "$dir/assets/$pattern" "$out/"
      found=1
    fi
  done
  [ "$found" -eq 1 ]
  ;;
edit)
  [ -d "$dir" ] || exit 1
  [ -n "${notes:-}" ] && echo "$notes" >"$dir/notes"
  true
  ;;
*) exit 64 ;;
esac
GH

# fleet: only "fleet gh ..." is known, and it is gh.
cat >"$tmp/fleet-bin/fleet" <<'FLEET'
#!/usr/bin/env bash
set -euo pipefail
[ "$1" = "gh" ] || exit 64
shift
echo "$*" >>"$GH_STATE/.fleet-log"
exec gh "$@"
FLEET

# apksigner: "sign" copies the build behind a mark, having seen the key
# and its password file; "verify --print-certs" names the certificate in
# $APKSIGNER_CERT (lower-case hex, as apksigner prints it).
cat >"$tmp/bin/apksigner" <<'APKSIGNER'
#!/usr/bin/env bash
set -euo pipefail
verb="$1"
shift
case "$verb" in
sign)
  ks="" pass="" out="" in=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
    --ks) ks="$2" && shift ;;
    --ks-type | --ks-key-alias) shift ;;
    --ks-pass) pass="$2" && shift ;;
    --out) out="$2" && shift ;;
    --*) exit 64 ;;
    *) in="$1" ;;
    esac
    shift
  done
  [ -f "$ks" ] && [ -f "${pass#file:}" ] && [ "${pass#file:}" != "$pass" ] || exit 1
  echo "sign $in" >>"$APKSIGNER_LOG"
  {
    printf 'signed:'
    cat "$in"
  } >"$out"
  ;;
verify)
  [ "$1" = "--print-certs" ] || exit 64
  echo "Signer #1 certificate DN: CN=PM console"
  echo "Signer #1 certificate SHA-256 digest: $(cat "$APKSIGNER_CERT")"
  ;;
*) exit 64 ;;
esac
APKSIGNER
chmod +x "$tmp/bin/gh" "$tmp/bin/apksigner" "$tmp/fleet-bin/fleet"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
field() { python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
calls() { if [ -f "$gh_state/.log" ]; then wc -l <"$gh_state/.log"; else echo 0; fi; }
signed() { if [ -f "$tmp/apksigner.log" ]; then wc -l <"$tmp/apksigner.log"; else echo 0; fi; }

right_cert="$(tr -d '[:space:]:' <"$here/signing-fingerprint.txt" | tr 'A-F' 'a-f')"
echo "$right_cert" >"$tmp/cert"
printf 'a keystore' >"$tmp/key.p12"
printf 'its password\n' >"$tmp/key-password"
serve="$home/.local/state/pm-console/android/pm-console.apk"
state="$(dirname "$serve")"
fixed="$gh_state/pm-console-android"
sha_a="$(printf 'a%.0s' {1..40})"
sha_b="$(printf 'b%.0s' {1..40})"
sha_c="$(printf 'c%.0s' {1..40})"

export GH_STATE="$gh_state" APKSIGNER_LOG="$tmp/apksigner.log" APKSIGNER_CERT="$tmp/cert"
export HOME="$home" PM_ANDROID_REPO="example/code"
export PM_ANDROID_KEYSTORE="$tmp/key.p12" PM_ANDROID_KEYSTORE_PASSWORD_FILE="$tmp/key-password"
unset PM_GH PM_CONSOLE_ANDROID_APK GITHUB_STEP_SUMMARY GITHUB_SHA GITHUB_REPOSITORY GITHUB_RUN_ID VERSION_NAME VERSION_CODE
with_fleet="$tmp/fleet-bin:$tmp/bin:$PATH"
without_fleet="$tmp/bin:$PATH"
export PATH="$with_fleet"

run_json() { # run_json <id> <event> <branch> <head repo> <conclusion> <sha>
  printf '{"id": %s, "event": "%s", "head_branch": "%s", "head_repository": {"full_name": "%s"}, "repository": {"full_name": "example/code"}, "conclusion": "%s", "head_sha": "%s"}' "$@"
}
runs() { # runs <run json>...
  local joined="" r
  for r in "$@"; do joined="${joined:+$joined, }$r"; done
  printf '{"workflow_runs": [%s]}\n' "$joined" >"$gh_state/.runs.json"
}
build() { # build <run id> <artifact name or ""> <apk bytes> <name> <code>
  if [ -n "$2" ]; then
    printf '{"artifacts": [{"name": "%s", "expired": false}, {"name": "something-else", "expired": false}]}\n' "$2" >"$gh_state/.artifacts-$1.json"
  else
    printf '{"artifacts": [{"name": "pm-console-apk-unsigned", "expired": true}]}\n' >"$gh_state/.artifacts-$1.json"
  fi
  rm -rf "$gh_state/.files-$1"
  mkdir -p "$gh_state/.files-$1"
  printf '%s' "$3" >"$gh_state/.files-$1/app-release-unsigned.apk"
  printf '%s\n%s\n' "$4" "$5" >"$gh_state/.files-$1/version"
}
go() { bash "$here/release-built.sh" >"$tmp/out" 2>&1; }
refused() { # refused <why> : the run must fail and change nothing that was released, served or recorded
  local before_run before_apk before_fixed
  before_run="$(cat "$state/released-run" 2>/dev/null || true)"
  before_apk="$(cat "$serve" 2>/dev/null || true)"
  before_fixed="$(cat "$fixed/assets/version.json" 2>/dev/null || true)"
  if go; then fail "$1: it did not fail: $(cat "$tmp/out")"; fi
  [ "$(cat "$state/released-run" 2>/dev/null || true)" = "$before_run" ] || fail "$1: the run was recorded as done"
  [ "$(cat "$serve" 2>/dev/null || true)" = "$before_apk" ] || fail "$1: the served app changed"
  [ "$(cat "$fixed/assets/version.json" 2>/dev/null || true)" = "$before_fixed" ] || fail "$1: the release changed"
}

# 1. Of everything GitHub lists, only this repository's own finished
#    build on main is signed: not a run of another repository or of a
#    fork (whatever its event and branch are called), not a pull
#    request, not another branch, not a failed run, not a schedule.
runs \
  "$(run_json 920 push main example/code success "$sha_c" | sed 's|"repository": {"full_name": "example/code"}|"repository": {"full_name": "someone/else"}|')" \
  "$(run_json 910 workflow_dispatch main someone/fork success "$sha_c")" \
  "$(run_json 900 pull_request main someone/fork success "$sha_c")" \
  "$(run_json 890 push feature example/code success "$sha_c")" \
  "$(run_json 880 schedule main example/code success "$sha_c")" \
  "$(run_json 870 push main example/code failure "$sha_c")" \
  "$(run_json 850 push main example/code success "$sha_b")" \
  "$(run_json 860 workflow_dispatch main example/code success "$sha_a")"
for id in 920 910 900 890 880 870; do build "$id" pm-console-apk-unsigned "not ours $id" "0.0.0-bad" 99999999; done
build 850 pm-console-apk-unsigned "an older build" "2026.11.22-bbbbbbb" 21756300
build 860 pm-console-apk-unsigned "build 860" "2026.11.23-aaaaaaa" 21756400
go || fail "1: $(cat "$tmp/out")"
[ "$(cat "$state/released-run")" = 860 ] || fail "1: the run it handled is not written down"
[ "$(cat "$serve")" = "signed:build 860" ] || fail "1: the console is not serving the signed build"
[ "$(stat -c %a "$serve")" = 644 ] || fail "1: the served file is not readable by the console"
numbered="$gh_state/pm-console-android-v21756400"
[ "$(cat "$numbered/assets/pm-console.apk")" = "signed:build 860" ] || fail "1: the numbered release does not hold the signed build"
[ "$(cat "$fixed/assets/pm-console.apk")" = "signed:build 860" ] || fail "1: the fixed address does not hold the signed build"
[ "$(field "$fixed/assets/version.json" name)" = "2026.11.23-aaaaaaa" ] || fail "1: wrong name released"
[ "$(field "$fixed/assets/version.json" code)" = 21756400 ] || fail "1: wrong code released"
[ "$(field "$fixed/assets/version.json" commit)" = "$sha_a" ] || fail "1: wrong commit released"
[ "$(cat "$numbered/target")" = "$sha_a" ] || fail "1: the release is not of the commit that was built"
grep -q "actions/runs/860" "$numbered/notes" || fail "1: the release does not say which run built it"
grep -q "Signed and released on the machine that holds the app's key" "$numbered/notes" || fail "1: the release does not say where it was signed"
grep -q "^sign .*app-release-unsigned.apk$" "$tmp/apksigner.log" || fail "1: apksigner was not given the build"
[ "$(signed)" = 1 ] || fail "1: signed $(signed) times, not once"
grep -q "^run download 860 " "$gh_state/.log" || fail "1: the build was not the one fetched"
! grep -q "download 920\|download 910\|download 900\|download 890\|download 880\|download 870\|download 850" "$gh_state/.log" || fail "1: a build that is not ours, or not the newest, was fetched"
# With fleet installed, every word to GitHub goes through it, release.sh's too.
[ "$(wc -l <"$gh_state/.fleet-log")" = "$(calls)" ] || fail "1: some gh calls did not go through fleet"

# 2. The same build again: one question to GitHub, and nothing else.
before="$(calls)"
go || fail "2: $(cat "$tmp/out")"
[ "$(calls)" = "$((before + 1))" ] || fail "2: asked GitHub $(($(calls) - before)) things, not one"
[ "$(signed)" = 1 ] || fail "2: signed again"

# 3. A newer build that comes out signed by another certificate is not
#    released, not served and not written down; once the certificate is
#    right, the next run releases it.
runs "$(run_json 950 push main example/code success "$sha_b")" "$(run_json 860 push main example/code success "$sha_a")"
build 950 pm-console-apk-unsigned "build 950" "2026.11.24-bbbbbbb" 21756500
echo "00$(printf '%s' "$right_cert" | cut -c3-)" >"$tmp/cert"
refused "3 (wrong certificate)"
[ ! -d "$gh_state/pm-console-android-v21756500" ] || fail "3: a build with the wrong certificate was released"
grep -q "not by the certificate the site lists" "$tmp/out" || fail "3: it does not say why: $(cat "$tmp/out")"
echo "$right_cert" >"$tmp/cert"
go || fail "3: $(cat "$tmp/out")"
[ "$(cat "$state/released-run")" = 950 ] || fail "3: the newer run is not written down"
[ "$(cat "$serve")" = "signed:build 950" ] || fail "3: the console is not serving the newer build"
[ "$(field "$fixed/assets/version.json" code)" = 21756500 ] || fail "3: the fixed address does not hold the newer build"
[ "$(cat "$numbered/assets/pm-console.apk")" = "signed:build 860" ] || fail "3: the earlier numbered release was changed"

# 4. GitHub cannot release it (create fails): nothing is served or
#    written down, and the next run does it.
runs "$(run_json 960 push main example/code success "$sha_c")"
build 960 pm-console-apk-unsigned "build 960" "2026.11.25-ccccccc" 21756600
touch "$gh_state/.cannot-release"
refused "4 (the release cannot be made)"
rm "$gh_state/.cannot-release"
go || fail "4: $(cat "$tmp/out")"
[ "$(cat "$serve")" = "signed:build 960" ] || fail "4: the build was not released on the next run"

# 5. What a build must carry: exactly one APK, and its version.
runs "$(run_json 970 push main example/code success "$sha_a")"
build 970 pm-console-apk-unsigned "build 970" "2026.11.26-aaaaaaa" 21756700
printf 'a second apk' >"$gh_state/.files-970/another.apk"
refused "5 (two APKs)"
# A build from before builds carried their version is passed over, and
# that is said; it is not a failure, because it can never be mended.
rm "$gh_state/.files-970/another.apk" "$gh_state/.files-970/version"
before_signed="$(signed)"
go || fail "5 (no version): $(cat "$tmp/out")"
grep -q "does not say which version it built" "$tmp/out" || fail "5: it does not say the version is missing: $(cat "$tmp/out")"
[ "$(signed)" = "$before_signed" ] || fail "5: a build without its version was signed"
[ "$(cat "$state/released-run")" = 960 ] || fail "5: a build without its version was written down as done"
[ "$(cat "$serve")" = "signed:build 960" ] || fail "5: the served app changed"
printf '%s\n%s\n' "--draft" 21756700 >"$gh_state/.files-970/version"
refused "5 (a version name that is an option)"
printf '%s\n%s\n' "2026.11.26-aaaaaaa" "not a number" >"$gh_state/.files-970/version"
refused "5 (a version code that is not a number)"
[ ! -d "$gh_state/pm-console-android-vnot a number" ] || fail "5: released under a code that is not a number"

# 6. A build GitHub signed itself (it holds the key) is not signed again
#    here: it is written down, and nothing else happens.
runs "$(run_json 980 push main example/code success "$sha_b")"
build 980 pm-console-apk "signed by GitHub" "2026.11.27-bbbbbbb" 21756800
before_signed="$(signed)"
go || fail "6: $(cat "$tmp/out")"
[ "$(signed)" = "$before_signed" ] || fail "6: a build GitHub had signed was signed again"
[ "$(cat "$state/released-run")" = 980 ] || fail "6: the run is not written down"
[ "$(cat "$serve")" = "signed:build 960" ] || fail "6: the served app changed"

# 7. A build whose artifact is gone (they expire) cannot be signed: that
#    is said, it is not a failure, and it is looked at again next time.
runs "$(run_json 990 push main example/code success "$sha_c")"
build 990 "" "gone" "2026.11.28-ccccccc" 21756900
go || fail "7: $(cat "$tmp/out")"
grep -q "kept no build to sign" "$tmp/out" || fail "7: it does not say the build is gone: $(cat "$tmp/out")"
[ "$(cat "$state/released-run")" = 980 ] || fail "7: a run with nothing to sign was written down as done"

# 8. GitHub does not answer: a failure, so that the unit shows it.
touch "$gh_state/.github-is-down"
refused "8 (GitHub does not answer)"
rm "$gh_state/.github-is-down"

# 9. No build of ours at all: nothing to do, and not a failure.
runs "$(run_json 995 pull_request main someone/fork success "$sha_c")"
go || fail "9: $(cat "$tmp/out")"
grep -q "no build of the app made on main" "$tmp/out" || fail "9: $(cat "$tmp/out")"

# 10. Two at once: the second leaves it to the first and asks nothing.
runs "$(run_json 996 push main example/code success "$sha_a")"
build 996 pm-console-apk-unsigned "build 996" "2026.11.29-aaaaaaa" 21757000
before="$(calls)"
(
  flock 8
  go || fail "10: $(cat "$tmp/out")"
) 8>"$state/release.lock"
grep -q "another run is at work" "$tmp/out" || fail "10: $(cat "$tmp/out")"
[ "$(calls)" = "$before" ] || fail "10: the second run went to GitHub"

# 11. Where fleet is not installed, plain gh is used, and the build that
#     was waiting behind the lock is released.
export PATH="$without_fleet"
fleet_before="$(wc -l <"$gh_state/.fleet-log")"
go || fail "11: $(cat "$tmp/out")"
[ "$(cat "$serve")" = "signed:build 996" ] || fail "11: the waiting build was not released"
[ "$(wc -l <"$gh_state/.fleet-log")" = "$fleet_before" ] || fail "11: fleet was used though it is not installed"
export PATH="$with_fleet"

# 12. Nothing is asked of GitHub on bad input.
before="$(calls)"
if bash "$here/release-built.sh" something >/dev/null 2>&1; then fail "12: took an argument"; fi
if PM_ANDROID_REPO="not a repository" bash "$here/release-built.sh" >/dev/null 2>&1; then fail "12: took a bad repository"; fi
[ "$(calls)" = "$before" ] || fail "12: went to GitHub on bad input"

echo "release-built.test.sh: ok"
