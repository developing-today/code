#!/usr/bin/env bash
set -euo pipefail
[[ $# == 0 ]] || exit 2
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
fixture=$(mktemp -d)
trap 'rm -r "$fixture"' EXIT
mkdir -p "$fixture"/{.git,.openai}
printf '%s\n' '{"project_id":"fixture-project","r2":"BUCKET","capabilities":["mcp"]}' > "$fixture/.openai/hosting.json"
refuse() {
  if "$@" >/dev/null 2>&1; then
    echo 'Unexpected success' >&2
    exit 1
  fi
}
refuse bash "$root/scripts/prepare-site-source.sh" wrong-project "$fixture"
refuse bash "$root/scripts/prepare-site-source.sh" fixture-project "$fixture" extra
test ! -e "$fixture/worker"
bash "$root/scripts/prepare-site-source.sh" fixture-project "$fixture"
(
  cd "$fixture"
  export SITE_PROJECT_ID=fixture-project
  refuse bash scripts/build.sh extra
  refuse node scripts/validate-artifact.mjs extra
  test ! -e dist
  bash scripts/build.sh
  node scripts/validate-artifact.mjs
  printf '%s' 'dummy secret' > dist/stale.txt
  refuse bash scripts/build.sh
  refuse node scripts/validate-artifact.mjs
  test -f dist/stale.txt
  rm dist/stale.txt
  mv dist/server/index.js worker/fixture-artifact.js
  ln -s ../../worker/fixture-artifact.js dist/server/index.js
  refuse bash scripts/build.sh
  refuse node scripts/validate-artifact.mjs
  rm dist/server/index.js
  bash scripts/build.sh
  node scripts/validate-artifact.mjs
  node test.mjs
)
