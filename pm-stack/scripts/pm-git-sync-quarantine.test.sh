#!/usr/bin/env bash
# pm-git-sync-quarantine.test.sh — pins pm-git-sync's clone hygiene
# (2026-11-20): untracked files are moved aside (content kept) and the
# pull goes ahead; tracked edits and a non-main branch still refuse.
# Self-contained: builds a throwaway origin + clone under a temp dir.
set -euo pipefail

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
(cd "$tmp/seed" && printf 'ignored.log\n' >.gitignore && echo one >tracked.txt &&
  git add . && git commit -qm one && git push -q origin main)
git clone -q "$tmp/origin.git" "$tmp/clone"
(cd "$tmp/seed" && echo two >tracked.txt && git commit -qam two && git push -q origin main)

export PM_GIT_SYNC_REPO="$tmp/clone"
export PM_GIT_SYNC_QUARANTINE="$tmp/quarantine"

# 1. Untracked files (top level and nested, with a space) are moved aside;
#    ignored files stay; the pull lands.
echo ledger >"$tmp/clone/advocate-ledger.md"
mkdir -p "$tmp/clone/pm-stack/x"
echo nested >"$tmp/clone/pm-stack/x/fire log.txt"
echo keep >"$tmp/clone/ignored.log"
out="$(bash "$sync" 2>&1)" || fail "untracked case exited non-zero: $out"
printf '%s\n' "$out" | grep -q "moved 2 untracked file(s)" || fail "no move summary: $out"
[ ! -e "$tmp/clone/advocate-ledger.md" ] || fail "untracked file left in clone"
[ -e "$tmp/clone/ignored.log" ] || fail "ignored file was moved"
q="$(find "$tmp/quarantine" -name advocate-ledger.md)"
[ "$(cat "$q")" = ledger ] || fail "quarantined content changed"
[ "$(cat "$(dirname "$q")/pm-stack/x/fire log.txt")" = nested ] || fail "nested path not kept"
[ "$(cat "$tmp/clone/tracked.txt")" = two ] || fail "pull did not land"
[ -z "$(git -C "$tmp/clone" status --porcelain)" ] || fail "clone not clean"

# 2. A tracked edit still refuses, and nothing untracked is moved.
(cd "$tmp/seed" && echo three >tracked.txt && git commit -qam three && git push -q origin main)
echo local >"$tmp/clone/tracked.txt"
echo stray >"$tmp/clone/stray.md"
if out="$(bash "$sync" 2>&1)"; then fail "tracked edit did not refuse: $out"; fi
printf '%s\n' "$out" | grep -q "working tree is dirty" || fail "wrong refusal: $out"
[ -e "$tmp/clone/stray.md" ] || fail "moved untracked file despite refusing"
git -C "$tmp/clone" checkout -q -- tracked.txt

# 3. Off main still refuses.
git -C "$tmp/clone" checkout -q -b side
if out="$(bash "$sync" 2>&1)"; then fail "off-main did not refuse: $out"; fi
printf '%s\n' "$out" | grep -q "not main" || fail "wrong off-main refusal: $out"
git -C "$tmp/clone" checkout -q main

# 4. Back on main with only the untracked file: it moves and the pull lands.
out="$(bash "$sync" 2>&1)" || fail "recovery run exited non-zero: $out"
[ "$(cat "$tmp/clone/tracked.txt")" = three ] || fail "recovery pull did not land"
[ ! -e "$tmp/clone/stray.md" ] || fail "stray left in clone"

echo "pm-git-sync quarantine suite passed."
