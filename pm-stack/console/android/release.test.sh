#!/usr/bin/env bash
# release.test.sh — pins what release.sh does to the two releases, with
# a stand-in for gh that keeps releases in a directory and writes down
# every call: the numbered release is made once and never changed, the
# fixed address always ends up holding the newest build and never an
# older one, a run started again changes nothing, and nothing is asked
# of GitHub on bad input.
set -euo pipefail

if [ "$#" -gt 0 ]; then
  echo "usage: release.test.sh (it takes no arguments)" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
state="$tmp/github"
mkdir -p "$tmp/bin" "$state"

# ---- a stand-in for gh ------------------------------------------------------
cat >"$tmp/bin/gh" <<'GH'
#!/usr/bin/env bash
# Releases live under $GH_STATE/<tag>/: draft, target, latest, title,
# notes, assets/. Tags under $GH_STATE/.tags/. Every call is logged.
set -euo pipefail
echo "$*" >>"$GH_STATE/.log"
if [ "$1" = "api" ]; then
  [ -e "$GH_STATE/.tag-cannot-move" ] && exit 1
  path="" sha=""
  for arg in "$@"; do
    case "$arg" in
    repos/*) path="$arg" ;;
    sha=*) sha="${arg#sha=}" ;;
    esac
  done
  mkdir -p "$GH_STATE/.tags"
  echo "$sha" >"$GH_STATE/.tags/${path##*/}"
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
view)
  [ -d "$dir" ]
  ;;
create)
  [ ! -d "$dir" ] || exit 1
  mkdir -p "$dir/assets"
  echo false >"$dir/draft"
  echo "${latest:-true}" >"$dir/latest"
  echo "${target:-}" >"$dir/target"
  echo "${title:-}" >"$dir/title"
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
  [ -n "${draft:-}" ] && echo "$draft" >"$dir/draft"
  [ -n "${latest:-}" ] && echo "$latest" >"$dir/latest"
  [ -n "${title:-}" ] && echo "$title" >"$dir/title"
  [ -n "${notes:-}" ] && echo "$notes" >"$dir/notes"
  true
  ;;
*) exit 64 ;;
esac
GH
chmod +x "$tmp/bin/gh"

export PATH="$tmp/bin:$PATH" GH_STATE="$state"
export GITHUB_REPOSITORY="example/code" GITHUB_RUN_ID=42 GITHUB_SERVER_URL="https://github.com"
unset GITHUB_STEP_SUMMARY

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
same() { cmp -s "$1" "$2" || fail "$3"; }
sha_a="$(printf 'a%.0s' {1..40})"
sha_b="$(printf 'b%.0s' {1..40})"
fixed="$state/pm-console-android"
field() { python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
release() { # release <apk> <name> <code> <sha>
  VERSION_NAME="$2" VERSION_CODE="$3" GITHUB_SHA="$4" bash "$here/release.sh" "$1"
}
calls() { wc -l <"$state/.log"; }

printf 'build one' >"$tmp/one.apk"
printf 'build one, built again' >"$tmp/one-again.apk"
printf 'build two!' >"$tmp/two.apk"

# 1. The first release, with the draft that is there today: a release
#    of its own for the build, and the draft published holding it.
mkdir -p "$fixed/assets"
echo true >"$fixed/draft"
echo true >"$fixed/latest"
printf 'signed on the laptop' >"$fixed/assets/pm-console.apk"
release "$tmp/one.apk" 2026.11.23-aaaaaaa 100 "$sha_a" >/dev/null
one="$state/pm-console-android-v100"
[ -d "$one" ] || fail "1: no release was made for the build itself"
same "$one/assets/pm-console.apk" "$tmp/one.apk" "1: the numbered release does not hold the build"
same "$fixed/assets/pm-console.apk" "$tmp/one.apk" "1: the fixed address does not hold the build"
[ "$(cat "$fixed/draft")" = false ] || fail "1: the fixed release is still a draft"
[ "$(cat "$fixed/latest")" = false ] || fail "1: the fixed release was marked the repository's latest"
[ "$(cat "$one/latest")" = false ] || fail "1: the numbered release was marked the repository's latest"
[ "$(cat "$one/target")" = "$sha_a" ] || fail "1: the numbered release is not at the build's commit"
[ "$(cat "$state/.tags/pm-console-android")" = "$sha_a" ] || fail "1: the fixed tag was not moved to the build's commit"
[ "$(field "$fixed/assets/version.json" code)" = 100 ] || fail "1: version.json gives the wrong code"
[ "$(field "$fixed/assets/version.json" name)" = 2026.11.23-aaaaaaa ] || fail "1: version.json gives the wrong name"
[ "$(field "$fixed/assets/version.json" commit)" = "$sha_a" ] || fail "1: version.json gives the wrong commit"
[ "$(field "$fixed/assets/version.json" size)" = 9 ] || fail "1: version.json gives the wrong size"
want="$(sha256sum "$tmp/one.apk" | cut -d' ' -f1)"
[ "$(field "$fixed/assets/version.json" sha256)" = "$want" ] || fail "1: version.json gives the wrong sha256"
[ "$(cat "$fixed/assets/pm-console.apk.sha256")" = "$want  pm-console.apk" ] || fail "1: the .sha256 file is not what sha256sum -c reads"
[ "$(field "$fixed/assets/version.json" certificate)" = "$(tr -d '[:space:]' <"$here/signing-fingerprint.txt")" ] || fail "1: version.json does not name the signing certificate"
# At the fixed address the build goes up before its description.
apk_at="$(grep -n "release upload pm-console-android .*pm-console.apk" "$state/.log" | head -1 | cut -d: -f1)"
json_at="$(grep -n "release upload pm-console-android .*version.json" "$state/.log" | head -1 | cut -d: -f1)"
[ -n "$apk_at" ] && [ -n "$json_at" ] && [ "$apk_at" -lt "$json_at" ] || fail "1: version.json was put up before the build it describes"

# 2. The same build's run started again, and the build came out a few
#    bytes different: nothing is changed anywhere.
before="$(calls)"
release "$tmp/one-again.apk" 2026.11.23-aaaaaaa 100 "$sha_a" >/dev/null
same "$one/assets/pm-console.apk" "$tmp/one.apk" "2: a numbered release was changed after it was made"
same "$fixed/assets/pm-console.apk" "$tmp/one.apk" "2: the fixed address took a second build of the same version"
if sed -n "$((before + 1)),\$p" "$state/.log" | grep -q "release create\|release upload pm-console-android-v"; then
  fail "2: the numbered release was written to again"
fi

# 3. A later build: its own release, and the fixed address moves to it.
release "$tmp/two.apk" 2026.11.24-bbbbbbb 200 "$sha_b" >/dev/null
two="$state/pm-console-android-v200"
same "$two/assets/pm-console.apk" "$tmp/two.apk" "3: the second build has no release of its own"
same "$fixed/assets/pm-console.apk" "$tmp/two.apk" "3: the fixed address did not move to the newer build"
same "$one/assets/pm-console.apk" "$tmp/one.apk" "3: the first numbered release was changed"
[ "$(field "$fixed/assets/version.json" code)" = 200 ] || fail "3: the fixed address describes the older build"
[ "$(cat "$state/.tags/pm-console-android")" = "$sha_b" ] || fail "3: the fixed tag did not follow"
grep -q "now 2026.11.24-bbbbbbb" "$fixed/notes" || fail "3: the fixed release's notes do not name the build it holds"

# 4. The older build's run started again after the newer one is out:
#    the fixed address is not taken back.
release "$tmp/one.apk" 2026.11.23-aaaaaaa 100 "$sha_a" >/dev/null
same "$fixed/assets/pm-console.apk" "$tmp/two.apk" "4: the fixed address went back to an older build"
[ "$(field "$fixed/assets/version.json" code)" = 200 ] || fail "4: the fixed address describes an older build"
[ "$(cat "$state/.tags/pm-console-android")" = "$sha_b" ] || fail "4: the fixed tag went back"

# 5. A run that died between the two releases: the numbered one is out
#    and the fixed address is behind. The next run brings it up, with
#    the build that was released, not its own.
printf 'build three' >"$tmp/three.apk"
printf 'build three, built again' >"$tmp/three-again.apk"
rm -rf "$fixed"
release "$tmp/three.apk" 2026.11.25-ccccccc 300 "$sha_b" >/dev/null
rm -rf "$fixed"
mkdir -p "$fixed/assets"
cp "$two/assets/"* "$fixed/assets/"
echo false >"$fixed/draft"
release "$tmp/three-again.apk" 2026.11.25-ccccccc 300 "$sha_b" >/dev/null
same "$fixed/assets/pm-console.apk" "$tmp/three.apk" "5: the fixed address was not brought up to the released build"
[ "$(field "$fixed/assets/version.json" code)" = 300 ] || fail "5: the fixed address still describes the older build"

# 6. No fixed release at all: it is made.
rm -rf "$fixed"
printf 'build four' >"$tmp/four.apk"
release "$tmp/four.apk" 2026.11.26-ddddddd 400 "$sha_a" >/dev/null
same "$fixed/assets/pm-console.apk" "$tmp/four.apk" "6: the fixed release was not made"
[ "$(cat "$fixed/latest")" = false ] || fail "6: a new fixed release was marked the repository's latest"

# 7. The tag cannot be moved: the release still holds the build, and the run does not fail.
touch "$state/.tag-cannot-move"
printf 'build five' >"$tmp/five.apk"
out="$(release "$tmp/five.apk" 2026.11.27-eeeeeee 500 "$sha_b")" || fail "7: a tag that cannot be moved failed the release"
same "$fixed/assets/pm-console.apk" "$tmp/five.apk" "7: the fixed address does not hold the build"
printf '%s' "$out" | grep -q "could not move the tag" || fail "7: nothing was said about the tag"
rm "$state/.tag-cannot-move"

# 8. A numbered release that is there but incomplete is not built upon,
#    and the run says which file it lacks before touching anything.
rm "$state/pm-console-android-v500/assets/pm-console.apk.sha256"
before="$(calls)"
if said="$(release "$tmp/five.apk" 2026.11.27-eeeeeee 500 "$sha_b" 2>&1)"; then
  fail "8: an incomplete numbered release was accepted"
fi
printf '%s' "$said" | grep -q "exists without pm-console.apk.sha256" || fail "8: it did not say what the release lacks"
if sed -n "$((before + 1)),\$p" "$state/.log" | grep -q "release upload\|release edit\|release create"; then
  fail "8: something was written on top of an incomplete release"
fi

# 9. Bad input stops it before GitHub is asked anything.
before="$(calls)"
bad() { # bad <what> <name> <code> <sha> [<apk>]
  if release "${5:-$tmp/one.apk}" "$2" "$3" "$4" >/dev/null 2>&1; then fail "9: accepted $1"; fi
  [ "$(calls)" = "$before" ] || fail "9: asked GitHub something on $1"
}
bad "a code that is not a number" 2026.11.23-aaaaaaa "12; echo" "$sha_a"
bad "a code of zero" 2026.11.23-aaaaaaa 0 "$sha_a"
bad "no code" 2026.11.23-aaaaaaa "" "$sha_a"
bad "a name with a space" "2026 11" 100 "$sha_a"
bad "a name that is an option" "--draft" 100 "$sha_a"
bad "a short commit id" 2026.11.23-aaaaaaa 100 aaaaaaa
bad "a file that is not there" 2026.11.23-aaaaaaa 100 "$sha_a" "$tmp/none.apk"
if GITHUB_REPOSITORY="not a repo" release "$tmp/one.apk" 2026.11.23-aaaaaaa 100 "$sha_a" >/dev/null 2>&1; then
  fail "9: accepted a repository that is not owner/name"
fi
if bash "$here/release.sh" >/dev/null 2>&1; then fail "9: ran with no file named"; fi

echo "release.test.sh: ok"
