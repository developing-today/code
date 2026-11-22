#!/usr/bin/env bash
# Regression test for scripts/build.sh variant handling (#6869).
#
# build.sh crashed for the `lib` variant before reaching cargo:
# needs_frontend was assigned only inside the web/assets Step 1 block
# but expanded unconditionally under `set -u`. This test runs all
# three variants of the real script in a fixture tree with a stubbed
# cargo, so a variant-specific unbound variable (or a variant quietly
# gaining/losing its cargo flags) fails here instead of in a just
# recipe.
# Usage: build-sh-test.sh
#   Accepts no positional arguments (operator-script argument contract,
#   #3352 / #10567): extra arguments are rejected before any fixture work.
set -euo pipefail

usage() {
  echo "Usage: build-sh-test.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

script="$(cd "$(dirname "$0")" && pwd)/build.sh"
self="$(cd "$(dirname "$0")" && pwd)/build-sh-test.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin" "$tmp/work/web/dist" "$tmp/work/src" "$tmp/work/crates" \
  "$tmp/work/target/debug" "$tmp/work/target/release"
cp "$script" "$tmp/work/build.sh"

# Stub cargo: record every invocation, produce nothing. build.sh
# touches the binary itself after a successful cargo run.
printf '#!/bin/sh\necho "cargo $@" >>"%s/cargo.log"\n' "$tmp" >"$tmp/bin/cargo"
chmod +x "$tmp/bin/cargo"
export PATH="$tmp/bin:$PATH"

cd "$tmp/work"
# An up-to-date manifest: the frontend step must stay a no-op so the
# web/assets controls never invoke bun.
printf '{"main.js":{}}\n' >web/dist/manifest.json
touch Cargo.toml Cargo.lock

fail() {
  echo "build-sh-test: FAIL: $1" >&2
  exit 1
}

# lib debug: must reach cargo with NO --features web and record the
# lib variant. (Pre-fix: exit 1, "needs_frontend: unbound variable",
# cargo never called.)
bash ./build.sh lib debug >lib.out 2>lib.err || {
  cat lib.err >&2
  fail "lib debug exited non-zero"
}
grep -qx 'cargo build' "$tmp/cargo.log" ||
  fail "lib debug did not invoke plain 'cargo build' (log: $(cat "$tmp/cargo.log" 2>/dev/null || echo none))"
[ "$(cat target/.build-variant)" = "lib" ] ||
  fail "lib debug did not write 'lib' to target/.build-variant"

# lib release: same contract on the release paths.
bash ./build.sh lib release >libr.out 2>libr.err || {
  cat libr.err >&2
  fail "lib release exited non-zero"
}
grep -qx 'cargo build --release' "$tmp/cargo.log" ||
  fail "lib release did not invoke 'cargo build --release'"
[ "$(cat target/.build-variant-release)" = "lib" ] ||
  fail "lib release did not write 'lib' to target/.build-variant-release"

# assets control: exits 0 and never invokes cargo.
before="$(wc -l <"$tmp/cargo.log")"
bash ./build.sh assets >assets.out 2>&1 ||
  fail "assets exited non-zero"
after="$(wc -l <"$tmp/cargo.log")"
[ "$before" = "$after" ] || fail "assets invoked cargo"

# web control: still invokes cargo with --features web and records
# the web variant (the variant mismatch with the lib build above is
# what must trigger the rebuild).
bash ./build.sh web debug >web.out 2>web.err || {
  cat web.err >&2
  fail "web debug exited non-zero"
}
grep -qx 'cargo build --features web' "$tmp/cargo.log" ||
  fail "web debug did not invoke 'cargo build --features web'"
[ "$(cat target/.build-variant)" = "web" ] ||
  fail "web debug did not write 'web' to target/.build-variant"

# ── Disturbed-marker cases (#6883) ──────────────────────────────────
# The marker is the only variant evidence for the binary at the shared
# target/debug/id path. A missing, empty, or garbage marker must force
# a rebuild of the requested variant — never an "up to date" verdict on
# a binary that may be the other variant. (Pre-fix: all three cases
# printed "up to date" and never invoked cargo.)

# Missing marker: delete it, request lib — cargo must run with the lib
# flags and the marker must be rewritten as 'lib'.
rm target/.build-variant
bash ./build.sh lib debug >missing.out 2>missing.err || {
  cat missing.err >&2
  fail "missing-marker lib debug exited non-zero"
}
[ "$(tail -n 1 "$tmp/cargo.log")" = "cargo build" ] ||
  fail "missing marker did not force a lib rebuild (log tail: $(tail -n 1 "$tmp/cargo.log"))"
[ "$(cat target/.build-variant)" = "lib" ] ||
  fail "missing-marker rebuild did not rewrite the marker as 'lib'"

# Empty marker (the interrupted-build state): truncate it, request
# web — cargo must run with --features web, marker rewritten as 'web'.
: >target/.build-variant
bash ./build.sh web debug >empty.out 2>empty.err || {
  cat empty.err >&2
  fail "empty-marker web debug exited non-zero"
}
[ "$(tail -n 1 "$tmp/cargo.log")" = "cargo build --features web" ] ||
  fail "empty marker did not force a web rebuild (log tail: $(tail -n 1 "$tmp/cargo.log"))"
[ "$(cat target/.build-variant)" = "web" ] ||
  fail "empty-marker rebuild did not rewrite the marker as 'web'"

# Garbage marker: content that is neither variant, request lib — cargo
# must run with the lib flags, marker rewritten as 'lib'.
printf 'banana\n' >target/.build-variant
bash ./build.sh lib debug >garbage.out 2>garbage.err || {
  cat garbage.err >&2
  fail "garbage-marker lib debug exited non-zero"
}
[ "$(tail -n 1 "$tmp/cargo.log")" = "cargo build" ] ||
  fail "garbage marker did not force a lib rebuild (log tail: $(tail -n 1 "$tmp/cargo.log"))"
[ "$(cat target/.build-variant)" = "lib" ] ||
  fail "garbage-marker rebuild did not rewrite the marker as 'lib'"

# Exact-match control: with the marker intact and matching, an
# immediate re-run of the same variant takes the up-to-date path and
# does NOT invoke cargo (the other side of the #6883 rule).
before="$(wc -l <"$tmp/cargo.log")"
bash ./build.sh lib debug >uptodate.out 2>uptodate.err || {
  cat uptodate.err >&2
  fail "exact-match lib debug exited non-zero"
}
after="$(wc -l <"$tmp/cargo.log")"
[ "$before" = "$after" ] ||
  fail "exact-match marker wrongly forced a rebuild (cargo invoked)"
grep -q 'up to date' uptodate.out ||
  fail "exact-match re-run did not report 'up to date'"

# ── Argument contract (#10501) ──────────────────────────────────────
# Extra positional arguments must be rejected before any frontend or
# Rust build work, with usage on stderr and non-zero exit.
before="$(wc -l <"$tmp/cargo.log")"
if bash ./build.sh lib debug extra >extra.out 2>extra.err; then
  fail "extra argument was accepted"
fi
after="$(wc -l <"$tmp/cargo.log")"
[ "$before" = "$after" ] || fail "extra argument invoked cargo"
grep -q 'Usage: build.sh \[variant\] \[profile\]' extra.err ||
  fail "extra argument did not print usage"
grep -q 'unaccounted argument' extra.err ||
  fail "extra argument did not report unaccounted argument"

# ── Own argument contract (#10567) ────────────────────────────────────
# This script takes no arguments either: a copy invoked with an extra
# argument must be rejected (exit 1, usage + unaccounted-argument on
# stderr) before any fixture work. TMPDIR is pointed at a fresh dir so
# a broken preflight would leave its mktemp tree behind as evidence.
mkdir -p "$tmp/selfcheck-empty"
cp "$self" "$tmp/selfcheck.sh"
rc=0
TMPDIR="$tmp/selfcheck-empty" bash "$tmp/selfcheck.sh" extra >"$tmp/selfcheck.out" 2>"$tmp/selfcheck.err" || rc=$?
[ "$rc" -eq 1 ] || fail "own extra argument exited $rc instead of 1"
grep -q 'Usage: build-sh-test.sh' "$tmp/selfcheck.err" ||
  fail "own extra argument did not print usage"
grep -q 'unaccounted argument' "$tmp/selfcheck.err" ||
  fail "own extra argument did not report unaccounted argument"
if [ -n "$(ls -A "$tmp/selfcheck-empty")" ]; then
  fail "own extra argument did fixture work before rejection"
fi

# ── Shared target path ──────────────────────────────────────────────
# fleet shell --build sets CARGO_TARGET_DIR to a broker lease. The helper
# must inspect and mark the binary there rather than a stale local target/.
shared="$tmp/shared-target"
mkdir -p "$shared/debug" "$shared/release"
rm -f target/debug/id
before="$(wc -l <"$tmp/cargo.log")"
CARGO_TARGET_DIR="$shared" bash ./build.sh lib debug >shared.out 2>shared.err || {
  cat shared.err >&2
  fail "shared-target lib debug exited non-zero"
}
after="$(wc -l <"$tmp/cargo.log")"
[ "$after" -eq "$((before + 1))" ] ||
  fail "shared target did not invoke cargo exactly once"
[ -f "$shared/debug/id" ] ||
  fail "shared target did not place the binary under CARGO_TARGET_DIR"
[ "$(cat "$shared/.build-variant")" = "lib" ] ||
  fail "shared target did not place the variant marker under CARGO_TARGET_DIR"
[ ! -e target/debug/id ] ||
  fail "shared target unexpectedly wrote a local target/debug/id"

# Exact match on the leased target must still avoid recompiling.
before="$after"
CARGO_TARGET_DIR="$shared" bash ./build.sh lib debug >shared-uptodate.out 2>shared-uptodate.err || {
  cat shared-uptodate.err >&2
  fail "shared-target exact match exited non-zero"
}
after="$(wc -l <"$tmp/cargo.log")"
[ "$before" = "$after" ] ||
  fail "shared-target exact match invoked cargo"
grep -q 'up to date' shared-uptodate.out ||
  fail "shared-target exact match did not report 'up to date'"

# ── Shared target path ──────────────────────────────────────────────
# fleet shell --build sets CARGO_TARGET_DIR to a broker lease. The helper
# must inspect and mark the binary there rather than a stale local target/.
shared="$tmp/shared-target"
mkdir -p "$shared/debug" "$shared/release"
rm -f target/debug/id
before="$(wc -l <"$tmp/cargo.log")"
CARGO_TARGET_DIR="$shared" bash ./build.sh lib debug >shared.out 2>shared.err || {
  cat shared.err >&2
  fail "shared-target lib debug exited non-zero"
}
after="$(wc -l <"$tmp/cargo.log")"
[ "$after" -eq "$((before + 1))" ] ||
  fail "shared target did not invoke cargo exactly once"
[ -f "$shared/debug/id" ] ||
  fail "shared target did not place the binary under CARGO_TARGET_DIR"
[ "$(cat "$shared/.build-variant")" = "lib" ] ||
  fail "shared target did not place the variant marker under CARGO_TARGET_DIR"
[ ! -e target/debug/id ] ||
  fail "shared target unexpectedly wrote a local target/debug/id"

# Exact match on the leased target must still avoid recompiling.
before="$after"
CARGO_TARGET_DIR="$shared" bash ./build.sh lib debug >shared-uptodate.out 2>shared-uptodate.err || {
  cat shared-uptodate.err >&2
  fail "shared-target exact match exited non-zero"
}
after="$(wc -l <"$tmp/cargo.log")"
[ "$before" = "$after" ] ||
  fail "shared-target exact match invoked cargo"
grep -q 'up to date' shared-uptodate.out ||
  fail "shared-target exact match did not report 'up to date'"

# ── Teardown: manifest/source rebuild path (#6883/#6869 fixed form) ─
# Not part of the arg-contract case; keep the file ending stable.
echo "build-sh tests passed"
