#!/usr/bin/env bash
set -euo pipefail

source_script="$(cd "$(dirname "$0")" && pwd)/just-recipes-check.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/work/scripts" "$tmp/elsewhere"
# Run a copy of the script inside the fixture tree (the build-sh-test.sh
# pattern): the script anchors to its own location (#6957), so the
# copy's tree is the fixture, not the real pkgs/id.
cp "$source_script" "$tmp/work/scripts/just-recipes-check.sh"
script="$tmp/work/scripts/just-recipes-check.sh"
printf '#!%s\n' "$(command -v bash)" >"$tmp/bin/just"
cat >>"$tmp/bin/just" <<'EOF'
cat "$FRESH"
EOF
chmod +x "$tmp/bin/just"
export PATH="$tmp/bin:$PATH" FRESH="$tmp/fresh.json"
cd "$tmp/work"
cat >just-recipes.json <<'EOF'
{"source":"old", "recipes":{"run":{"doc":"run", "private":false, "parameters":[{"name":"arg", "kind":"singular", "default":null, "export":false, "flag":false}], "dependencies":[{"recipe":"dep", "arguments":[["string", "a"]], "star":null}], "body":[["old"]]}}, "aliases":{"r":{"target":"run"}}}
EOF
cp just-recipes.json "$FRESH"
cp just-recipes.json "$tmp/pristine.json"
check_pass() { if ! bash "$script" >"$tmp/output" 2>&1; then
  cat "$tmp/output"
  exit 1
fi; }
check_fail() { if bash "$script" >"$tmp/output" 2>&1; then
  echo "unexpected success: $1" >&2
  exit 1
fi; }
change() { jq "$1" just-recipes.json >"$FRESH"; }

check_pass
change '.source = "new" | .recipes.run.body = [["new"]]'
check_pass
change '.recipes.run.parameters[0].export = true'
check_fail 'parameter export'
change '.recipes.run.parameters[0].flag = true'
check_fail 'parameter flag'
change '.recipes.run.dependencies[0].arguments[0][1] = "b"'
check_fail 'dependency argument'
change '.recipes.run.dependencies[0].star = true'
check_fail 'dependency star'
change '.aliases.r.target = "dep"'
check_fail 'alias target'
printf '{' >"$FRESH"
check_fail 'invalid fresh JSON'
cp just-recipes.json "$FRESH"
printf '{' >just-recipes.json
check_fail 'invalid committed JSON'

# Regression (#6957): the verdict must follow the script's own tree,
# not the caller's CWD. Restore the pristine in-sync fixture, then
# invoke from a foreign directory holding a decoy just-recipes.json
# whose doc differs from the fresh dump: a CWD-relative check would
# fail here; the anchored check passes on the fixture.
cp "$tmp/pristine.json" just-recipes.json
cp "$tmp/pristine.json" "$FRESH"
jq '.recipes.run.doc = "decoy"' just-recipes.json >"$tmp/elsewhere/just-recipes.json"
if ! (cd "$tmp/elsewhere" && bash "$script" >"$tmp/output" 2>&1); then
  cat "$tmp/output"
  echo "unexpected failure: foreign-CWD in-sync fixture" >&2
  exit 1
fi
# The dangerous direction: drift the fixture and give the CWD a decoy
# that matches the fresh dump — a CWD-relative check would certify
# the drifted file; the anchored check must still fail.
jq '.recipes.run.doc = "drifted"' just-recipes.json >"$tmp/drift.json"
mv "$tmp/drift.json" just-recipes.json
cp "$tmp/pristine.json" "$tmp/elsewhere/just-recipes.json"
if (cd "$tmp/elsewhere" && bash "$script" >"$tmp/output" 2>&1); then
  echo "unexpected success: foreign-CWD drifted fixture with matching decoy" >&2
  exit 1
fi
echo 'just-recipes-check tests passed'
