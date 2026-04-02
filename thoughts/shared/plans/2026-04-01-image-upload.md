# Implementation Plan: Image Drag-Drop/Paste Upload

**Design**: [2026-04-01-image-upload-design.md](../designs/2026-04-01-image-upload-design.md)
**Branch**: `image-upload`

## Implementation Decisions

- **No new cargo deps**: axum 0.7 has built-in `axum::extract::Multipart` and `axum::extract::DefaultBodyLimit`
- **No new npm packages**: ProseMirror `Decoration`, `DecorationSet`, `Plugin` already available
- **Toolbar**: Use existing `menuItems.insertImage` from `prosemirror-example-setup` (prompts for URL) — plus a file-picker button that triggers upload flow
- **Schema guard**: Only register image upload plugin when `editorSchema.nodes.image` exists (rich/markdown/plain modes, not raw)
- **MIME allowlist**: `image/png`, `image/jpeg`, `image/gif`, `image/webp`, `image/svg+xml`, `image/bmp`, `image/x-icon`
- **Max upload**: 10MB enforced client-side (reject before upload) and server-side via `DefaultBodyLimit`
- **Clipboard paste naming**: `paste-{unix_timestamp_ms}.{ext}` where ext derived from MIME type

## Batch 1: CSS + TypeScript Constants (parallel)

### Task 1a: CSS Section 17 — Image Upload Styles

**File**: `web/src/editor-compat.css`
**Action**: Append new section 17 after the existing section 16 (Go to Line Dialog)

Add:
- `.image-upload-placeholder` — inline-block, animated pulsing border, min dimensions, centered text "Uploading..."
- `.ProseMirror img` — `max-width: 100%`, `display: block`, `border-radius: 0.25rem`, `margin: 0.5rem 0`
- `.ProseMirror img:hover` — subtle `outline: 2px solid oklch(var(--p) / 0.3)`
- `.ProseMirror img.ProseMirror-selectednode` — `outline: 2px solid oklch(var(--p) / 0.6)`

### Task 1b: Image Upload Constants & Helpers

**File**: `web/src/image-upload.ts` (NEW)
**Action**: Create file with:
- `ALLOWED_IMAGE_TYPES` — array of MIME strings
- `MAX_IMAGE_SIZE` — 10 * 1024 * 1024 (10MB)
- `isImageFile(file: File): boolean` — checks type against allowlist
- `generatePasteFilename(mimeType: string): string` — returns `paste-{Date.now()}.{ext}`
- `mimeToExtension(mime: string): string` — maps MIME to file extension
- Type: `UploadResponse = { hash: string; name: string; url: string }`

## Batch 2: Server Endpoint + ProseMirror Plugin (parallel)

### Task 2a: Server `POST /api/upload` Endpoint

**File**: `src/web/routes.rs`
**Actions**:

1. Add route to `create_router()` after existing `/api/new`:
   ```
   .route("/api/upload", post(upload_handler))
   .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
   ```
   Note: `DefaultBodyLimit` applies per-route via tower layer

2. Add `UploadResponse` struct (near other response structs):
   - `hash: String`
   - `name: String`  
   - `url: String`

3. Add `upload_handler` function:
   - Signature: `async fn upload_handler(State(state): State<AppState>, mut multipart: Multipart) -> Response`
   - Read single field from multipart
   - Validate content_type is in ALLOWED_IMAGE_TYPES
   - Read bytes, validate not empty
   - Generate filename: use original filename from field, or `paste-{timestamp}.{ext}` for unnamed
   - `state.store.blobs().add_bytes(bytes)` → get hash
   - `state.store.tags().set(tag_name, hash)` → create named tag
   - Set metadata tags (created, modified) following `new_file_handler` pattern
   - Return JSON: `{ hash, name, url: "/blob/{hash}?filename={name}" }`
   - Error cases: 400 for non-image/empty, 413 handled by DefaultBodyLimit layer

4. Add `ALLOWED_IMAGE_TYPES` constant array in routes.rs:
   `["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml", "image/bmp", "image/x-icon"]`

5. Add Rust unit tests in the existing `#[cfg(test)] mod tests` block:
   - `test_upload_response_serialization` — verify JSON shape
   - `test_allowed_image_types` — verify allowlist contents
   - `test_paste_filename_generation` — verify format

### Task 2b: ProseMirror Image Upload Plugin

**File**: `web/src/image-upload.ts` (extend from Batch 1)
**Action**: Add the main plugin logic:

1. `uploadImageFile(file: File): Promise<UploadResponse>` — constructs FormData, fetches `/api/upload`, returns parsed JSON

2. `createPlaceholderDecoration(pos: number, id: string): DecorationSet` — creates inline widget decoration at pos showing upload spinner

3. `createImageUploadPlugin(schema: Schema): Plugin | null` — returns null if `schema.nodes.image` doesn't exist. Plugin spec:
   - `state.init` → empty DecorationSet
   - `state.apply` → maps decorations through transaction, handles add/remove placeholder actions via transaction meta
   - `props.decorations` → returns current DecorationSet
   - `props.handlePaste(view, event)` → check `event.clipboardData?.files` for images, upload each, insert node
   - `props.handleDrop(view, event)` → check `event.dataTransfer?.files` for images, get drop position via `view.posAtCoords`, upload each, insert node

4. Upload flow (shared by paste and drop):
   - Validate file (isImageFile, size check)
   - Generate placeholder ID (random string)
   - Add placeholder decoration at insert position via transaction meta
   - Call `uploadImageFile(file)`
   - On success: remove placeholder, insert `schema.nodes.image.create({ src: url, alt: name })` at placeholder position
   - On failure: remove placeholder, console.warn

5. Export: `createImageUploadPlugin`, `uploadImageFile`, `isImageFile`, `ALLOWED_IMAGE_TYPES`, `MAX_IMAGE_SIZE`

## Batch 3: Editor Wiring + Toolbar

### Task 3a: Register Plugin in Editor

**File**: `web/src/editor.ts`
**Actions**:

1. Import `createImageUploadPlugin` from `./image-upload`

2. In `initEditor()`, after the existing `createIndentPlugin()` push, add:
   ```
   const imageUploadPlugin = createImageUploadPlugin(editorSchema);
   if (imageUploadPlugin) {
     plugins.push(imageUploadPlugin);
   }
   ```
   This naturally guards on schema having image node (raw mode → null → not added)

3. In the toolbar section (where customMenu is built), add `menuItems.insertImage` to Row 1 after `menuItems.toggleLink`:
   ```
   [menuItems.toggleStrong, menuItems.toggleEm, menuItems.toggleCode, menuItems.toggleLink, menuItems.insertImage]
   ```
   The stock `insertImage` from prosemirror-example-setup opens a prompt dialog for URL — this covers the "insert by URL" case alongside paste/drop for file upload.

## Batch 4: Unit Tests

### Task 4a: TypeScript Unit Tests

**File**: `web/src/image-upload.test.ts` (NEW)
**Action**: Create vitest tests:

1. `describe("isImageFile")` — accepts each allowed MIME, rejects text/plain, rejects application/pdf, rejects empty type

2. `describe("generatePasteFilename")` — correct extension for each MIME, includes timestamp, format matches `paste-{digits}.{ext}`

3. `describe("mimeToExtension")` — maps each MIME to correct ext, unknown returns "bin"

4. `describe("ALLOWED_IMAGE_TYPES")` — contains all 7 expected types, is an array

5. `describe("MAX_IMAGE_SIZE")` — equals 10MB

6. `describe("createImageUploadPlugin")` — returns Plugin when schema has image node, returns null when schema lacks image node (use rawSchema), plugin has handlePaste prop, plugin has handleDrop prop

7. `describe("uploadImageFile")` — mock fetch, verify FormData construction, verify response parsing, verify error handling on network failure

### Task 4b: Verify Existing Tests Still Pass

**Command**: `just test-web-unit` and `just test-web-typecheck`

## Batch 5: E2E Tests + Build Verification

### Task 5a: E2E Tests

**File**: `e2e/tests/editor-features.spec.ts`
**Action**: Add new describe block "Image Upload" with tests:

1. **"uploaded image displays in editor"** — Create .md file via API, upload image via `/api/upload` endpoint (construct multipart), insert image node via ProseMirror transaction, verify `<img>` element visible in editor

2. **"image src points to blob URL"** — Verify img src matches `/blob/{hash}?filename=...` pattern

3. **"image persists after save and reload"** — Upload image, save file, reload page, verify image still present

4. **"upload endpoint rejects non-image"** — POST text file to `/api/upload`, verify 400 response

Note: Testing actual paste/drop in Playwright is complex (requires `page.dispatchEvent` with synthetic DataTransfer). Focus on API-level upload + ProseMirror node insertion via `page.evaluate()`.

### Task 5b: Build & Nix Verification

**Commands** (sequential):
1. `just build` — full web build
2. `just test-web-unit` — all TS unit tests pass
3. `just test-web-typecheck` — no TS errors  
4. `just test-e2e-firefox` — all E2E tests pass
5. `touch src/web/assets.rs` — force rust-embed recompile
6. Individual nix checks: `nix build -L .#checks.x86_64-linux.id-test-web-unit` etc.

### Task 5c: Commit

Single commit with message: `feat(web): image drag-drop/paste upload with blob storage`

Files changed:
- `src/web/routes.rs` (modified — upload endpoint + tests)
- `web/src/image-upload.ts` (new — plugin + helpers)
- `web/src/image-upload.test.ts` (new — unit tests)
- `web/src/editor.ts` (modified — plugin registration + toolbar)
- `web/src/editor-compat.css` (modified — section 17)
- `e2e/tests/editor-features.spec.ts` (modified — image E2E tests)

## Verification Checklist

- [ ] `POST /api/upload` accepts multipart image, returns `{ hash, name, url }`
- [ ] `POST /api/upload` rejects non-image MIME types with 400
- [ ] Paste image in rich/markdown editor → uploads and inserts `<img>`
- [ ] Drop image file on editor → uploads and inserts `<img>`
- [ ] Placeholder shown during upload, replaced on success
- [ ] Failed upload removes placeholder, no broken state
- [ ] Toolbar has image insert button (URL prompt)
- [ ] Raw mode editor does NOT have image upload plugin
- [ ] Image `src` is `/blob/{hash}?filename=...` (immutable, cacheable)
- [ ] Markdown round-trip: image node → `![alt](src)` → image node
- [ ] `just test-web-unit` passes
- [ ] `just test-web-typecheck` passes
- [ ] `just test-e2e-firefox` passes
- [ ] Nix checks pass
