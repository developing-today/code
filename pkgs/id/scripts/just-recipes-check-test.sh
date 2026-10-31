#!/usr/bin/env bash
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/just-recipes-check.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin" "$tmp/work"
printf '#!%s\n' "$(command -v bash)" > "$tmp/bin/just"
cat >> "$tmp/bin/just" <<'EOF'
cat "$FRESH"
EOF
chmod +x "$tmp/bin/just"
export PATH="$tmp/bin:$PATH" FRESH="$tmp/fresh.json"
cd "$tmp/work"
cat > just-recipes.json <<'EOF'
{"source":"old", "recipes":{"run":{"doc":"run", "private":false, "parameters":[{"name":"arg", "kind":"singular", "default":null, "export":false, "flag":false}], "dependencies":[{"recipe":"dep", "arguments":[["string", "a"]], "star":null}], "body":[["old"]]}}, "aliases":{"r":{"target":"run"}}}
EOF
cp just-recipes.json "$FRESH"
check_pass() { if ! bash "$script" > "$tmp/output" 2>&1; then cat "$tmp/output"; exit 1; fi; }
check_fail() { if bash "$script" > "$tmp/output" 2>&1; then echo "unexpected success: $1" >&2; exit 1; fi; }
change() { jq "$1" just-recipes.json > "$FRESH"; }

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
printf '{' > "$FRESH"
check_fail 'invalid fresh JSON'
cp just-recipes.json "$FRESH"
printf '{' > just-recipes.json
check_fail 'invalid committed JSON'
echo 'just-recipes-check tests passed'
