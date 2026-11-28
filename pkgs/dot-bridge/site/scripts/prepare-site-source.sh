#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 2 || ! $1 =~ ^[a-zA-Z0-9_-]{8,128}$ || $2 != /* ]]; then
  echo "Usage: bash scripts/prepare-site-source.sh PROJECT_ID /absolute/site-checkout" >&2
  exit 2
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
target=$(cd "$2" && pwd -P)
id=$1
[[ $target != "$root" && -d "$target/.git" ]] || {
  echo "Use a separate native Sites checkout" >&2
  exit 2
}
# Validate source, target identity, and existing checkout paths before writes.
for file in worker/index.js scripts/build.sh scripts/validate-artifact.mjs package.json test.mjs; do
  [[ -f "$root/$file" && ! -L "$root/$file" && ! -L "$target/$file" ]] || exit 2
done
for dir in worker scripts .openai; do [[ ! -L "$target/$dir" ]] || exit 2; done
[[ ! -L "$target/.openai/hosting.json" ]] || exit 2
node --input-type=module - "$target" "$id" <<'JS'
import fs from 'node:fs';
const [target,id] = process.argv.slice(2);
const p=target+'/.openai/hosting.json';
if (!fs.existsSync(p)) throw Error('Native checkout must already contain its Site identity');
const m=JSON.parse(fs.readFileSync(p,'utf8'));
if (m.project_id!==id || m.static || m.r2!=='BUCKET' || !Array.isArray(m.capabilities) || !m.capabilities.includes('mcp'))
  throw Error('Native manifest must already match target project, Worker type, BUCKET binding, and MCP capability');
JS
mkdir -p "$target"/{worker,scripts}
for file in worker/index.js scripts/build.sh scripts/validate-artifact.mjs package.json test.mjs; do
  cp "$root/$file" "$target/$file"
done
printf '%s\n' 'Source staged locally. No git push, package, save, or deployment performed.'
