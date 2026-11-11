# Refactor record: split `pkgs/id/src/web/markdown.rs` into four modules

- Date: 2026-11-11T01-29-34Z
- Kind: refactor record (per `pkgs/id/AGENTS.md`, "Documenting Design &
  Architecture Decisions", and `docs/DOCUMENTATION_PROTOCOL.md`)
- Scope: `pkgs/id/src/web/markdown.rs` and the new
  `pkgs/id/src/web/markdown/` module directory
- Tracking: production-line limit grandfather entry for `markdown.rs`
  (`.github/scripts/check-production-line-limit.sh`, LIMIT=2500) —
  split tracked by issue #4704 and issue #1438; implementation
  preserved on PR #7256 (branch `queue/6599-1438-prod-lines-check`,
  task 6599, shepherded as task 13547)
- Codex P1 this record answers: PR #7256 review comment 4505057410
  ("Document the Markdown module split before landing it")

## Intent

On current main, `pkgs/id/src/web/markdown.rs` is a 12,368-line
monolith holding the whole Markdown ↔ ProseMirror conversion
pipeline: source pre-scans, two conversion directions, and their
safeguards. It exceeds the production-line limit by ~5x and is
grandfathered only because the split is tracked work. The split
separates the pipeline by conversion stage so each stage's
invariants live in one file, without changing any behavior or any
public signature.

The split as preserved on PR #7256 was implemented before this
record existed; this record documents the boundaries as implemented
there and is the required documentation gate before the split is
segmented onto current main and landed (see "Landing" below).

## Module boundaries

All four new modules are private children of the `markdown` module
(`mod` declarations in the root file). Nothing outside
`web::markdown` names them.

### `markdown.rs` (root; 5,179 lines on the PR #7256 branch)

- The module contract: the `//!` doc (ProseMirror schema — node and
  mark inventory), `ConversionError`, and the public entry points
  `markdown_to_prosemirror`, `plain_text_to_prosemirror`,
  `raw_text_to_prosemirror`.
- Orchestration of the conversion pipeline (comrak parse setup,
  pre-scan invocation, node conversion, serialization) and the
  wiring of the four child modules.
- The full `#[cfg(test)]` suite stays in the root: the tests pin
  end-to-end conversion behavior across all four modules, and
  keeping them in one place keeps that oracle intact.

### `markdown/markdown_scan.rs` (2,187 lines)

Source scans and recovery for footnote and link-reference
definitions — the pre-parse passes that find definitions comrak
would otherwise misplace, plus the line-classification predicates
they share: container-prefix stripping, fence open/close detection,
`is_atx_heading`, `is_setext_underline`, `is_thematic_break`,
`is_complete_html_tag` / HTML block tracking, indentation and
list-marker probes (`scan_footnote_definition_blocks`,
`footnote_definition_label`, `starts_new_block`, ...). Rule: if a
function reads raw source lines to classify or recover structure
before the AST exists, it belongs here.

### `markdown/markdown_serialization.rs` (1,324 lines)

The outbound direction: ProseMirror JSON → comrak AST → Markdown.
Owns `prosemirror_to_markdown` (re-exported by the root, below) and
`json_to_ast` (used by the root's tests), delimiter-edge space
hoisting (`hoist_delimiter_edge_spaces`), fenced-code-block forcing
inside blockquotes, spaced-destination substitution, and the
rendered-char edge probes. Rule: if a function shapes the emitted
Markdown text or the AST built to emit it, it belongs here.

### `markdown/parse_helpers.rs` (375 lines)

Parsing safeguards for destinations, tables, and restored
attributes — the small, sharp pre/post passes that protect the
parser from known lossy shapes: `protect_newline_destinations`,
`escape_table_code_span_pipes`, `restore_attr_newlines` (all
`pub(super)`), plus their private line predicates (table delimiter
rows, fence lines, escape checks). Rule: a safeguard that rewrites
source or output around the parse, without owning a conversion
direction, belongs here.

### `markdown/prosemirror_json.rs` (1,318 lines)

The inbound direction: comrak AST → ProseMirror JSON. Owns
`convert_node` and `collect_block_children` (`pub(super)`), the
`Mark` type and `mark_nesting_rank`, list-run collection, and
alt-text Markdown recovery (`alt_markdown_to_nodes`,
`escape_attr_value`). Rule: if a function turns AST nodes into
JSON values, it belongs here.

## Shared helpers and visibility discipline

- The child modules are implementation detail. Cross-module access
  is deliberately narrow: the root glob-imports `markdown_scan`,
  `parse_helpers`, and `prosemirror_json` for its orchestration;
  helper items shared upward are `pub(super)`, never `pub`.
- The single outward re-export is
  `pub use self::markdown_serialization::prosemirror_to_markdown;`
  in the root — the serialization module owns the function, the
  root keeps its historical path.
- `ConversionError` stays in the root: both conversion directions
  and the public API share it, so it belongs to the module contract,
  not to either direction.
- No helper was duplicated to avoid a cross-module call; where the
  scan predicates are needed by both directions they stay in
  `markdown_scan` and are reached through the root's imports.

## Public conversion API preservation

The public surface of `web::markdown` is unchanged by the split —
same names, same signatures, same paths:

- `markdown_to_prosemirror(markdown: &str) -> Value`
- `plain_text_to_prosemirror(text: &str) -> Value`
- `raw_text_to_prosemirror(text: &str) -> Value`
- `prosemirror_to_markdown(doc: &Value) -> Result<String, ConversionError>`
- `ConversionError`

Callers therefore need no changes: `web/mod.rs` re-exports the four
functions, `web/collab.rs` dispatches on them per content mode, and
`web/routes.rs` calls `prosemirror_to_markdown` /
`plain_text_to_prosemirror` directly. Behavior preservation is
pinned by the root test suite, which moves with the root file
unchanged; the split commits move code without editing logic.

## Landing (remaining steps after this record)

1. Segment CURRENT main's `markdown.rs` by the item-to-module map
   above (the PR #7256 branch predates later main work; main-only
   items go by affinity — e.g. the GitHub-alert / link-URL /
   hard-break helpers follow the scan/serialization module that
   owns their callers).
2. Drop the branch's superseded hunks (line-limit scripts,
   `pkgs/id/flake.nix`, plan status row) in favor of main's versions.
3. `cargo test -p id` on the result (expect only the pre-existing
   frames byte-refusal failure, issue #8354).
4. On merge, remove `markdown.rs` from the grandfather list in
   `check-production-line-limit.sh` and close the loop on #4704.
