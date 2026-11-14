# Refactor record: split of `pkgs/id/src/web/markdown.rs` (#4704)

## Request and intent

Issue #4704: `pkgs/id/src/web/markdown.rs` held 6,273 production lines at
the split's base (5,325 when the F11.10 guard landed), far over the
2,500 production-line limit enforced by
`.github/scripts/check-production-line-limit.sh` (#1411). The file was
carried on the guard's grandfather list; the issue asks for a
behavior-preserving split into cohesive modules below the limit and
removal of only this file's grandfather entry in the same change. The
entries for #4715 (cli.rs), #4716 (web/routes), and #4717
(world_session) are separate issues and are not this change's business.

This record was written at review time (Codex finding on PR #8404
noted the missing dated record). The decisions below are the ones the
split in PR #8404 actually implements; the module inventory in the
module header of `web/markdown.rs` carries the same map in brief.

## Approach

Split along the codec's existing pipeline seams, moving code verbatim —
no logic changes in the split commits. Anything a moved item referenced
across a new module boundary gained `pub(crate)` visibility, and the
root module re-exports the moved items, so the crate-visible and
public API surface is unchanged.

## Module boundaries

- `web/markdown/markdown_scan.rs` (1,282 production lines) — source
  scans over raw markdown text: fence and HTML-block tracking,
  container prefixes, and the footnote / link-reference source scans
  that run before parsing.
- `web/markdown/parse_helpers.rs` (1,281 production lines) — parse-side
  helpers: link reference definitions, footnote synthesis, destination
  protection, and table code-span escapes.
- `web/markdown/prosemirror_json.rs` (1,479 production lines) —
  ProseMirror JSON construction from the CommonMark AST
  (`convert_node`, inline/list conversion, placement validation).
- `web/markdown/markdown_serialization.rs` (1,072 production lines) —
  the save direction: `prosemirror_to_markdown` and its rendering
  helpers, including hard-break placement normalization.
- `web/markdown.rs` (root, ~1,176 production lines) — keeps the public
  API, `json_to_ast`, the module declarations and re-exports, and the
  test module.

Every resulting production file is at or below the 2,500-line limit.

## Visibility decisions

- Moved items are `pub(crate)` at most; nothing became `pub` that was
  not already reachable through the root module's re-exports. The split
  widens visibility only across the new intra-crate module boundaries,
  never across the crate boundary.
- Helpers shared by exactly one consumer module moved with that
  consumer (e.g. the fence-placeholder helpers live with serialization,
  their only caller). Helpers used by the root's retained code stayed
  reachable via the root's re-exports rather than duplicating code.

## Guard changes

- The `pkgs/id/src/web/markdown.rs` grandfather entry is removed from
  `.github/scripts/check-production-line-limit.sh` in the same PR.
- The guard's contract-test example for a grandfathered path moved to
  a path that remains grandfathered (originally `pkgs/id/src/cli.rs`,
  #4715).

## Verification (at split time)

- `cargo check --lib --features web,sandbox,ssh,world` in `pkgs/id` —
  clean (after the #8355 commit below).
- `cargo test --lib --features web web::markdown` — 266 passed,
  0 failed (the full existing markdown suite, unchanged).
- The line guard over the tree: no markdown file flagged; the guard's
  contract tests (`check-production-line-limit.test.sh`) pass.

Also in PR #8404, as a separate first commit: the one-character
`serve.rs` typo fix for #8355 (`_started_server_config`), which broke
every web+world build on the then-current main — the split's
verification could not compile without it.

---

## 2026-11-11T03-19-54Z Update: merge with main and review record

While PR #8404 waited, main moved: cli.rs was split for #4715 (its
grandfather entry removed), the identical #8355 typo fix landed as
85deec737d, and the #1002 blockquote blank-line fix added helpers and
a test inside the monolith. The PR branch merged main (afb8c9800e)
with these resolutions:

- Guard list: both removals combined — only the #4717
  `world_session/mod.rs` entry remains. The contract-test grandfather
  example now uses `pkgs/id/src/world_session/mod.rs`.
- The #1002 helpers (`is_blank_quote_line`, `is_quote_content_line`,
  `strip_leading_blank_quote_lines`) and their `prosemirror_to_markdown`
  call site were ported byte-identical into `markdown_serialization.rs`
  (their consumer's module); the #1002 round-trip test and the test
  module's `clippy::panic` allow were ported into the root test module;
  main's panic-safe char read in `escape_code_span_pipes_in_line` was
  ported into `parse_helpers.rs`.
- Re-verified after the merge: `cargo test --lib --features web
web::markdown` — 267 passed, 0 failed (266 + the ported #1002 test);
  guard contract tests all pass; the guard over the merged tree flags
  only the four over-limit files that current main itself already
  fails on (fleet-queue `client.rs` and `turso_queue.rs`, id
  `serve.rs` and `directory.rs`) — pre-existing on main, unchanged by
  this PR.
