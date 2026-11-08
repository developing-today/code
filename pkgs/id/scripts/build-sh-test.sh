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
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/build.sh"
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

echo "build-sh tests passed"
