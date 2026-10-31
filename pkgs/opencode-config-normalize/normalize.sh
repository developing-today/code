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

# agents -> agent: fold the v2 `agents` map into `agent` (existing `agent`
# wins per key). The `agents` block is dropped only once the merged map has
# landed in an `agent` block, and the full-document verification before the
# write below proves no entry was lost; if the splice/insert cannot be
# done, exit 1 with the file left byte-identical — a repair tool must
# never be a data-loss path (#2364).
if 'agents' in doc:
    agents = doc.get('agents') or {}
    has_agent = 'agent' in doc
    agent = doc.get('agent') or {}
    merged = {**agents, **agent}
    if merged != agent:
        new_agent = json.dumps(merged, indent=2)
        new_agent = '\n'.join(
            ('  ' + ln if i else ln) for i, ln in enumerate(new_agent.split('\n'))
        )
        if has_agent:
            out, n = re.subn(
                r'"agent"(\s*):\s*\{.*?\n  \}',
                lambda m: '"agent"' + m.group(1) + ': ' + new_agent,
                out, count=1, flags=re.S,
            )
            if not n:
                sys.exit(1)  # cannot splice: leave the file untouched
            changed = True
            out, n = re.subn(r'\n\s*"agents"\s*:\s*\{.*?\n  \},?', '', out, count=1, flags=re.S)
            if not n:
                sys.exit(1)
        else:
            # No `agent` block to splice into: replace the `agents` block
            # itself with the merged `agent` block, keeping a trailing
            # comma when the `agents` block had one.
            def _insert(m):
                comma = ',' if m.group(0).rstrip().endswith(',') else ''
                return '"agent"' + m.group(1) + ': ' + new_agent + comma
            out, n = re.subn(
                r'"agents"(\s*):\s*\{.*?\n  \},?',
                _insert,
                out, count=1, flags=re.S,
            )
            if not n:
                sys.exit(1)  # unrecognised agents shape: leave untouched
            changed = True
    else:
        # Every agents entry is already present in `agent` verbatim, so
        # dropping the duplicate block cannot lose anything. If the drop
        # shape misses, verification below refuses the write.
        out, n = re.subn(r'\n\s*"agents"\s*:\s*\{.*?\n  \},?', '', out, count=1, flags=re.S)
        changed = changed or bool(n)

if not changed:
    sys.exit(0)

# Never write something we cannot parse — and never write a result whose
# parsed document differs from the intended one. The intended document is
# `doc` with the renames/folds above applied; comparing the whole document
# is what proves no `agents` entry was dropped on the floor (#2364).
verify = re.sub(r'^\s*//.*$', '', out, flags=re.M)
verify = re.sub(r',(\s*[}\]])', r'\1', verify)
try:
    parsed = json.loads(verify)
except Exception:
    sys.exit(1)

expected = dict(doc)
if 'plugins' in expected and 'plugin' not in doc:
    expected['plugin'] = expected.pop('plugins')
if 'agents' in expected:
    merged_agent = {**(expected.get('agents') or {}), **(expected.get('agent') or {})}
    del expected['agents']
    if merged_agent or 'agent' in doc:
        expected['agent'] = merged_agent
if parsed != expected:
    sys.exit(1)

tmp = path.with_suffix(path.suffix + '.normalize-tmp')
tmp.write_text(out)
tmp.replace(path)
print(f"opencode-config-normalize: repaired {path}", file=sys.stderr)
PY
