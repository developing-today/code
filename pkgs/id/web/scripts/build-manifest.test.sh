#!/bin/sh
# Regression fixture for build-manifest.ts double-run idempotency (#6969).
#
# Exercises, in a throwaway fixture dist:
#   1. run 1 on styles.css + a JS bundle: manifest maps styles.css to a
#      hashed file that exists, main.js is mapped;
#   2. run 2 (no unhashed styles.css left): the hashed stylesheet and
#      its manifest entry SURVIVE (the #6969 bug deleted both);
#   3. rotation: a new styles.css with different content replaces the
#      old hashed file and the manifest points at the new one;
#   4. adoption from scratch: a dist holding only a contract-valid
#      styles.<8-hex>.css adopts it into the manifest untouched;
#   5. a non-contract name (styles.darkmode.css) is never deleted.
#
# Run: sh pkgs/id/web/scripts/build-manifest.test.sh   (needs bun)
set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
BUN=${BUN:-bun}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $1" >&2; exit 1; }

manifest_styles() {
  # print the manifest's styles.css target, or nothing
  sed -n 's/.*"styles.css": "\([^"]*\)".*/\1/p' "$1/dist/manifest.json"
}

run_manifest() {
  (cd "$1" && "$BUN" run "$SCRIPT_DIR/build-manifest.ts" >/dev/null)
}

# --- cases 1+2: double run -------------------------------------------------
D=$TMP/double
mkdir -p "$D/dist"
printf 'body { color: red; }\n' > "$D/dist/styles.css"
printf 'console.log(1);\n' > "$D/dist/main.abc12xyz.js"
run_manifest "$D"
FIRST=$(manifest_styles "$D")
[ -n "$FIRST" ] || fail "run 1: manifest has no styles.css entry"
[ -f "$D/dist/$FIRST" ] || fail "run 1: $FIRST missing from dist"
grep -q '"main.js": "main.abc12xyz.js"' "$D/dist/manifest.json" || fail "run 1: main.js entry missing"
run_manifest "$D"
SECOND=$(manifest_styles "$D")
[ "$SECOND" = "$FIRST" ] || fail "run 2: styles.css entry changed or dropped ($FIRST -> $SECOND)"
[ -f "$D/dist/$FIRST" ] || fail "run 2: $FIRST was deleted"
echo "ok: double run keeps $FIRST"

# --- case 3: rotation on a changed styles.css ------------------------------
printf 'body { color: blue; }\n' > "$D/dist/styles.css"
run_manifest "$D"
THIRD=$(manifest_styles "$D")
[ -n "$THIRD" ] && [ "$THIRD" != "$FIRST" ] || fail "rotation: manifest did not rotate ($FIRST -> $THIRD)"
[ -f "$D/dist/$THIRD" ] || fail "rotation: $THIRD missing from dist"
[ ! -e "$D/dist/$FIRST" ] || fail "rotation: stale $FIRST was not removed"
echo "ok: changed styles.css rotates to $THIRD"

# --- case 4: adoption from scratch -----------------------------------------
A=$TMP/adopt
mkdir -p "$A/dist"
printf 'body { color: green; }\n' > "$A/dist/styles.deadbeef.css"
printf 'console.log(2);\n' > "$A/dist/main.abc12xyz.js"
run_manifest "$A"
ADOPTED=$(manifest_styles "$A")
[ "$ADOPTED" = "styles.deadbeef.css" ] || fail "adoption: manifest maps styles.css to $ADOPTED"
[ -f "$A/dist/styles.deadbeef.css" ] || fail "adoption: styles.deadbeef.css was deleted"
echo "ok: pre-hashed stylesheet adopted"

# --- case 5: non-contract names are left alone ------------------------------
printf 'body { color: black; }\n' > "$A/dist/styles.darkmode.css"
run_manifest "$A"
[ -f "$A/dist/styles.darkmode.css" ] || fail "non-contract: styles.darkmode.css was deleted"
[ "$(manifest_styles "$A")" = "styles.deadbeef.css" ] || fail "non-contract: adopted entry changed"
echo "ok: non-contract stylesheet untouched"

echo "build-manifest double-run fixture: all cases passed"
