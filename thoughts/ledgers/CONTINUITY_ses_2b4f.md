---
session: ses_2b4f
updated: 2026-04-01T22:03:03.460Z
---



## Summary

### Task
Execute the image upload implementation plan at `/home/user/code/pkgs/id/.opencode/worktrees/image-upload/pkgs/id/thoughts/shared/plans/2026-04-01-image-upload.md` — Batches 1-4 (CSS, helpers, server endpoint, plugin, wiring, unit tests). Skip Batch 5 (E2E + nix builds). Do NOT commit.

### Working Directory
All paths relative to: `/home/user/code/pkgs/id/.opencode/worktrees/image-upload/pkgs/id` (a git worktree)

### Completed Work

**Batch 1 — ✅ DONE**

1. **Task 1a: CSS Section 17** — Appended image upload styles to `web/src/editor-compat.css` (lines 1152-1198). Includes `.image-upload-placeholder` with pulse animation, `.ProseMirror img` max-width/block/border-radius/margin, hover outline, selected node outline. Touched `src/web/assets.rs` to force rust-embed recompile.

2. **Task 1b: image-upload.ts** — Created `web/src/image-upload.ts` with:
   - Constants: `ALLOWED_IMAGE_TYPES` (7 MIME types), `MAX_IMAGE_SIZE` (10MB)
   - Helpers: `isImageFile()`, `mimeToExtension()`, `generatePasteFilename()`
   - Upload API: `UploadResponse` interface, `uploadImageFile()` (POST FormData to `/api/upload`)
   - Plugin: `createImageUploadPlugin(schema)` — returns null if no `image` node in schema; handles paste/drop with placeholder decorations
   - TypeScript compilation verified clean (zero errors)

### Remaining Work

**Batch 2 — NOT STARTED**
- **Task 2a**: Add `POST /api/upload` endpoint to `src/web/routes.rs` — multipart handler, MIME validation, blob storage, metadata tags, JSON response `{hash, name, url}`. Add `DefaultBodyLimit` layer (10MB). Add Rust unit tests.
- **Task 2b**: Already done as part of 1b (plugin was combined into single file)

**Batch 3 — NOT STARTED**
- **Task 3a**: Wire plugin into `web/src/editor.ts`:
  - Import `createImageUploadPlugin` from `./image-upload`
  - After `createIndentPlugin()` push (line 256), add image upload plugin with schema guard
  - Add `menuItems.insertImage` to Row 1 of customMenu (line 201) after `menuItems.toggleLink`

**Batch 4 — NOT STARTED**
- **Task 4a**: Create `web/src/image-upload.test.ts` with vitest tests for all helpers, constants, plugin creation, and upload function (mock fetch)
- **Task 4b**: Run `cd web && bun run test` and `npx tsc --noEmit` to verify all tests pass

### Key Technical Context

- **axum Multipart import**: `use axum::extract::Multipart;` and `use axum::extract::DefaultBodyLimit;` — built into axum 0.7, no new deps
- **richSchema** has image node (from prosemirror-schema-basic), **rawSchema** does NOT — plugin guards on this
- **routes.rs** is ~2026 lines; route registration in `create_router()` around line 190; test block starts around line 1870
- **editor.ts** is 372 lines; plugin registration around line 256; toolbar menu built around line 199-212
- **Test patterns**: existing tests use `describe()/it()` from vitest; look at `highlight.test.ts`, `wrap.test.ts`
- **Build commands**: `cd web && bun install && bun run build`, `cargo build --no-default-features`, `cd web && bun run test`

### Next Step
Proceed with **Batch 2, Task 2a**: Implement the Rust upload endpoint in `src/web/routes.rs`.
