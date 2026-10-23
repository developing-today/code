#!/usr/bin/env bash
# Normalise opencode's config back to the `plugin` / `agent` keys.
#
# Why this exists:
#   OpenChamber rewrites ~/.config/opencode/opencode.jsonc on startup, migrating
#   the v1 `plugin` array to a v2 `plugins` array (its source: "OpenChamber reads
#   both and always writes `plugins` with v2 entries"), and likewise `agent` ->
#   `agents`. That is fine for OpenChamber, which runs opencode 2.x.
#
#   It is NOT fine for the system opencode, which is 1.x. Verified empirically
#   with `opencode debug config` on both builds:
#
#     key          opencode 1.18.19     opencode 2.0.23
#     "plugin"     loads                loads (normalised to `plugins`)
#     "plugins"    IGNORED              loads
#
#   So `plugin` is the only spelling that works on both, and every OpenChamber
#   launch silently breaks plugin loading for the v1 CLI.
#
# This script converts `plugins` -> `plugin` and folds `agents` back into
# `agent`, preserving everything else. It is idempotent and a no-op when the
# file is already correct, so it is safe to run on a path-unit trigger.

set -euo pipefail

CONFIG="${OPENCODE_CONFIG:-$HOME/.config/opencode/opencode.jsonc}"
[ -r "$CONFIG" ] || exit 0

python3 - "$CONFIG" <<'PY'
import json, re, sys, pathlib

path = pathlib.Path(sys.argv[1])
raw = path.read_text()

# Cheap JSONC -> JSON for the membership test only; we never write this back.
probe = re.sub(r'^\s*//.*$', '', raw, flags=re.M)
probe = re.sub(r',(\s*[}\]])', r'\1', probe)
try:
    doc = json.loads(probe)
except Exception:
    sys.exit(0)  # unparseable: leave it alone rather than risk mangling it

changed = False
out = raw

# plugins -> plugin
if 'plugins' in doc and 'plugin' not in doc:
    out, n = re.subn(r'"plugins"(\s*):', r'"plugin"\1:', out, count=1)
    changed = changed or bool(n)

# agents.build -> agent.build, then drop the now-empty agents block
if 'agents' in doc:
    agents = doc.get('agents') or {}
    agent = doc.get('agent') or {}
    merged = {**agents, **agent}
    if merged != agent:
        new_agent = json.dumps(merged, indent=2)
        new_agent = '\n'.join(
            ('  ' + ln if i else ln) for i, ln in enumerate(new_agent.split('\n'))
        )
        out, n = re.subn(
            r'"agent"(\s*):\s*\{.*?\n  \}',
            lambda m: '"agent"' + m.group(1) + ': ' + new_agent,
            out, count=1, flags=re.S,
        )
        changed = changed or bool(n)
    out, n = re.subn(r'\n\s*"agents"\s*:\s*\{.*?\n  \},?', '', out, count=1, flags=re.S)
    changed = changed or bool(n)

if not changed:
    sys.exit(0)

# Never write something we cannot parse.
verify = re.sub(r'^\s*//.*$', '', out, flags=re.M)
verify = re.sub(r',(\s*[}\]])', r'\1', verify)
try:
    json.loads(verify)
except Exception:
    sys.exit(1)

tmp = path.with_suffix(path.suffix + '.normalize-tmp')
tmp.write_text(out)
tmp.replace(path)
print(f"opencode-config-normalize: repaired {path}", file=sys.stderr)
PY
