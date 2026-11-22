#!/usr/bin/env bash
# pm-git-sync-markers.test.sh — pins what pm-git-sync does when the
# clone's main has commits origin does not (2026-11-22): empty rebuild
# markers that block the pull are kept under tags and main follows
# origin; a local commit that changes a file, or a merge, still refuses
# and moves nothing; a marker that blocks nothing is left alone.
# Self-contained: builds a throwaway origin + clone under a temp dir.
set -euo pipefail

if [ "$#" -gt 0 ]; then
  echo "usage: pm-git-sync-markers.test.sh (it takes no arguments)" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sync="$here/pm-git-sync.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

git init -q --bare -b main "$tmp/origin.git"
git clone -q "$tmp/origin.git" "$tmp/seed" 2>/dev/null
(cd "$tmp/seed" && echo one >tracked.txt && git add . && git commit -qm one && git push -q origin main)
git clone -q "$tmp/origin.git" "$tmp/clone"
clone="$tmp/clone"

export PM_GIT_SYNC_REPO="$clone"
export PM_GIT_SYNC_QUARANTINE="$tmp/quarantine"

upstream() { # one more commit on origin
  (cd "$tmp/seed" && git pull -q origin main && echo "$1" >tracked.txt && git commit -qam "$1" && git push -q origin main)
}
head_of() { git -C "$1" rev-parse HEAD; }

# 1. A marker with origin unmoved blocks nothing: it is left where it is
#    (the hourly keeper pushes it), and no tag is written.
git -C "$clone" commit -q --allow-empty -m "amd 569"
marker1="$(head_of "$clone")"
out="$(bash "$sync" 2>&1)" || fail "ahead-only exited non-zero: $out"
[ "$(head_of "$clone")" = "$marker1" ] || fail "ahead-only marker was moved: $out"
[ -z "$(git -C "$clone" tag -l)" ] || fail "ahead-only wrote a tag"

# 2. Origin moves: the clone is ahead and behind at once. Two empty
#    markers are kept under tags named for them and main follows origin.
git -C "$clone" commit -q --allow-empty -m "amd 570"
marker2="$(head_of "$clone")"
upstream two
out="$(bash "$sync" 2>&1)" || fail "marker case exited non-zero: $out"
[ "$(head_of "$clone")" = "$(git -C "$tmp/seed" rev-parse HEAD)" ] || fail "main did not follow origin: $out"
[ "$(cat "$clone/tracked.txt")" = two ] || fail "the pull did not land"
[ "$(git -C "$clone" rev-parse 'rebuild-marker/amd-569^{commit}')" = "$marker1" ] || fail "first marker not kept: $out"
[ "$(git -C "$clone" rev-parse 'rebuild-marker/amd-570^{commit}')" = "$marker2" ] || fail "second marker not kept: $out"
[ -z "$(git -C "$clone" status --porcelain)" ] || fail "clone not clean"
printf '%s\n' "$out" | grep -q 'moved 2 empty commit(s) aside' || fail "no summary: $out"
# The next run is an ordinary one.
out="$(bash "$sync" 2>&1)" || fail "follow-up run exited non-zero: $out"
printf '%s\n' "$out" | grep -q 'up to date' || fail "follow-up run not up to date: $out"

# 3. A marker whose name is already taken by another commit gets a tag
#    of its own; the earlier tag still points at the earlier commit.
git -C "$clone" commit -q --allow-empty -m "amd 570"
marker3="$(head_of "$clone")"
upstream three
out="$(bash "$sync" 2>&1)" || fail "same-name marker exited non-zero: $out"
[ "$(git -C "$clone" rev-parse 'rebuild-marker/amd-570^{commit}')" = "$marker2" ] || fail "earlier tag was overwritten"
[ "$(git -C "$clone" rev-parse "rebuild-marker/$(git -C "$clone" rev-parse --short=12 "$marker3")^{commit}")" = "$marker3" ] ||
  fail "same-name marker not kept under its own tag: $out"
[ "$(cat "$clone/tracked.txt")" = three ] || fail "pull after same-name marker did not land"

# 4. A subject that is not a plain name still gives a usable tag.
git -C "$clone" commit -q --allow-empty -m "nixos 12: weird/../subject.lock"
marker4="$(head_of "$clone")"
upstream four
out="$(bash "$sync" 2>&1)" || fail "odd subject exited non-zero: $out"
[ "$(git -C "$clone" for-each-ref --points-at "$marker4" --format='%(refname)' refs/tags | wc -l)" = 1 ] ||
  fail "odd-subject marker not kept: $out"
[ "$(cat "$clone/tracked.txt")" = four ] || fail "pull after odd subject did not land"

# 5. A local commit that changes a file refuses, even beside an empty
#    one, and nothing is tagged or moved.
tags_before="$(git -C "$clone" tag -l | sort)"
git -C "$clone" commit -q --allow-empty -m "amd 571"
(cd "$clone" && echo lock >flake.lock && git add flake.lock && git commit -qm "amd 572")
local_head="$(head_of "$clone")"
upstream five
if out="$(bash "$sync" 2>&1)"; then fail "a commit with a change did not refuse: $out"; fi
printf '%s\n' "$out" | grep -q 'amd 572 changes files or is a merge' || fail "wrong refusal: $out"
[ "$(head_of "$clone")" = "$local_head" ] || fail "main moved despite refusing"
[ "$(git -C "$clone" tag -l | sort)" = "$tags_before" ] || fail "tagged despite refusing"
[ "$(cat "$clone/flake.lock")" = lock ] || fail "local change lost"

# 6. So does a merge, even one that changes nothing against its first parent.
git -C "$clone" reset -q --hard "$(git -C "$clone" rev-parse origin/main~1)"
git -C "$clone" checkout -q -b side
git -C "$clone" commit -q --allow-empty -m side
git -C "$clone" checkout -q main
git -C "$clone" merge -q --no-ff -m "merge side" side
merge_head="$(head_of "$clone")"
if out="$(bash "$sync" 2>&1)"; then fail "a local merge did not refuse: $out"; fi
printf '%s\n' "$out" | grep -q 'changes files or is a merge' || fail "wrong merge refusal: $out"
[ "$(head_of "$clone")" = "$merge_head" ] || fail "main moved despite a local merge"

# 7. A rebuild that commits while the sync is at work (it does not
#    coordinate with it) never loses its commit. A stand-in git makes a
#    commit in the clone the first time the sync runs a chosen step.
real_git="$(command -v git)"
mkdir -p "$tmp/bin"
cat >"$tmp/bin/git" <<SHIM
#!/usr/bin/env bash
if [ "\${1:-}" = "\${SHIM_ON:-}" ] && [ ! -e "$tmp/shim-done" ]; then
  : >"$tmp/shim-done"
  "$real_git" -C "$clone" commit -q --allow-empty -m "amd 600"
fi
exec "$real_git" "\$@"
SHIM
chmod +x "$tmp/bin/git"
fresh() { # back on origin/main with one marker, origin one ahead
  "$real_git" -C "$clone" checkout -q main 2>/dev/null || true
  "$real_git" -C "$clone" reset -q --hard origin/main
  "$real_git" -C "$clone" commit -q --allow-empty -m "amd 599"
  upstream "$1"
  rm -f "$tmp/shim-done"
}

# 7a. It lands while the markers are being tagged: main is not moved at
#     all, and the next run takes both markers aside.
fresh six
if out="$(SHIM_ON=tag PATH="$tmp/bin:$PATH" bash "$sync" 2>&1)"; then fail "a commit during tagging did not refuse: $out"; fi
printf '%s\n' "$out" | grep -q 'main moved while this ran' || fail "wrong refusal for a commit during tagging: $out"
[ "$(git -C "$clone" log -1 --format=%s)" = "amd 600" ] || fail "the commit made during tagging is not on main"
out="$(bash "$sync" 2>&1)" || fail "run after a commit during tagging exited non-zero: $out"
[ "$(git -C "$clone" log -1 --format=%s 'rebuild-marker/amd-600')" = "amd 600" ] || fail "late marker not kept: $out"
[ "$(cat "$clone/tracked.txt")" = six ] || fail "pull after a commit during tagging did not land"

# 7b. It lands in the instant before main is moved: main follows origin,
#     the commit is kept under a tag of its own, and the run fails loudly.
fresh seven
if out="$(SHIM_ON=reset PATH="$tmp/bin:$PATH" bash "$sync" 2>&1)"; then fail "a commit at the move did not fail: $out"; fi
printf '%s\n' "$out" | grep -q 'is NOT on main' || fail "wrong message for a commit at the move: $out"
late="$(git -C "$clone" for-each-ref --format='%(refname:short)' 'refs/tags/rebuild-marker/unexpected-*')"
[ -n "$late" ] && [ "$(git -C "$clone" log -1 --format=%s "$late")" = "amd 600" ] || fail "commit at the move not kept: $out"
[ "$(head_of "$clone")" = "$(git -C "$clone" rev-parse origin/main)" ] || fail "main did not follow origin after a commit at the move"
out="$(bash "$sync" 2>&1)" || fail "run after a commit at the move exited non-zero: $out"

echo "pm-git-sync markers suite passed."
