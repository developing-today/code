# Image Upload: Paste, Drag-Drop, and Toolbar Insert

> See [original plan](../../thoughts/shared/plans/2026-04-01-image-upload.md)
>
> See [design document](../../thoughts/shared/designs/2026-04-01-image-upload-design.md)


## Feature Summary

Users can insert images into the ProseMirror editor through three methods:

1. **Clipboard paste** — Copy an image (e.g., screenshot) and paste it directly into the editor. The image data is extracted from the clipboard event, uploaded to the server, and inserted inline.
2. **Drag-and-drop** — Drag image files from the desktop or file manager into the editor. Files are uploaded on drop and inserted at the cursor position.
3. **Toolbar "Insert Image" button** — Click the toolbar button, enter an image URL in the prompt dialog, and the image node is inserted without uploading (for linking to external images).

Uploaded images are stored as content-addressed blobs in the iroh blob store. Each blob receives a named tag and metadata (created timestamp, modified timestamp, content-type). The server returns a blob URL in the form `/blob/{hash}?filename={name}`, which the editor uses as the image `src`. Because blobs are content-addressed, duplicate uploads naturally deduplicate — identical bytes always produce the same hash.


## Architecture

### Server: `POST /api/upload`

The upload endpoint in `src/web/routes.rs` accepts `multipart/form-data` requests. It validates the uploaded file's MIME type against an allowlist of 7 image types:

| MIME Type | Extension |
|-----------|-----------|
| `image/png` | `.png` |
| `image/jpeg` | `.jpg` |
| `image/gif` | `.gif` |
| `image/webp` | `.webp` |
| `image/svg+xml` | `.svg` |
| `image/bmp` | `.bmp` |
| `image/x-icon` | `.ico` |

Request size is capped at 10MB via axum's `DefaultBodyLimit`. On successful validation, the handler:

1. Reads the file bytes from the multipart field
2. Stores the bytes as an iroh blob (content-addressed by hash)
3. Creates a named tag for the blob using the original or generated filename
4. Sets metadata tags: `created` (unix timestamp), `modified` (unix timestamp), `content-type` (MIME string)
5. Returns an `UploadResponse` JSON: `{ hash, name, url }` where `url = /blob/{hash}?filename={name}`

For pasted images that lack an original filename, `generate_paste_filename()` creates one using the pattern `paste-{unix_timestamp}.{ext}`, where the extension is derived from the MIME type via `mime_to_extension()`.

### Client: `image-upload.ts` ProseMirror Plugin

The TypeScript plugin (`web/src/image-upload.ts`) creates a ProseMirror plugin with two event handlers:

- **`handlePaste`** — Intercepts clipboard events, filters for image files from `clipboardData.files`, validates MIME type and file size client-side, then initiates the upload flow.
- **`handleDrop`** — Intercepts drop events, uses `view.posAtCoords()` to determine the insertion position from the drop coordinates, then follows the same upload flow.

During upload, a **placeholder decoration** (a ProseMirror `Decoration.widget`) is displayed at the insertion point — a pulsing CSS animation indicates the upload is in progress. On success, the placeholder is removed and an `image` node is inserted with `src` set to the returned blob URL. On failure, the placeholder is cleaned up and no broken image is left in the document.

### Editor Wiring

The plugin is registered in `web/src/editor.ts` after `createIndentPlugin()`. Registration is guarded on `schema.nodes.image` existing in the current schema — this means the plugin is active in rich, markdown, and plain content modes (which include the `image` node in their schemas) but returns `null` for raw mode, preventing paste/drop handlers from activating where they don't apply.

### Toolbar Integration

The "Insert Image" button uses `menuItems.insertImage` from `prosemirror-example-setup`, which presents a URL prompt dialog. It is added to Row 1 of the custom menu bar, positioned after `toggleLink`. This provides a non-upload path for inserting images by URL (e.g., linking to an existing blob or external image).

### CSS Styles

Section 17 in `web/src/editor-compat.css` provides styles for:

- **Placeholder animation** — Pulsing opacity effect while upload is in progress
- **Image display** — `max-width: 100%`, `border-radius`, and `margin` for inline images
- **Hover/selected outlines** — Visual feedback when images are hovered or selected in the editor


## Data Flow

### Paste Flow

1. User pastes (Ctrl+V / Cmd+V) with image data on the clipboard
2. `handlePaste` intercepts the `ClipboardEvent`, iterates `clipboardData.files`
3. Each image file is validated: must match `ALLOWED_IMAGE_TYPES` and be under 10MB
4. A placeholder decoration is added at the current cursor position via a plugin transaction
5. `FormData` is constructed with the file and POSTed to `/api/upload`
6. On success: placeholder is removed, an `image` node is inserted with `src` = returned URL
7. On failure: placeholder is removed, no node is inserted

### Drop Flow

1. User drops image file(s) onto the editor
2. `handleDrop` intercepts the `DragEvent`, calls `view.posAtCoords({ left: event.clientX, top: event.clientY })` to determine the document position
3. Same upload flow as paste (validate → placeholder → POST → insert or cleanup)

### Toolbar Flow

1. User clicks "Insert Image" in the toolbar menu
2. `prosemirror-example-setup`'s built-in URL prompt dialog appears
3. User enters an image URL and confirms
4. An `image` node is inserted directly with the provided URL as `src` — no upload occurs

### Blob Serving

Uploaded images are served by the existing `/blob/:hash?filename=name` endpoint. This handler sets the correct `Content-Type` header based on stored metadata and includes `Cache-Control: public, max-age=31536000, immutable` since content-addressed blobs are immutable by definition — the same hash always returns the same bytes.

### Markdown Round-Trip

Image nodes participate in the existing ProseMirror ↔ Markdown serialization:

- **Serialization:** `prosemirror_to_markdown()` converts `image` nodes to `![alt](src)` syntax
- **Parsing:** `markdown_to_prosemirror()` parses `![alt](src)` back into `image` nodes

This means images pasted into a markdown document are preserved when the document is saved and reloaded, with the blob URL appearing as the image source in the markdown text.


## Files Changed

| File | Change | Description |
|------|--------|-------------|
| `src/web/routes.rs` | Modified | Added `POST /api/upload` handler, `UploadResponse` struct, `ALLOWED_IMAGE_TYPES` constant, `mime_to_extension()` and `generate_paste_filename()` helpers, 4 unit tests |
| `Cargo.toml` | Modified | Added `"multipart"` to axum features (pulls `encoding_rs` + `multer` as transitive deps) |
| `web/src/image-upload.ts` | **New** | ProseMirror plugin with `handlePaste`, `handleDrop`, placeholder decorations, upload logic (~244 lines) |
| `web/src/image-upload.test.ts` | **New** | 39 vitest unit tests covering MIME validation, extension mapping, filename generation, plugin creation, upload mocking |
| `web/src/editor.ts` | Modified | Plugin registration after `createIndentPlugin()`, toolbar "Insert Image" button in Row 1 |
| `web/src/editor-compat.css` | Modified | Section 17: placeholder animation, image display styles, hover/selected outlines |
| `e2e/tests/editor-features.spec.ts` | Modified | 5 new image upload E2E tests |


## Design Decisions

### No new npm packages required

ProseMirror provides `Decoration`, `DecorationSet`, and `Plugin` as built-in primitives. The placeholder decoration pattern uses only these core APIs — no additional packages were needed for the client-side upload UX.

### No new cargo crates required

Axum 0.7 includes built-in `Multipart` extractor and `DefaultBodyLimit` layer support. The only change was enabling the `"multipart"` feature flag in `Cargo.toml`, which pulls `encoding_rs` and `multer` as transitive dependencies. No new direct crate dependencies were added.

### Placeholder decorations over optimistic image insertion

Instead of inserting an `<img>` tag with a temporary `blob:` URL during upload (which would show a broken image on failure), the implementation uses a non-content placeholder decoration. This approach means upload failures leave no trace in the document — the placeholder simply disappears.

### Schema guard for plugin activation

The plugin factory checks `schema.nodes.image` before constructing the plugin. If the schema lacks an `image` node (as in raw mode), the factory returns `null`. This prevents paste/drop handlers from intercepting events in editing modes where images cannot be represented.

### Content-addressed filename generation

Pasted images typically lack filenames (the clipboard provides raw image data, not named files). The `generate_paste_filename()` function creates deterministic-style names using `paste-{unix_timestamp}.{ext}`. This provides human-readable names in the file list while keeping the content-addressed hash as the true identifier.

### Natural deduplication

Because iroh's blob store is content-addressed, uploading the same image twice produces the same hash and reuses the existing blob. The named tag is updated to point to the same hash, so storage is not wasted on duplicate uploads.


## Testing

### TypeScript Unit Tests (39 tests)

`web/src/image-upload.test.ts` covers:

- **MIME validation** — Verifying the 7 allowed types are accepted and others rejected
- **Extension mapping** — `mime_to_extension()` returns correct file extensions for each MIME type
- **Filename generation** — Pasted images receive `paste-{timestamp}.{ext}` names
- **Plugin creation** — Plugin is created when schema has `image` node, returns `null` without it
- **Upload function** — Mocked `fetch` tests verify FormData construction, success handling (image node insertion), and failure handling (placeholder cleanup)

### Rust Unit Tests (4 tests)

In `src/web/routes.rs`:

- `test_upload_response_serialization` — `UploadResponse` JSON output matches expected format
- `test_allowed_image_types` — All 7 MIME types are in the allowlist
- `test_rejected_image_types` — Non-image MIME types are correctly rejected
- `test_mime_to_extension` — Each allowed MIME type maps to the correct file extension

### Playwright E2E Tests (5 tests)

In `e2e/tests/editor-features.spec.ts`:

- Upload endpoint accepts valid image files and returns correct JSON response
- Upload endpoint rejects non-image files with appropriate error
- Blob URL returned by upload is accessible and serves the image
- Uploaded image appears in the file list with correct name
- Image renders correctly in the markdown editor view


## References

- [Original plan](../../thoughts/shared/plans/2026-04-01-image-upload.md)
- [Design document](../../thoughts/shared/designs/2026-04-01-image-upload-design.md)
- [Testing architecture](../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md)
- Content modes documentation: `docs/2026-03-21T05-47-48Z_feature_content_modes/`
- ProseMirror editor source: `web/src/editor.ts`
- Axum routes source: `src/web/routes.rs`
