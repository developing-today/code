#!/usr/bin/env bash
set -euo pipefail

source_script="$(cd "$(dirname "$0")" && pwd)/just-recipes-root-check.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/work/scripts" "$tmp/elsewhere"
cp "$source_script" "$tmp/work/scripts/just-recipes-root-check.sh"
script="$tmp/work/scripts/just-recipes-root-check.sh"
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
check_usage_fail() {
  local label="$1"
  shift
  local status=0
  if bash "$script" "$@" >"$tmp/output" 2>&1; then
    echo "unexpected success: $label" >&2
    exit 1
  else
    status=$?
  fi
  if [[ $status -ne 2 ]]; then
    cat "$tmp/output"
    echo "unexpected exit status $status (expected 2): $label" >&2
    exit 1
  fi
  if ! grep -q "unexpected argument" "$tmp/output"; then
    cat "$tmp/output"
    echo "missing unexpected-argument message: $label" >&2
    exit 1
  fi
}
change() { jq "$1" just-recipes.json >"$FRESH"; }

check_pass
check_usage_fail 'extra positional argument' extra-arg-test
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

cp "$tmp/pristine.json" just-recipes.json
cp "$tmp/pristine.json" "$FRESH"
jq '.recipes.run.doc = "decoy"' just-recipes.json >"$tmp/elsewhere/just-recipes.json"
if ! (cd "$tmp/elsewhere" && bash "$script" >"$tmp/output" 2>&1); then
  cat "$tmp/output"
  echo "unexpected failure: foreign-CWD in-sync fixture" >&2
  exit 1
fi
jq '.recipes.run.doc = "drifted"' just-recipes.json >"$tmp/drift.json"
mv "$tmp/drift.json" just-recipes.json
cp "$tmp/pristine.json" "$tmp/elsewhere/just-recipes.json"
if (cd "$tmp/elsewhere" && bash "$script" >"$tmp/output" 2>&1); then
  echo "unexpected success: foreign-CWD drifted fixture with matching decoy" >&2
  exit 1
fi
echo 'just-recipes-root-check tests passed'
