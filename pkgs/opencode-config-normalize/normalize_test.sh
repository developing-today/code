#!/usr/bin/env bash
# Regression fixtures for normalize.sh (#2364).
#
# Each case runs the real script against a fixture config in a temp dir
# and asserts the FULL parsed document afterwards (not substrings), the
# exit code, and idempotence (a second run is a byte-identical no-op).
# The agent-only case is the data-loss regression mirror: folding the
# v1  map into  must never drop a definition.
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
# String-aware JSONC strip (mirrors normalize.sh jsonc_to_json, #3179):
# comments and trailing commas are removed only outside string literals.
out = []
i, n = 0, len(raw)
in_str = False
while i < n:
    c = raw[i]
    if in_str:
        out.append(c)
        if c == '\\' and i + 1 < n:
            out.append(raw[i + 1]); i += 2; continue
        if c == '"':
            in_str = False
        i += 1; continue
    if c == '"':
        in_str = True; out.append(c); i += 1; continue
    if c == '/' and raw[i + 1:i + 2] == '/':
        while i < n and raw[i] != '\n':
            i += 1
        continue
    if c == '/' and raw[i + 1:i + 2] == '*':
        end = raw.find('*/', i + 2)
        i = n if end < 0 else end + 2
        out.append(' '); continue
    out.append(c); i += 1
s = ''.join(out)
out = []
i, n = 0, len(s)
in_str = False
while i < n:
    c = s[i]
    if in_str:
        out.append(c)
        if c == '\\' and i + 1 < n:
            out.append(s[i + 1]); i += 2; continue
        if c == '"':
            in_str = False
        i += 1; continue
    if c == '"':
        in_str = True; out.append(c); i += 1; continue
    if c == ',':
        j = i + 1
        while j < n and s[j] in ' \t\r\n':
            j += 1
        if j < n and s[j] in '}]':
            i += 1; continue
    out.append(c); i += 1
print(json.dumps(json.loads(''.join(out)), sort_keys=True))
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
    echo "FAIL $name: exit $got_exit, want $want_exit"
    failures=$((failures + 1))
    return
  fi
  if [ "$want_exit" -ne 0 ]; then
    # Failure contract: the file is left byte-identical.
    after_first="$(sha256sum "$cfg" | cut -d' ' -f1)"
    if [ "$before" != "$after_first" ]; then
      echo "FAIL $name: file modified despite exit $want_exit"
      failures=$((failures + 1))
      return
    fi
    echo "PASS $name"
    return
  fi
  if [ "$expected" = "__UNPARSEABLE__" ]; then
    if [ "$(sha256sum "$cfg" | cut -d' ' -f1)" != "$before" ]; then
      echo "FAIL $name: unparseable file was modified"
      failures=$((failures + 1))
      return
    fi
    echo "PASS $name"
    return
  fi
  local got want
  got="$(parsed_doc "$cfg")"
  want="$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1]), sort_keys=True))' "$expected")"
  if [ "$got" != "$want" ]; then
    echo "FAIL $name: parsed document mismatch"
    echo "  got:  $got"
    echo "  want: $want"
    failures=$((failures + 1))
    return
  fi
  # Idempotence: second run is a byte-identical no-op with exit 0.
  after_first="$(sha256sum "$cfg" | cut -d' ' -f1)"
  OPENCODE_CONFIG="$cfg" bash "$script" 2>/dev/null
  if [ $? -ne 0 ] || [ "$(sha256sum "$cfg" | cut -d' ' -f1)" != "$after_first" ]; then
    echo "FAIL $name: second run was not a byte-identical no-op"
    failures=$((failures + 1))
    return
  fi
  echo "PASS $name"
}

# Case 1: agent-only (2-space pretty) — the mirror of the #2364
# data-loss shape. Folding the v1 `agent` map into `agents` must never
# drop a definition, and the v1 `plugin` list must rename with it.
cat >"$tmp/agent-only.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" },
    "plan": { "model": "anthropic/claude-haiku-4-5" }
  },
  "plugin": ["opencode-plugin-x"]
}
EOF
check agent-only "$tmp/agent-only.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"},"plan":{"model":"anthropic/claude-haiku-4-5"}},"plugins":["opencode-plugin-x"]}' 0

# Case 2: agent-only as the last key (no trailing comma after the block).
cat >"$tmp/agent-last.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check agent-last "$tmp/agent-last.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 3: agent + agents merge — existing agents (v2) wins per key,
# matching the fork's mergeMaps native-wins semantics.
cat >"$tmp/merge.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "build": { "model": "v1/loses" },
    "scout": { "model": "anthropic/claude-haiku-4-5" }
  },
  "agents": {
    "build": { "model": "existing/wins" },
    "plan": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check merge "$tmp/merge.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"existing/wins"},"scout":{"model":"anthropic/claude-haiku-4-5"},"plan":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 4: plugin-only rename, nothing else touched.
cat >"$tmp/plugin-only.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["opencode-plugin-x", "opencode-plugin-y"]
}
EOF
check plugin-only "$tmp/plugin-only.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugins":["opencode-plugin-x","opencode-plugin-y"]}' 0

# Case 5: unparseable file is left untouched, exit 0.
cat >"$tmp/unparseable.fixture" <<'EOF'
{ this is not json at all
EOF
check unparseable "$tmp/unparseable.fixture" '__UNPARSEABLE__' 0
# (exit-0 + byte-identical is asserted inside check via the failure path
# only for non-zero exits; verify the bytes explicitly here too)
cp "$tmp/unparseable.fixture" "$tmp/unparseable.copy"
OPENCODE_CONFIG="$tmp/unparseable.copy" bash "$script" 2>/dev/null
cmp -s "$tmp/unparseable.fixture" "$tmp/unparseable.copy" || {
  echo "FAIL unparseable-bytes"
  failures=$((failures + 1))
}

# Case 6: agents in a 4-space indent with a v1 `agent` block also
# present (#4612) — the locator-based fold splices whatever layout the
# value span has, so this normalises like the 2-space merge (Case 3).
# (Before #4612 the layout-bound regex splice could not recognise this
# shape and the case asserted the fail-closed exit 1 instead.)
cat >"$tmp/unrecognised.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  },
    "agents": {
        "plan": { "model": "anthropic/claude-haiku-4-5" }
    }
}
EOF
check unrecognised "$tmp/unrecognised.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"},"plan":{"model":"anthropic/claude-haiku-4-5"}}}' 0

# Case 7: inline // comments (#3179) — the old probe stripper removed
# only full-line comments, so this file failed to parse and every
# repair was silently skipped. Must repair exactly like its
# comment-free twin.
cat >"$tmp/inline-comments.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json", // schema URL
  "plugin": ["opencode-plugin-x"], // v1 spelling, must rename
  "model": "anthropic/claude-sonnet-4-5" // trailing note
}
EOF
check inline-comments "$tmp/inline-comments.fixture" \
  '{"$schema":"https://opencode.ai/config.json","model":"anthropic/claude-sonnet-4-5","plugins":["opencode-plugin-x"]}' 0

# Case 8: /* */ block comments (#3179), standalone and inline.
cat >"$tmp/block-comments.fixture" <<'EOF'
{
  /* written by a v1-era tool */
  "plugin": ["opencode-plugin-x", /* inline block */ "opencode-plugin-y"]
}
EOF
check block-comments "$tmp/block-comments.fixture" \
  '{"plugins":["opencode-plugin-x","opencode-plugin-y"]}' 0

# Case 9: string lookalikes (#3179) — comment markers and ,} / ,]
# sequences inside string values are data and must survive verbatim,
# while a real trailing comma is still tolerated.
cat >"$tmp/string-lookalikes.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["opencode-plugin-x"],
  "note": "docs at https://example.com/a//b, markers /* not a comment */ and ,} ,] stay",
  "list": [1, 2,]
}
EOF
check string-lookalikes "$tmp/string-lookalikes.fixture" \
  '{"$schema":"https://opencode.ai/config.json","list":[1,2],"note":"docs at https://example.com/a//b, markers /* not a comment */ and ,} ,] stay","plugins":["opencode-plugin-x"]}' 0

# Case 10: unterminated block comment (PR #4312 review) — the stripper
# used to drop everything after the open /* and then repair and write
# the file anyway, leaving a still-invalid config behind. An
# unterminated comment is unparseable: leave the file byte-identical.
cat >"$tmp/unterminated-comment.fixture" <<'EOF'
{
  "plugin": ["opencode-plugin-x"]
}
/* trailing note that never closes
EOF
check unterminated-comment "$tmp/unterminated-comment.fixture" '__UNPARSEABLE__' 0

# Case 11: "plugin": inside comments before the live key (PR #4312
# review) — a raw-text rename would select the comment text first,
# leaving the live key unrepaired. The live key is the one renamed;
# comments are preserved.
cat >"$tmp/comment-plugin-key.fixture" <<'EOF'
{
  /* v1 spelling was "plugin": ["old"] — comment, do not touch */
  // likewise "plugin": in a line comment is not a key
  "plugin": ["opencode-plugin-x"]
}
EOF
check comment-plugin-key "$tmp/comment-plugin-key.fixture" \
  '{"plugins":["opencode-plugin-x"]}' 0

# Case 12: already canonical (v2 spellings only) — the steady state.
# Must be a byte-identical no-op on the FIRST run, not just the second.
cat >"$tmp/canonical.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  },
  "plugins": ["opencode-plugin-x"]
}
EOF
check canonical "$tmp/canonical.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"}},"plugins":["opencode-plugin-x"]}' 0
cp "$tmp/canonical.fixture" "$tmp/canonical.copy"
OPENCODE_CONFIG="$tmp/canonical.copy" bash "$script" 2>/dev/null
cmp -s "$tmp/canonical.fixture" "$tmp/canonical.copy" || {
  echo "FAIL canonical-bytes"
  failures=$((failures + 1))
}

# Case 13: plugin + plugins both present — legacy entries first, then
# native (the fork's encoded.plugins order), and the v1 key is gone.
cat >"$tmp/plugins-merge.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["opencode-plugin-a"],
  "plugins": ["opencode-plugin-b", ["opencode-plugin-c", {"opt": 1}]]
}
EOF
check plugins-merge "$tmp/plugins-merge.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugins":["opencode-plugin-a","opencode-plugin-b",["opencode-plugin-c",{"opt":1}]]}' 0

# Case 14 (#4612, probe B): `agents` inline empty object with a
# standard v1 `agent` block — the old regex splice demanded a
# newline + two-space closing brace, so this exited 1 forever.
cat >"$tmp/inline-agents.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  },
  "agents": {}
}
EOF
check inline-agents "$tmp/inline-agents.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 15 (#4612, probe C): `agents` on a single line with content,
# plus a v1 `agent` block adding a key — merges with exit 0.
cat >"$tmp/singleline-agents.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": { "scout": { "model": "anthropic/claude-haiku-4-5" } },
  "agents": { "build": { "model": "anthropic/claude-sonnet-4-5" } }
}
EOF
check singleline-agents "$tmp/singleline-agents.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"},"scout":{"model":"anthropic/claude-haiku-4-5"}}}' 0

# Case 16 (#4612, probe F): an "agents": lookalike inside a // comment
# above the live keys — the fold must splice the LIVE keys and leave
# the comment byte-identical (asserted explicitly after the check).
cat >"$tmp/comment-agents-key.fixture" <<'EOF'
{
  // example only: "agents": { "ghost": { "model": "comment/lookalike" } }
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "scout": { "model": "anthropic/claude-haiku-4-5" }
  },
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check comment-agents-key "$tmp/comment-agents-key.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"},"scout":{"model":"anthropic/claude-haiku-4-5"}}}' 0
cp "$tmp/comment-agents-key.fixture" "$tmp/comment-agents-key.copy"
OPENCODE_CONFIG="$tmp/comment-agents-key.copy" bash "$script" 2>/dev/null
grep -Fq '// example only: "agents": { "ghost": { "model": "comment/lookalike" } }' "$tmp/comment-agents-key.copy" || {
  echo "FAIL comment-agents-key-bytes: comment line not byte-identical"
  failures=$((failures + 1))
}

# Case 17 (#4612, probe E): fully duplicated entries (merged ==
# agents) with the v1 `agent` block on ONE line — the old deletion
# regex missed this layout and silently exited 0 with the v1 block
# left in place. The block must actually be removed.
cat >"$tmp/duplicate-singleline.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agent": { "build": { "model": "anthropic/claude-sonnet-4-5" } },
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check duplicate-singleline "$tmp/duplicate-singleline.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"}}}' 0
cp "$tmp/duplicate-singleline.fixture" "$tmp/duplicate-singleline.copy"
OPENCODE_CONFIG="$tmp/duplicate-singleline.copy" bash "$script" 2>/dev/null
grep -Fq '"agent":' "$tmp/duplicate-singleline.copy" && {
  echo "FAIL duplicate-singleline-bytes: v1 agent key still present"
  failures=$((failures + 1))
}

# Case 18 (#4612): duplicated entries with the v1 `agent` block as
# the LAST entry (no trailing comma) — deletion must remove the
# preceding separator comma and leave a valid single `agents` block.
cat >"$tmp/duplicate-last.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  },
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check duplicate-last "$tmp/duplicate-last.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 19: plugin + plugins both present with comments inside BOTH
# arrays (#4639) — the merge is a textual element splice, so every
# comment survives byte-identical instead of the arrays being
# re-serialised.
cat >"$tmp/plugins-merge-comments.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  // file-level note
  "plugin": [
    /* legacy note */
    "opencode-plugin-a"
  ],
  "plugins": [
    // native note
    "opencode-plugin-b"
    // trailing native
  ]
}
EOF
check plugins-merge-comments "$tmp/plugins-merge-comments.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugins":["opencode-plugin-a","opencode-plugin-b"]}' 0
for needle in '/* legacy note */' '// native note' '// trailing native' '// file-level note'; do
  grep -qF "$needle" "$tmp/plugins-merge-comments.jsonc" || {
    echo "FAIL plugins-merge-comments: lost comment: $needle"
    failures=$((failures + 1))
  }
done

# Case 20: a // line comment trailing the last legacy element (#4639
# boundary) — the joining comma would be swallowed by the comment, so
# the merge fails closed: exit 1 with the file byte-identical.
cat >"$tmp/plugins-merge-trailing-comment.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": [
    "opencode-plugin-a" // trailing legacy note
  ],
  "plugins": [
    "opencode-plugin-b"
  ]
}
EOF
check plugins-merge-trailing-comment "$tmp/plugins-merge-trailing-comment.fixture" '' 1

# Case 21 (Codex P2 on PR #4644): a trailing comma before a trailing
# block comment is still the legacy array's element separator. Remove
# that comma while preserving the comment, then perform the merge.
cat >"$tmp/plugins-merge-trailing-block-comment.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["opencode-plugin-a", /* keep trailing block */],
  "plugins": ["opencode-plugin-b"]
}
EOF
check plugins-merge-trailing-block-comment "$tmp/plugins-merge-trailing-block-comment.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugins":["opencode-plugin-a","opencode-plugin-b"]}' 0
grep -qF '/* keep trailing block */' "$tmp/plugins-merge-trailing-block-comment.jsonc" || {
  echo "FAIL plugins-merge-trailing-block-comment: lost trailing block comment"
  failures=$((failures + 1))
}

# Case 22 (Codex P2 on PR #4644): legacy array holds only a comment,
# e.g. [/* disabled */] — no joining comma is emitted, so the merge
# succeeds and the comment survives instead of failing closed.
cat >"$tmp/plugins-merge-comment-only-legacy.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": [/* disabled */],
  "plugins": ["opencode-plugin-b"]
}
EOF
check plugins-merge-comment-only-legacy "$tmp/plugins-merge-comment-only-legacy.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugins":["opencode-plugin-b"]}' 0
grep -qF '/* disabled */' "$tmp/plugins-merge-comment-only-legacy.jsonc" || {
  echo "FAIL plugins-merge-comment-only-legacy: lost legacy comment"
  failures=$((failures + 1))
}

# Case 23 (Codex P2 on PR #4644): multiline block comment with a URL on
# its final line inside the legacy array — the // in the URL is inside
# /* */, not a live line comment, so the merge succeeds.
cat >"$tmp/plugins-merge-url-comment.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["opencode-plugin-a" /* note
https://example.com/docs */],
  "plugins": ["opencode-plugin-b"]
}
EOF
check plugins-merge-url-comment "$tmp/plugins-merge-url-comment.fixture" \
  '{"$schema":"https://opencode.ai/config.json","plugins":["opencode-plugin-a","opencode-plugin-b"]}' 0
grep -qF 'https://example.com/docs */' "$tmp/plugins-merge-url-comment.jsonc" || {
  echo "FAIL plugins-merge-url-comment: lost block comment"
  failures=$((failures + 1))
}

# Case: agent as the last entry, with an inline comment between the
# preceding entry's comma and the agent key (PR #4625 Codex P2). The
# backward separator scan in delete_top_level_entry must skip the
# comment; the fold must succeed and stay idempotent.
cat >"$tmp/comment-before-agent-last.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agents": {
    "plan": { "model": "anthropic/claude-sonnet-4-5" }
  }, // native agents live here
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check comment-before-agent-last "$tmp/comment-before-agent-last.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"},"plan":{"model":"anthropic/claude-sonnet-4-5"}}}' 0

# Case 24 (#4770): a block comment containing a literal /* before the
# final legacy `agent` entry — the backward separator scan must match
# the comment's actual opening delimiter, not the inner literal, so
# the fold succeeds and the comment text is preserved byte-identical.
cat >"$tmp/block-comment-inner-delimiter.fixture" <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "agents": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  },
  /* example syntax: /* text */
  "agent": {
    "build": { "model": "anthropic/claude-sonnet-4-5" }
  }
}
EOF
check block-comment-inner-delimiter "$tmp/block-comment-inner-delimiter.fixture" \
  '{"$schema":"https://opencode.ai/config.json","agents":{"build":{"model":"anthropic/claude-sonnet-4-5"}}}' 0
grep -qF '/* example syntax: /* text */' "$tmp/block-comment-inner-delimiter.jsonc" || {
  echo "FAIL block-comment-inner-delimiter: comment text not preserved"
  failures=$((failures + 1))
}

# Case 7: a repair preserves the config's original permission bits.
# (Input is the v1 `plugin` spelling — the canonical direction since
# the v2 rename — so the run performs a real rewrite.)
cat >"$tmp/mode.fixture" <<'EOF'
{
  "plugin": ["opencode-plugin-x"]
}
EOF
cp "$tmp/mode.fixture" "$tmp/mode.jsonc"
chmod 0600 "$tmp/mode.jsonc"
OPENCODE_CONFIG="$tmp/mode.jsonc" bash "$script" 2>/dev/null
if [ "$?" -ne 0 ] || [ "$(stat -c '%a' "$tmp/mode.jsonc")" != "600" ] ||
  [ "$(parsed_doc "$tmp/mode.jsonc")" != '{"plugins": ["opencode-plugin-x"]}' ]; then
  echo "FAIL mode-preserved: repair must preserve 0600 and normalize plugins"
  failures=$((failures + 1))
else
  echo "PASS mode-preserved"
fi

# Case 8: a symlinked config is refused before writes; keep both the link
# target and its contents unchanged.
cat >"$tmp/symlink-target.jsonc" <<'EOF'
{
  "plugin": ["opencode-plugin-x"]
}
EOF
ln -s symlink-target.jsonc "$tmp/symlink.jsonc"
link_before="$(readlink "$tmp/symlink.jsonc")"
target_before="$(sha256sum "$tmp/symlink-target.jsonc" | cut -d' ' -f1)"
OPENCODE_CONFIG="$tmp/symlink.jsonc" bash "$script" 2>"$tmp/symlink.stderr"
symlink_exit=$?
target_after="$(sha256sum "$tmp/symlink-target.jsonc" | cut -d' ' -f1)"
if [ "$symlink_exit" -eq 0 ] || [ ! -L "$tmp/symlink.jsonc" ] ||
  [ "$(readlink "$tmp/symlink.jsonc")" != "$link_before" ] ||
  [ "$target_before" != "$target_after" ] ||
  ! grep -q 'refusing symlinked config' "$tmp/symlink.stderr"; then
  echo "FAIL symlink-refused: expected a clear refusal with link and target unchanged"
  failures=$((failures + 1))
else
  echo "PASS symlink-refused"
fi

if [ "$failures" -ne 0 ]; then
  echo "normalize_test: $failures failure(s)"
  exit 1
fi

echo "normalize_test: all cases passed"
