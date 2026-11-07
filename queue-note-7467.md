🤖 **Triage (queue task 7467): fix already landed — closed as completed**

Verified against fresh `main` (`0467ae0f9`) on 2026-11-03. This issue was already implemented by PR #2881 (task 1675), merged 2026-11-02T16:03:50Z as commit `b6969b0`.

**What is on main now**
- comrak `front_matter_delimiter = Some("---")` — `pkgs/id/src/web/markdown.rs:90`
- `NodeValue::FrontMatter(text)` → `front_matter` ProseMirror node, verbatim `text` attr — `pkgs/id/src/web/markdown.rs:2765` and `:4514`
- `front_matter` legal only as first child of `doc`; misplaced/unclosed/non-empty-content cases rejected — `pkgs/id/src/web/markdown.rs:3916-3951`
- Editor rich schema renders `front_matter` as an atom leaf, never a heading/paragraph — `pkgs/id/web/src/editor.ts:360`
- Regression tests: `test_front_matter_roundtrip_fixpoint`, `test_front_matter_mid_doc_dashes_stay_horizontal_rule`, `test_front_matter_unclosed_leading_dashes_stay_break`, `test_front_matter_misplaced_is_rejected` — `pkgs/id/src/web/markdown.rs:10215+`
- PR checks: CodeRabbit SUCCESS; PR body records 119 codec tests passing.

**Acceptance criteria**
- [x] YAML block round-trips verbatim/fixpoint, including multi-key, blank-line-in-block, heading body and plain-text body (round-trip test cases)
- [x] Front matter is not editable as body: dedicated atom node
- [x] Leading `---` that is not front matter still round-trips as `horizontal_rule` (control test)
- [x] Regression tests in `pkgs/id/src/web/markdown.rs` mirroring existing codec test style
- [x] TOML (`+++`) explicitly out of scope in the fix PR

Outcome: `already_done`. No new branch or PR needed. Queue/work labels cleared; issue closed as completed with this evidence.

<!-- agent-note:v1 agent=repo-worker-code state=done at=2026-11-03T15:47:00Z task=7467 -->
