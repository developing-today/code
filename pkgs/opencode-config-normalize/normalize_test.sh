#!/usr/bin/env bash
# Regression fixtures for normalize.sh (#2364).
#
# Each case runs the real script against a fixture config in a temp dir
# and asserts the FULL parsed document afterwards (not substrings), the
# exit code, and idempotence (a second run is a byte-identical no-op).
# The agents-only case is the data-loss regression: the script used to
# drop every agent definition while reporting "repaired".
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/normalize.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0

# parsed_doc <file> — print the JSONC-stripped parsed document as
# canonical JSON (sorted keys) for comparison.
parsed_doc() {
  python3 - "$1" <<'PY'
import json, re, sys, pathlib
raw = pathlib.Path(sys.argv[1]).read_text()
probe = re.sub(r'^\s*//.*$', '', raw, flags=re.M)
probe = re.sub(r',(\s*[}\]])', r'\1', probe)
print(json.dumps(json.loads(probe), sort_keys=True))
PY
}

# check <name> <fixture> <expected-json> <expected-exit>
# Runs the script on a copy of <fixture>, asserts exit code and full
# parsed document, then asserts a second run changes nothing.
check() {
  local name="$1" fixture="$2" expected="$3" want_exit="$4"
  local cfg="$tmp/$name.jsonc"
  cp "$fixture" "$cfg"
  local before after_first
  before="$(sha256sum "$cfg" | cut -d' ' -f1)"
  OPENCODE_CONFIG="$cfg" bash "$script" 2>/dev/null
  local got_exit=$?
  if [ "$got_exit" -ne "$want_exit" ]; then
    echo "FAIL $name: exit $got_exit, want $want_exit"; failures=$((failures+1)); return
  fi
  if [ "$want_exit" -ne 0 ]; then
    # Failure contract: the file is left byte-identical.
    after_first="$(sha256sum "$cfg" | cut -d' ' -f1)"
    if [ "$before" != "$after_first" ]; then
      echo "FAIL $name: file modified despite exit $want_exit"; failures=$((failures+1)); return
    fi
    echo "PASS $name"; return
  fi
  if [ "$expected" = "__UNPARSEABLE__" ]; then
    if [ "$(sha256sum "$cfg" | cut -d' ' -f1)" != "$before" ]; then
      echo "FAIL $name: unparseable file was modified"; failures=$((failures+1)); return
    fi
    echo "PASS $name"; return
  fi
  local got want
  got="$(parsed_doc "$cfg")"
  want="$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1]), sort_keys=True))' "$expected")"
  if [ "$got" != "$want" ]; then
    echo "FAIL $name: parsed document mismatch"
    echo "  got:  $got"
    echo "  want: $want"
    failures=$((failures+1)); return
  fi
  # Idempotence: second run is a byte-identical no-op with exit 0.
  after_first="$(sha256sum "$cfg" | cut -d' ' -f1)"
  OPENCODE_CONFIG="$cfg" bash "$script" 2>/dev/null
  if [ $? -ne 0 ] || [ "$(sha256sum "$cfg" | cut -d' ' -f1)" != "$after_first" ]; then
    echo "FAIL $name: second run was not a byte-identical no-op"; failures=$((failures+1)); return
  fi
  echo "PASS $name"
}

# Case 1: agents-only (2-space pretty) — the #2364 data-loss shape.
cat > "$tmp/agents-only.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" },
    "plan": { "model": "anthropic/claude-haiku-4-5" }
  },
  "plugins": ["opencode-plugin-x"]
}
EOF
check agents-only "$tmp/agents-only.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agent":{"build":{"model":"anthropic/claude-sonnet-4-5"},"plan":{"model":"anthropic/claude-haiku-4-5"}},"plugin":["opencode-plugin-x"]}' 0

# Case 2: agents-only as the last key (no trailing comma after the block).
cat > "$tmp/agents-last.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check agents-last "$tmp/agents-last.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agent":{"build":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 3: agent + agents merge — existing agent wins per key.
cat > "$tmp/merge.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "build": { "model": "existing/wins" },
    "scout": { "model": "anthropic/claude-haiku-4-5" }
  },
  "agents": {
    "build": { "model": "v2/loses" },
    "plan": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check merge "$tmp/merge.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agent":{"build":{"model":"existing/wins"},"scout":{"model":"anthropic/claude-haiku-4-5"},"plan":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 4: plugins-only rename, nothing else touched.
cat > "$tmp/plugins-only.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugins": ["opencode-plugin-x", "opencode-plugin-y"]
}
EOF
check plugins-only "$tmp/plugins-only.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugin":["opencode-plugin-x","opencode-plugin-y"]}' 0

# Case 5: unparseable file is left untouched, exit 0.
cat > "$tmp/unparseable.fixture" <<'EOF'
{ this is not json at all
EOF
check unparseable "$tmp/unparseable.fixture" '__UNPARSEABLE__' 0
# (exit-0 + byte-identical is asserted inside check via the failure path
# only for non-zero exits; verify the bytes explicitly here too)
cp "$tmp/unparseable.fixture" "$tmp/unparseable.copy"
OPENCODE_CONFIG="$tmp/unparseable.copy" bash "$script" 2>/dev/null
cmp -s "$tmp/unparseable.fixture" "$tmp/unparseable.copy" || { echo "FAIL unparseable-bytes"; failures=$((failures+1)); }

# Case 6: agents shape the splice cannot recognise (4-space indent) —
# the file must be left byte-identical with exit 1, never stripped.
cat > "$tmp/unrecognised.fixture" <<'EOF'
{
    "$schema": "https://opencode.ai/config.json",
    "agents": {
        "build": { "model": "anthropic/claude-sonnet-4-5" }
    }
}
EOF
check unrecognised "$tmp/unrecognised.fixture" '' 1

if [ "$failures" -ne 0 ]; then
  echo "normalize_test: $failures failure(s)"
  exit 1
fi
echo "normalize_test: all cases passed"
