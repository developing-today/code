---
session: ses_2b53
updated: 2026-04-01T20:50:07.128Z
---

## Summary

### Task
Research the `pkgs/id` codebase to understand how to add an image upload endpoint that stores images as separate files and returns a URL for use in the ProseMirror editor.

### What Was Done
A comprehensive codebase research was completed — **no code was modified**. The following was analyzed in detail:

### Research Findings

**1. Server-Side File/Upload API** (`src/web/routes.rs`)
- All existing POST endpoints accept `Json<...>` bodies only — **no multipart/raw-bytes upload exists**
- `POST /api/new` creates empty files (name → detect_mode → empty bytes → `store.blobs().add_bytes()` → `store.tags().set()` → metadata tags)
- `POST /api/save` converts ProseMirror JSON to bytes based on file extension, stores as blob, archives old version
- Other endpoints: `/api/rename`, `/api/copy`, `/api/download`, `/api/delete`, `/api/restore`, `/api/hard-delete`
- Rate limiting via `SaveRateLimiter` (5s cooldown per file)

**2. richSchema Image Node** (`web/src/editor.ts:41-44`)
- `richSchema` = `addListNodes(basicSchema.spec.nodes)` + `basicSchema.spec.marks` — inherits stock `prosemirror-schema-basic` image node (attrs: `src`, `alt`, `title`; renders as `<img>`)
- **No custom image insert command, paste handler, or drag-drop handler exists** — the node is in the schema but not usable from the UI
- `rawSchema` has no image node (only `doc`, `text`, `code_block`)

**3. File Serving** (`src/web/routes.rs:488-525`)
- `GET /blob/:hash` (`blob_handler`) — serves raw bytes with Content-Type from extension-based detection
- Accepts `?filename=` query param for MIME detection; falls back to tag name lookup via `get_file_name()`
- Cache header: `public, max-age=31536000, immutable` (content-addressed)
- `GET /file/*name` resolves tag name → hash → renders HTML viewer (editor/media/binary), does NOT serve raw bytes

**4. Store/Blob Model** (`src/store.rs`, `src/web/mod.rs`)
- Content-addressed blobs via `iroh_blobs` — `store.blobs().add_bytes(Vec<u8>)` returns hash, `store.blobs().get_bytes(hash)` reads
- Named tags: `store.tags().set(name, hash)` / `.get()` / `.list()` / `.delete()` — multiple tags can point to same hash
- Archive pattern: `{name}.archive.{unix_timestamp}`
- Metadata tags via separate `TagStore` (iroh-docs backed): `created`, `modified`, `deleted`, user-defined key-value pairs

**5. Content Type Handling** (`src/web/content_mode.rs`)
- Purely extension-based — no magic byte sniffing
- `get_content_type()` maps extensions to MIME strings (images, video, audio, PDF, text, code, default octet-stream)
- `detect_mode()` / `detect_mode_with_content()` classifies into Rich/Markdown/Plain/Raw/Media/Binary
- Media images already render in a viewer via `render_media_viewer()` with `<img src="/blob/{hash}?filename=...">` 

### What Needs to Be Done (Implementation Plan)

**Server side** (`src/web/routes.rs`):
1. Add `POST /api/upload` route accepting multipart form data (file bytes + filename)
2. Store bytes: `state.store.blobs().add_bytes(bytes).await`
3. Create tag: `state.store.tags().set(filename, hash).await`
4. Set metadata tags (created, modified)
5. Return `{ "hash": "...", "name": "...", "url": "/blob/{hash}?filename={name}" }`
6. Register route in `create_router()` at line 183

**Frontend** (`web/src/editor.ts` and/or `web/src/main.ts`):
1. Add image insert command/button to the toolbar (for rich/markdown modes with `hasToolbar()`)
2. Add paste handler for image data (clipboard paste)
3. Add drag-and-drop handler for image files
4. Each handler: upload via `POST /api/upload`, then insert ProseMirror image node with `src` = returned URL

**Dependencies**: Will likely need `axum::extract::Multipart` on the Rust side. Check if already in `Cargo.toml` or needs adding.
