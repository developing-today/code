---
session: ses_2aed
updated: 2026-04-03T04:59:16.440Z
---

## Summary: Phase 2 Markdown Polish Implementation

### Overall Goal
Implement Phase 2 (Markdown Polish) — all 6 parts with unit + E2E tests. Commit after each part. Keep phase-2 plan doc updated. Do NOT proceed to Phase 3 or update features.md until user approves.

### Completed Work

**✅ Part 1 (Strikethrough)** — Commit `0c8b6a89`
- Rust: comrak strikethrough, MD↔PM conversion, 3 unit tests
- TS: strikethrough mark, toolbar button (`~~S~~`), `Mod-Shift-s` keymap
- E2E: 3 tests (render, roundtrip, toolbar)

**✅ Part 2 (Task Lists)** — Commit `a3cb5670`
- Rust: tasklist in comrak, task_list/task_list_item nodes, 3 unit tests (26 total pass)
- TS: task_list + task_list_item nodes, nodeView with checkbox toggle
- CSS: task list styles
- E2E: 2 tests pass on firefox

**⏳ Part 3 (Tables)** — Code complete, E2E tests not yet inserted into file

#### Part 3 Done:
- **Rust** (29/29 tests pass): `table=true` in comrak, handlers for Table/TableRow/TableCell in both directions, paragraph wrapping/unwrapping for cells, 3 unit tests
- **TypeScript** (typecheck passes): 4 new nodes (table, table_row, table_cell, table_header) in richSchema via `.append()`
- **CSS**: `.pm-table` styles in editor-compat.css
- **Decision**: No prosemirror-tables dependency — tables render/roundtrip but no toolbar insert or row/column management. Follow-up later.

#### Part 3 Remaining:
1. Insert 2 E2E tests into `editor-features.spec.ts` (after line ~1392, Task Lists closing)
2. Build: `cd pkgs/id/web && bun run build`
3. Build Rust: `cargo build --features web` in pkgs/id/
4. Run E2E: `npx playwright test --grep "Table" --project firefox`
5. Update phase-2 plan status (Part 3 → Done)
6. Commit

### Parts 4-6 Still TODO:
- Part 4: Alt-Text for Images
- Part 5: Image Resize
- Part 6: Image Browser

### Key Files
- `pkgs/id/src/web/markdown.rs` — Rust MD↔PM conversion
- `pkgs/id/web/src/editor.ts` — ProseMirror schema/menu/plugins
- `pkgs/id/web/src/editor-compat.css` — Editor CSS
- `pkgs/id/e2e/tests/editor-features.spec.ts` — E2E tests
- `thoughts/shared/plans/.../phase-2-markdown-polish.md` — Plan status

### Key Architecture & Commands
- MD conversion is server-side (comrak/Rust); both Rust+TS schemas must stay in sync
- Rust tests: `cargo test -p id --features web --lib web::markdown::tests`
- TS typecheck: `npx tsc --noEmit` in pkgs/id/web/
- E2E: `npx playwright test --grep "..." --project firefox` in pkgs/id/e2e/
- Web build: `cd pkgs/id/web && bun run build`
- E2E tests require `doc_id: hash` in save requests
