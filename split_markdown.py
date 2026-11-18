#!/usr/bin/env python3
"""Split pkgs/id/src/web/markdown.rs into cohesive submodules.

Reads the original file, carves contiguous chunks at doc-block-aware
boundaries, applies pub(crate) visibility to moved items, and writes
the facade (markdown.rs) plus six submodules under markdown/.
Verbatim otherwise: no behavior change.
"""
import re
import sys

SRC = "pkgs/id/src/web/markdown.rs"
OUT_DIR = "pkgs/id/src/web/markdown"

with open(SRC) as f:
    lines = f.read().split("\n")
# drop single trailing empty from split
if lines and lines[-1] == "":
    lines.pop()

TOTAL = len(lines)
print("total lines:", TOTAL, file=sys.stderr)
assert TOTAL == 12368, TOTAL

# Anchor starts (1-based) of items beginning new chunks (from outline).
ANCHORS = [1542, 2782, 3675, 4144, 5348]
# Chunk assignment in order:
NAMES = ["scan", "linkrefs", "convert", "media", "serialize", "ast_build"]

FACADE_END = 164  # 1-based inclusive: `}` closing markdown_to_prosemirror_inner
TESTS_ATTRS_AT = 6275  # 1-based: #[cfg(test)] line


def is_attr_or_doc(s):
    t = s.strip()
    return (
        t == ""
        or t.startswith("///")
        or t.startswith("//!")
        or t.startswith("#[")
        or (t.startswith("//") and not t.startswith("///"))
    )


def chunk_start(anchor):
    """Walk back over doc comments/attrs/blanks to the chunk's first line (1-based)."""
    i = anchor
    while i > 1 and is_attr_or_doc(lines[i - 2]):
        # stop if we'd swallow a blank line followed by code? No: blanks
        # separate items; the doc block directly above `anchor` belongs to it.
        # But a `//` line could be a trailing comment of previous code --
        # in this file, `//` lines directly above a fn start are its docs.
        i -= 1
    return i


starts = [chunk_start(a) for a in ANCHORS]
print("chunk starts:", starts, file=sys.stderr)

bounds = []  # (name, start_1based, end_1based_inclusive)
prev_end = FACADE_END
for name, s in zip(NAMES, starts):
    # previous chunk ends at last non-blank line before s
    e = s - 1
    while e > prev_end and lines[e - 1].strip() == "":
        e -= 1
    bounds.append((name, prev_end + 1 if not bounds else None, None))
    prev_end = None  # placeholder

# rebuild properly
bounds = []
lo = FACADE_END + 1
# skip blanks between facade end and first chunk start
while lo <= TOTAL and lines[lo - 1].strip() == "":
    lo += 1
for idx, (name, s) in enumerate(zip(NAMES, starts)):
    assert s >= lo, (name, s, lo)
    if s > lo:
        # blank gap belongs to neither; new chunk starts at s
        pass
    if idx + 1 < len(starts):
        nxt = starts[idx + 1]
        hi = nxt - 1
        while hi > s and lines[hi - 1].strip() == "":
            hi -= 1
    else:
        hi = TESTS_ATTRS_AT - 1
        while hi > s and lines[hi - 1].strip() == "":
            hi -= 1
    bounds.append((name, s, hi))
    lo = hi + 1

for name, s, hi in bounds:
    print(f"{name}: {s}-{hi} ({hi - s + 1} lines)", file=sys.stderr)

facade = lines[:FACADE_END]
tests_block = lines[TESTS_ATTRS_AT - 1:]

# sanity: tests block starts with #[cfg(test)]
assert tests_block[0].strip() == "#[cfg(test)]", tests_block[0]
# sanity: each chunk's last line is code, first line is doc/attr
for name, s, hi in bounds:
    assert lines[s - 1].strip() != "", (name, "starts blank")
    assert lines[hi - 1].strip() != "", (name, "ends blank")
    assert lines[hi - 1].strip() != "}", True  # usually } but fine either way

TOP_RE = re.compile(r"^(fn|struct|enum|const|static|type) ")
METHOD_RE = re.compile(r"^    (fn )")


def transform(chunk_lines, make_fields_pub=False, field_names=()):
    out = []
    for ln in chunk_lines:
        m = TOP_RE.match(ln)
        if m:
            # don't touch already-pub (none expected except none) — keep pub fns? caller decides
            out.append("pub(crate) " + ln)
            continue
        m2 = METHOD_RE.match(ln)
        if m2:
            out.append("    pub(crate) " + ln[4:])
            continue
        if make_fields_pub:
            fm = re.match(r"^    (%s): " % "|".join(field_names), ln)
            if fm:
                out.append("    pub(crate) " + ln[4:])
                continue
        out.append(ln)
    return out


CROSS = {
    "scan": ["collect_link_reference_definitions", "protect_newline_destinations"],
    "linkrefs": [
        "FenceTracker", "FootnoteDefinitionBlock", "HtmlBlockTracker",
        "carries_container_prefix", "container_signature", "fence_feed_line",
        "footnote_definition_label", "is_atx_heading", "is_blank_line",
        "is_non_interrupting_ordered_marker", "is_setext_underline",
        "is_thematic_break", "markdown_to_prosemirror_inner",
        "restore_attr_newlines", "scan_footnote_definition_blocks",
        "starts_new_block", "starts_with_list_marker",
        "strip_container_levels", "strip_container_prefixes",
    ],
    "convert": [
        "Placement", "commonmark_options", "create_marked_text",
        "image_with_link", "is_mark_wrapper", "json_to_ast",
        "parse_anchor_tag", "parse_img_tag", "parse_linked_img_tag",
        "sanitize_image_url", "sanitize_link_url",
    ],
    "media": [
        "ConversionError", "Placement", "collect_inline_children",
        "json_to_ast", "validate_placement",
    ],
    "serialize": [
        "ConversionError", "FenceTracker", "HtmlBlockTracker",
        "append_table_cell_child", "commonmark_options", "convert_list_runs",
        "create_marked_text", "fence_feed_line", "is_blank_line",
        "is_mark_wrapper", "json_to_ast", "split_quote_prefix",
        "starts_new_block", "strip_quote_depth",
    ],
    "ast_build": [
        "ConversionError", "alt_markdown_to_nodes", "convert_node",
        "delimiter_from_attrs", "escape_attr_value", "is_block_node",
        "mark_nesting_rank", "normalize_hard_break_placement",
        "normalize_inline_child_json", "prosemirror_to_markdown",
        "sanitize_image_url", "sanitize_link_url",
    ],
}

EXTERN = {
    "scan": "use comrak::nodes::{AstNode, NodeValue};",
    "linkrefs": "use serde_json::Value;\nuse std::collections::{HashMap, HashSet};",
    "convert": (
        "use comrak::nodes::{AstNode, ListDelimType, ListType, NodeValue};\n"
        "use comrak::{Arena, parse_document};\n"
        "use serde_json::{Value, json};"
    ),
    "media": (
        "use serde_json::{Value, json};\n"
        "use std::collections::HashMap;"
    ),
    "serialize": (
        "use comrak::nodes::{AstNode, NodeValue};\n"
        "use comrak::{Arena, format_commonmark};\n"
        "use serde_json::{Value, json};"
    ),
    "ast_build": (
        "use comrak::nodes::{AstNode, ListType, NodeValue};\n"
        "use comrak::{Arena, format_commonmark};\n"
        "use serde_json::{Value, json};"
    ),
}

DOCS = {
    "scan": "//! Block-level source scans: fences, HTML blocks, containers, footnotes.",
    "linkrefs": "//! Link reference definitions, footnote preservation, and table escapes.",
    "convert": "//! Markdown AST to ProseMirror JSON conversion.",
    "media": "//! HTML media parsing and URL sanitizing for inline content.",
    "serialize": "//! ProseMirror JSON to markdown serialization.",
    "ast_build": "//! ProseMirror JSON to comrak AST construction.",
}

PUB_FNS = {
    "serialize": ["prosemirror_to_markdown"],
    "ast_build": ["plain_text_to_prosemirror", "raw_text_to_prosemirror"],
}

import os

os.makedirs(OUT_DIR, exist_ok=True)
for name, s, hi in bounds:
    chunk = lines[s - 1 : hi]
    keep_pub = set(PUB_FNS.get(name, []))
    # transform, but restore `pub` on the public API fns
    t = transform(
        chunk,
        make_fields_pub=(name == "scan"),
        field_names=("label", "source", "line"),
    )
    if keep_pub:
        fixed = []
        for ln in t:
            m = re.match(r"^pub\(crate\) fn (\w+)", ln)
            if m and m.group(1) in keep_pub:
                fixed.append("pub " + ln[len("pub(crate) "):])
            else:
                fixed.append(ln)
        t = fixed
    use_super = ""
    if CROSS.get(name):
        use_super = "use super::{%s};\n" % ", ".join(CROSS[name])
    body = "\n".join(t)
    content = f"{DOCS[name]}\n\n{EXTERN[name]}\n{use_super}\n{body}\n"
    with open(os.path.join(OUT_DIR, name + ".rs"), "w") as f:
        f.write(content)

print("wrote submodules", file=sys.stderr)

# Facade: original head (minus old use block) + mods + re-exports + tests
# Original head lines 1..FACADE_END include the 3 use statements; keep comrak/serde
# imports trimmed to what facade needs: Arena, Options, parse_document, Value.
old_head = "\n".join(facade)
old_head = old_head.replace(
    "use comrak::nodes::{AstNode, ListDelimType, ListType, NodeValue};\n"
    "use comrak::{Arena, Options, format_commonmark, parse_document};\n"
    "use serde_json::{Value, json};",
    "use comrak::{Arena, Options, parse_document};\nuse serde_json::Value;",
)
facade_text = (
    old_head
    + "\n\nmod ast_build;\nmod convert;\nmod linkrefs;\nmod media;\nmod scan;\nmod serialize;\n\n"
    + "pub use ast_build::{plain_text_to_prosemirror, raw_text_to_prosemirror};\n"
    + "pub use serialize::prosemirror_to_markdown;\n\n"
    + "pub(crate) use ast_build::*;\n"
    + "pub(crate) use convert::*;\n"
    + "pub(crate) use linkrefs::*;\n"
    + "pub(crate) use media::*;\n"
    + "pub(crate) use scan::*;\n"
    + "pub(crate) use serialize::*;\n\n"
    + "\n".join(tests_block)
    + "\n"
)
with open(SRC, "w") as f:
    f.write(facade_text)
print("wrote facade", file=sys.stderr)
