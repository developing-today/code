---
session: ses_2aed
updated: 2026-04-03T03:31:58.110Z
---



## Conversation Summary

### Overall Task
Implement Phase 2 (Markdown Polish) for pkgs/id collaborative web platform — all 6 parts with unit + E2E tests. Commit after each part. Keep phase-2 plan doc updated. Do NOT proceed to Phase 3 or update features.md until user approves.

### Phase 1 Status: ✅ Complete
All 4 parts committed with E2E tests (commits: eceac4d4, 62570d15, 0a3ce640, 4232b9e9, 71950437, 89585365, 772fbdbf, 706f4e76).

### Phase 2 Plan: ✅ Written
Plan doc at `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-2-markdown-polish.md` — 6 parts approved by user.

### Phase 2 Part 1 (Strikethrough): 🔧 IN PROGRESS

**Completed changes:**

1. **markdown.rs** — All Rust-side changes done:
   - `commonmark_options()` (~line 64): Changed comment and enabled `options.extension.strikethrough = true`
   - `convert_node()` Strikethrough handler (~line 332): Changed from "pass through without mark" to properly adding `strikethrough` mark via `marks.push(Mark { mark_type: "strikethrough", ... })`
   - `create_marked_text()` (~line 615): Added `"strikethrough" => NodeValue::Strikethrough` in the mark_type match
   - Module doc comment: Added `strikethrough` to marks list
   - Added 3 unit tests: `test_strikethrough`, `test_strikethrough_with_other_marks`, `test_roundtrip_strikethrough`

2. **editor.ts** — Partial TypeScript changes done:
   - Updated imports: Added `toggleMark` from prosemirror-commands, `MenuItem` and `MarkType` types from prosemirror-menu/prosemirror-model
   - Updated `richSchema`: Added `strikethrough` mark with `parseDOM` supporting `<s>`, `<del>`, `<strike>` tags and `text-decoration: line-through` style, `toDOM` returns `["s", 0]`
   - Updated schema JSDoc comment

**Still needed for Part 1:**
   - Add `markMenuItem()` helper function for creating toolbar items for custom marks (attempted but hit "multiple matches" error on `/**` pattern)
   - Add strikethrough button to toolbar row 1 (after toggleCode, before toggleLink)
   - Add `Mod-Shift-s` keymap for strikethrough toggle
   - Run Rust unit tests to verify
   - Write E2E Playwright test
   - Commit Part 1

### Key Files Being Modified
- `pkgs/id/src/web/markdown.rs` — Rust markdown↔ProseMirror conversion (comrak)
- `pkgs/id/web/src/editor.ts` — ProseMirror schema, menu, plugins setup
- `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-2-markdown-polish.md` — Phase 2 plan doc

### Remaining Phase 2 Parts
- Part 2: GFM Task Lists (task_list/task_list_item nodes, checkbox nodeView)
- Part 3: GFM Tables (prosemirror-tables dep, 4 table nodes, table editing)
- Part 4: Image Alt-Text Editing (ImageNodeView with alt-text popover)
- Part 5: Image Resize Handles (width/height attrs, drag handles)
- Part 6: Image Browser (GET /api/images endpoint, modal gallery)

### Key Architecture Notes
- All markdown conversion is server-side via comrak (Rust) — no client-side markdown parsing
- Both Rust (markdown.rs) and TypeScript (editor.ts) schemas must stay in sync
- Wire protocol sends PM JSON, not markdown
- Image upload already works (paste/drag-drop via image-upload.ts)
- Existing design doc: `thoughts/shared/designs/2026-04-01-image-upload-design.md`

### User Instructions
- Go one by one through each part, don't parallelize
- Commit after each step
- Keep phase-2 doc updated
- Do not proceed to Phase 3 or update features.md until approval
