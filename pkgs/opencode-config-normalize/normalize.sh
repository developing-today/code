#!/usr/bin/env bash
# Normalise opencode's config to the canonical v2 `plugins` / `agents` keys.
#
# Why this exists:
#   Every opencode consumer on this machine is the 2.x fork — the CLI and
#   the standing server are both v2.0.30+6fb3bac (verified with
#   `opencode --version` in both the per-user and the system profile;
#   the 1.x CLI this file used to protect against is gone). In the fork,
#   `plugins` / `agents` are the canonical spellings: packages/core/src/
#   config/normalize.ts decodes the v1 `plugin` / `agent` keys as LEGACY
#   and migrates them into `plugins` / `agents`, and `opencode debug
#   config` presents the resolved document with the v2 keys. OpenChamber
#   likewise reads both spellings but only ever WRITES native v2
#   (packages/web/server/lib/opencode/config-v2.js: "Everything
#   OpenChamber writes is native v2"; its managed-plugin writer folds a
#   legacy `plugin` key into `plugins` and deletes it).
#
#   This script used to convert the other way (v2 -> v1) because the
#   system opencode was 1.x and ignored `plugins` entirely, so every
#   OpenChamber launch silently broke plugin loading for the CLI and the
#   path unit converted the file back — the two writers flapped the keys
#   on every launch. With 1.x retired, v1 is the odd one out: converge
#   the file to v2 once, keep converging any v1 spelling that lands in
#   it later (hand edits, older tools), and the flap has nothing to
#   fight over.
#
# Scope: only the `plugin` / `agent` families are normalised — they are
# the keys another live writer (OpenChamber) also rewrites. Other v1
# spellings the fork still decodes (provider, command, permission, ...)
# are written by other fleet tooling that reads them back in that
# spelling, and are deliberately left alone.
#
# The conversion renames `plugin` -> `plugins` and folds `agent` into
# `agents` (existing `agents` wins per key — the fork's mergeMaps
# native-wins semantics), preserving everything else including comments.
# It is idempotent and a no-op when the file is already canonical, so it
# is safe to run on a path-unit trigger.

set -euo pipefail

CONFIG="${OPENCODE_CONFIG:-$HOME/.config/opencode/opencode.jsonc}"
if [ -L "$CONFIG" ]; then
  printf 'opencode-config-normalize: refusing symlinked config %s\n' "$CONFIG" >&2
  exit 1
fi
[ -r "$CONFIG" ] || exit 0

python3 - "$CONFIG" <<'PY'
import json, os, pathlib, re, stat, sys, tempfile

path = pathlib.Path(sys.argv[1])
if path.is_symlink():
    print(f'opencode-config-normalize: refusing symlinked config {path}', file=sys.stderr)
    sys.exit(1)
original_mode = stat.S_IMODE(path.stat().st_mode)
raw = path.read_text()


def jsonc_to_json(text):
    # String-aware JSONC -> JSON (#3179): strip // line comments and
    # /* */ block comments plus trailing commas, but never touch the
    # contents of string literals — "//", "/*", ",}" and ",]" inside a
    # string value are data, not comments or trailing commas. Used for
    # the membership probe and the pre-write verification only; the
    # repaired output is still spliced from the original text, so
    # comments in the user's file are preserved.
    out = []
    i, n = 0, len(text)
    in_str = False
    while i < n:
        c = text[i]
        if in_str:
            out.append(c)
            if c == '\\' and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
            continue
        if c == '/' and text[i + 1:i + 2] == '/':
            while i < n and text[i] != '\n':
                i += 1
            continue
        if c == '/' and text[i + 1:i + 2] == '*':
            end = text.find('*/', i + 2)
            if end < 0:
                raise ValueError('unterminated block comment')
            i = end + 2
            out.append(' ')
            continue
        out.append(c)
        i += 1
    s = ''.join(out)
    out = []
    i, n = 0, len(s)
    in_str = False
    while i < n:
        c = s[i]
        if in_str:
            out.append(c)
            if c == '\\' and i + 1 < n:
                out.append(s[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
            continue
        if c == ',':
            j = i + 1
            while j < n and s[j] in ' \t\r\n':
                j += 1
            if j < n and s[j] in '}]':
                i += 1
                continue  # trailing comma: drop it
        out.append(c)
        i += 1
    return ''.join(out)

# JSONC -> JSON for the membership test only; we never write this back.
try:
    doc = json.loads(jsonc_to_json(raw))
except Exception:
    sys.exit(0)  # unparseable: leave it alone rather than risk mangling it
if not isinstance(doc, dict):
    sys.exit(0)  # a config document is an object; anything else is not ours

changed = False
out = raw

def find_top_level_key(text, key):
    # Index of the opening quote of the live top-level "key": token in
    # text, skipping string contents and // /* */ comments (PR #4312
    # review): a "key": lookalike inside a comment or a nested object
    # must never be selected for a splice. Returns -1 when absent.
    i, n = 0, len(text)
    depth = 0
    in_str = False
    str_start = -1
    while i < n:
        c = text[i]
        if in_str:
            if c == '\\' and i + 1 < n:
                i += 2
                continue
            if c == '"':
                in_str = False
                if depth == 1 and text[str_start + 1:i] == key:
                    j = i + 1
                    while j < n and text[j] in ' \t\r\n':
                        j += 1
                    if j < n and text[j] == ':':
                        return str_start
            i += 1
            continue
        if c == '"':
            in_str = True
            str_start = i
            i += 1
            continue
        if c == '/' and text[i + 1:i + 2] == '/':
            while i < n and text[i] != '\n':
                i += 1
            continue
        if c == '/' and text[i + 1:i + 2] == '*':
            end = text.find('*/', i + 2)
            if end < 0:
                return -1
            i = end + 2
            continue
        if c in '{[':
            depth += 1
        elif c in '}]':
            depth -= 1
        i += 1
    return -1


def find_value_span(text, key):
    # (start, end) of the VALUE of the live top-level "key" entry in
    # text — start is the first value character, end is just past the
    # last one. Bracket- and string/comment-aware. Returns None when
    # the key or a well-formed value cannot be located.
    at = find_top_level_key(text, key)
    if at < 0:
        return None
    i = text.index(':', at) + 1
    n = len(text)
    while i < n and text[i] in ' \t\r\n':
        i += 1
    start = i
    if i >= n:
        return None
    if text[i] in '{[':
        stack = []
        in_str = False
        while i < n:
            c = text[i]
            if in_str:
                if c == '\\' and i + 1 < n:
                    i += 2
                    continue
                if c == '"':
                    in_str = False
                i += 1
                continue
            if c == '"':
                in_str = True
                i += 1
                continue
            if c == '/' and text[i + 1:i + 2] == '/':
                while i < n and text[i] != '\n':
                    i += 1
                continue
            if c == '/' and text[i + 1:i + 2] == '*':
                end = text.find('*/', i + 2)
                if end < 0:
                    return None
                i = end + 2
                continue
            if c in '{[':
                stack.append(c)
            elif c in '}]':
                if not stack:
                    return None
                stack.pop()
                if not stack:
                    return (start, i + 1)
            i += 1
        return None
    if text[i] == '"':
        i += 1
        while i < n:
            if text[i] == '\\' and i + 1 < n:
                i += 2
                continue
            if text[i] == '"':
                return (start, i + 1)
            i += 1
        return None
    # Scalar (number / true / false / null): runs to the next , or }.
    while i < n and text[i] not in ',}':
        i += 1
    return (start, i)


def _line_comment_start(text, start, end):
    # Index of the '//' opening a line comment within text[start:end],
    # skipping string contents, or -1 when the segment holds none.
    i = start
    in_str = False
    while i < end:
        c = text[i]
        if in_str:
            if c == '\\' and i + 1 < end:
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            i += 1
            continue
        if c == '/' and i + 1 < end and text[i + 1] == '/':
            return i
        i += 1
    return -1


def _block_comment_start_ending_at(text, k):
    # Index of the '/*' opening the block comment whose closing '*/'
    # ends exactly at k (text[k-2:k] == '*/'), found by a forward
    # lexical scan so a literal /* inside the comment body is never
    # mistaken for the opener (#4770): a plain backward rfind('/*')
    # selects the inner literal in e.g. /* example syntax: /* text */
    # and the separator scan then fails closed on a previously
    # supported layout. String contents and // line comments never
    # open a block comment. Returns -1 when no comment closes
    # exactly at k.
    opener = -1
    cur_open = -1
    in_str = False
    in_line = False
    i, n = 0, len(text)
    while i < k:
        c = text[i]
        if in_line:
            if c == '\n':
                in_line = False
            i += 1
            continue
        if cur_open >= 0:
            if c == '*' and text[i + 1:i + 2] == '/':
                if i + 2 == k:
                    opener = cur_open
                cur_open = -1
                i += 2
                continue
            i += 1
            continue
        if in_str:
            if c == '\\' and i + 1 < n:
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            i += 1
            continue
        if c == '/' and text[i + 1:i + 2] == '/':
            in_line = True
            i += 2
            continue
        if c == '/' and text[i + 1:i + 2] == '*':
            cur_open = i
            i += 2
            continue
        i += 1
    return opener


def delete_top_level_entry(text, key):
    # Remove the live top-level "key": value entry, with its comma.
    # Returns the new text, or None when the entry cannot be located.
    at = find_top_level_key(text, key)
    span = find_value_span(text, key)
    if at < 0 or span is None:
        return None
    _vs, ve = span
    j = ve
    while j < len(text) and text[j] in ' \t\r':
        j += 1
    if j < len(text) and text[j] == ',':
        # Entry separator follows: drop entry + comma + the newline and
        # indentation that trail it, so no blank line is left behind.
        j += 1
        while j < len(text) and text[j] in ' \t':
            j += 1
        if j < len(text) and text[j] == '\n':
            j += 1
            while j < len(text) and text[j] in ' \t':
                j += 1
        return text[:at] + text[j:]
    # Last entry in the object: the separator comma precedes the key,
    # with only whitespace and comments between it and the key. Skip
    # both while scanning back (PR #4625 review): an inline // note
    # after the previous entry, or a /* */ block, must not make a
    # previously supported commented layout un-deletable. The skipped
    # segment is preserved in the output — a repair tool never
    # deletes a user's comment (#2364) — only the comma and the entry
    # go. Anything else in the gap still returns None, fail-closed.
    k = at
    while True:
        while k > 0 and text[k - 1] in ' \t\r\n':
            k -= 1
        if k > 0 and text[k - 1] == ',':
            return text[:k - 1] + text[k:at] + text[ve:]
        if k >= 2 and text[k - 2:k] == '*/':
            start = _block_comment_start_ending_at(text, k)
            if start < 0:
                return None
            k = start
            continue
        line_start = text.rfind('\n', 0, k) + 1
        comment_at = _line_comment_start(text, line_start, k)
        if comment_at < 0 or comment_at >= k:
            return None
        k = comment_at


def line_comment_at_end(fragment):
    # True when fragment ends inside a // line comment: a joining
    # comma appended after it would be swallowed by the comment.
    # String- and block-comment-aware so // inside a string value or a
    # /* */ comment does not misfire (#3179). The whole fragment is
    # scanned while tracking block-comment state (Codex P2 on PR
    # #4644): a multiline block comment whose final line holds a URL
    # (e.g. "a" /* note\nhttps://example.com */) must not have its
    # // mistaken for a live line comment.
    in_str = False
    in_block = False
    in_line = False
    i, n = 0, len(fragment)
    while i < n:
        c = fragment[i]
        if in_line:
            if c == '\n':
                in_line = False
            i += 1
            continue
        if in_block:
            if c == '*' and fragment[i + 1:i + 2] == '/':
                in_block = False
                i += 2
                continue
            i += 1
            continue
        if in_str:
            if c == '\\' and i + 1 < n:
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
        elif c == '/' and fragment[i + 1:i + 2] == '/':
            in_line = True
            i += 2
            continue
        elif c == '/' and fragment[i + 1:i + 2] == '*':
            in_block = True
            i += 2
            continue
        i += 1
    if in_block:
        return True  # unterminated block comment: fail closed
    return in_line


def strip_trailing_element_comma(fragment):
    # Remove the trailing element-separator comma (the one before the
    # closing bracket), skipping over trailing comments, so a legacy
    # array like ["a", /* keep */] splices without a doubled comma
    # (Codex P2 on PR #4644: the naive endswith(',') check missed the
    # comma hidden behind the block comment and the merge wrongly
    # failed closed on input the old code repaired). Only a comma
    # followed solely by whitespace/comments is removed; a comma inside
    # a string, a comment, or a nested array/object is never touched.
    # Returns the fragment unchanged when there is no trailing comma.
    last_comma = -1
    i = 0
    n = len(fragment)
    depth = 0
    in_str = False
    in_line = False
    in_block = False
    while i < n:
        c = fragment[i]
        if in_line:
            if c == '\n':
                in_line = False
            i += 1
            continue
        if in_block:
            if c == '*' and fragment[i + 1:i + 2] == '/':
                in_block = False
                i += 2
                continue
            i += 1
            continue
        if in_str:
            if c == '\\' and i + 1 < n:
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
        elif c == '/' and fragment[i + 1:i + 2] == '/':
            in_line = True
            i += 2
            continue
        elif c == '/' and fragment[i + 1:i + 2] == '*':
            in_block = True
            i += 2
            continue
        elif c in '[{':
            depth += 1
        elif c in ']}':
            depth -= 1
        elif c == ',' and depth == 0:
            last_comma = i
        i += 1
    if last_comma < 0:
        return fragment
    tail = fragment[last_comma + 1:]
    k = 0
    m = len(tail)
    while k < m:
        if tail[k] in ' \t\r\n':
            k += 1
            continue
        if tail.startswith('//', k):
            nl = tail.find('\n', k)
            k = m if nl < 0 else nl + 1
            continue
        if tail.startswith('/*', k):
            end = tail.find('*/', k + 2)
            if end < 0:
                return fragment  # unterminated: leave alone, fail closed
            k = end + 2
            continue
        return fragment  # real content after the comma: not trailing
    return fragment[:last_comma] + tail


def splice_plugin_merge(text, legacy_has_elements=True):
    # Both-present plugin+plugins merge (#4639) as a textual element
    # splice: the legacy elements move into the `plugins` array (legacy
    # first, native appended — the fork's encoded.plugins order) and the
    # v1 `plugin` entry is dropped, all without re-serialising, so
    # comments inside both arrays survive byte-identical. Returns the new
    # text, or None when the splice cannot be done comment-safely (e.g. a
    # // line comment trailing the last legacy element would swallow the
    # joining comma); the caller then exits 1 with the file
    # byte-identical — a repair tool must never be a data-loss path
    # (#2364). legacy_has_elements reports whether the legacy array
    # holds actual elements (vs comments only): a comment-only legacy
    # array needs no joining comma, and emitting one would be a leading
    # syntax error (Codex P2 on PR #4644: [/* disabled */] + ["b"]
    # wrongly failed closed on input the old code repaired).
    legacy_span = find_value_span(text, 'plugin')
    native_span = find_value_span(text, 'plugins')
    if legacy_span is None or native_span is None:
        return None
    lvs, lve = legacy_span
    nvs, nve = native_span
    legacy_text = text[lvs:lve]
    native_text = text[nvs:nve]
    if not (legacy_text.startswith('[') and legacy_text.endswith(']')):
        return None
    if not (native_text.startswith('[') and native_text.endswith(']')):
        return None
    legacy_inner = legacy_text[1:-1].strip()
    native_inner = native_text[1:-1].strip()
    if legacy_inner and native_inner:
        if legacy_has_elements:
            core = strip_trailing_element_comma(legacy_inner).rstrip()
            if line_comment_at_end(core):
                return None
            inner = core + ',\n' + native_inner
        else:
            inner = legacy_inner + '\n' + native_inner
    elif legacy_inner:
        inner = legacy_inner
    else:
        inner = None
    if inner is None:
        merged = native_text  # nothing to move: keep native bytes verbatim
    elif line_comment_at_end(inner):
        # A // comment runs to end of line: the closing bracket goes on
        # its own line so it is not swallowed by the comment.
        merged = '[' + inner + '\n]'
    else:
        merged = '[' + inner + ']'
    text = text[:nvs] + merged + text[nve:]
    return delete_top_level_entry(text, 'plugin')


# plugin -> plugins. When `plugins` is absent this is a bare key rename
# (array contents are byte-preserved, comments included). When both
# spellings are present, merge the way the fork does — legacy entries
# first, native entries appended (normalize.ts: encoded.plugins =
# [...legacyPlugins, ...nativePlugins]) — then drop the v1 entry. The
# merge is a textual element splice, not a re-serialise, so comments
# inside both arrays survive; shapes that cannot be spliced
# comment-safely fail closed with exit 1 and the file byte-identical
# (#4639). A raw
# text search can hit "plugin": inside an earlier comment and leave the
# live key unrepaired (PR #4312 review), so all splices go through the
# string/comment-aware locators, and any splice that cannot be done
# exits 1 with the file byte-identical: a repair tool must never be a
# data-loss path (#2364).
if 'plugin' in doc:
    if 'plugins' not in doc:
        at = find_top_level_key(out, 'plugin')
        if at < 0:
            sys.exit(1)  # live key not locatable: leave the file untouched
        out = out[:at] + '"plugins"' + out[at + len('"plugin"'):]
        changed = True
    else:
        legacy = doc.get('plugin')
        native = doc.get('plugins')
        if not isinstance(legacy, list) or not isinstance(native, list):
            sys.exit(1)  # unmergeable shapes: leave the file untouched
        out = splice_plugin_merge(out, bool(legacy))
        if out is None:
            sys.exit(1)  # not spliceable comment-safely: fail closed
        changed = True

# agent -> agents: fold the v1 `agent` map into `agents` (existing
# `agents` wins per key — the fork's mergeMaps native-wins semantics).
# When `agents` is absent this is again a bare key rename, so a large
# hand-maintained agent block is byte-preserved instead of being
# re-serialised. When both are present, the merged map lands in the
# `agents` block and the `agent` block is dropped only afterwards; the
# full-document verification before the write below proves no entry was
# lost, and any splice that cannot be done exits 1 with the file left
# byte-identical (#2364).
if 'agent' in doc:
    agent = doc.get('agent') or {}
    has_agents = 'agents' in doc
    agents = doc.get('agents') or {}
    if not isinstance(agent, dict) or not isinstance(agents, dict):
        sys.exit(1)  # unmergeable shapes: leave the file untouched
    merged = {**agent, **agents}
    if not has_agents:
        at = find_top_level_key(out, 'agent')
        if at < 0:
            sys.exit(1)  # live key not locatable: leave the file untouched
        out = out[:at] + '"agents"' + out[at + len('"agent"'):]
        changed = True
    elif merged != agents:
        # Splice the merged map into the live `agents` value span via
        # the string/comment-aware locators (#4612): the old regex
        # splice demanded a newline + two-space closing brace and took
        # the first textual match, so inline, single-line, other-indent
        # and comment-lookalike documents exited 1 or never converged.
        span = find_value_span(out, 'agents')
        if span is None:
            sys.exit(1)  # cannot splice: leave the file untouched
        vs, ve = span
        at = find_top_level_key(out, 'agents')
        line_start = out.rfind('\n', 0, at) + 1
        indent = out[line_start:at]
        if indent.strip():
            indent = '  '
        new_agents = json.dumps(merged, indent=2)
        new_agents = '\n'.join(
            (indent + ln if i else ln)
            for i, ln in enumerate(new_agents.split('\n'))
        )
        out = out[:vs] + new_agents + out[ve:]
        changed = True
        out = delete_top_level_entry(out, 'agent')
        if out is None:
            sys.exit(1)  # cannot drop the v1 block: refuse the write
    else:
        # Every `agent` entry is already present in `agents` verbatim,
        # so dropping the duplicate block cannot lose anything. Drop it
        # via the locator, whatever its layout; if it cannot be located
        # the write is refused (exit 1, file byte-identical) instead of
        # silently leaving the v1 block in place forever (#4612).
        out = delete_top_level_entry(out, 'agent')
        if out is None:
            sys.exit(1)
        changed = True

if not changed:
    sys.exit(0)

# Never write something we cannot parse — and never write a result whose
# parsed document differs from the intended one. The intended document is
# `doc` with the renames/folds above applied; comparing the whole document
# is what proves no entry was dropped on the floor (#2364).
try:
    parsed = json.loads(jsonc_to_json(out))
except Exception:
    sys.exit(1)

expected = dict(doc)
if 'plugin' in expected:
    if 'plugins' in doc:
        expected['plugins'] = list(doc['plugin']) + list(doc['plugins'])
    else:
        expected['plugins'] = expected['plugin']
    del expected['plugin']
if 'agent' in expected:
    expected['agents'] = {**(doc.get('agent') or {}), **(doc.get('agents') or {})}
    del expected['agent']
if parsed != expected:
    sys.exit(1)

fd, tmp_name = tempfile.mkstemp(prefix=f'.{path.name}.normalize-', dir=path.parent)
tmp = pathlib.Path(tmp_name)
try:
    with os.fdopen(fd, 'w') as f:
        fd = -1  # ownership transferred to the file object
        f.write(out)
        f.flush()
        os.fchmod(f.fileno(), original_mode)
        os.fsync(f.fileno())
    tmp.replace(path)
finally:
    if fd >= 0:
        os.close(fd)
    try:
        tmp.unlink()
    except FileNotFoundError:
        pass
print(f"opencode-config-normalize: repaired {path}", file=sys.stderr)
PY
