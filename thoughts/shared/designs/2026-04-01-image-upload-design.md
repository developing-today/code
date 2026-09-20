---
date: 2026-04-01
topic: "Image Drag-Drop/Paste Upload"
status: validated
---

# Image Drag-Drop/Paste Upload

## Problem Statement

The rich/markdown editor has an `image` node in its ProseMirror schema (from `prosemirror-schema-basic`) and markdown round-trip already works for images (`![alt](src)` ↔ image node). However, there is **no way to add images through the web UI** — no paste handler, no drag-drop handler, no toolbar button, and no upload endpoint.

We need GitHub-style image paste/drop: user drops or pastes an image, it gets uploaded as a separate content-addressed blob, and an image node is inserted into the document referencing it by URL.

## Constraints

- **No base64 inline images** — they bloat document JSON, break collab (huge steps), and bypass content-addressing
- **Content-addressed storage** — images stored as iroh blobs, served via existing `/blob/:hash` endpoint with `Cache-Control: immutable`
- **Rich/markdown modes only** — raw mode has no image node in its schema
- **No new npm dependencies** — ProseMirror already has everything needed (schema, node types, transactions)
- **File size limit** — reject uploads above a configurable maximum (10MB default) client-side before upload
- **Image types only** — the upload endpoint validates MIME type (png, jpg, gif, webp, svg, bmp, ico)

## Approach

Two-layer approach: a new server upload endpoint + a client-side ProseMirror plugin.

**Why separate blobs?** They're immutable, cacheable forever (existing `/blob/:hash` returns `Cache-Control: immutable`), and deduplicated automatically by iroh's content-addressed store. An image pasted twice stores one blob.

**Why not reuse `/api/save`?** Save expects ProseMirror JSON, not binary files. A dedicated upload endpoint keeps concerns separate and supports multipart/form-data which is the standard for file uploads.

## Architecture

### Server: `POST /api/upload`

New route in `src/web/routes.rs`:

- Accepts `multipart/form-data` with a single file field
- Validates: file is an image MIME type, size under limit
- Stores bytes via `state.store.blobs().add_bytes(file_bytes)`
- Creates a tag: `state.store.tags().set(filename, hash)`
- Sets metadata tags (`created`, `modified`) via `state.tag_store`
- Returns JSON: `{ "hash": "...", "name": "...", "url": "/blob/{hash}?filename={name}" }`

Filename generation for clipboard pastes (no original name): `paste-{unix_timestamp_ms}.{ext}` where ext is derived from MIME type.

### Client: `image-upload.ts` plugin

New ProseMirror plugin registered in `initEditor()` for rich/markdown/plain modes:

- **Paste handler**: `handlePaste` prop — checks `clipboardData.files` for image MIME types
- **Drop handler**: `handleDrop` prop — checks `dataTransfer.files` for image MIME types
- **Upload flow**: `File` → `FormData` → `fetch("/api/upload")` → response with URL
- **Node insertion**: creates `schema.nodes.image.create({ src: url, alt: filename })` and inserts at cursor/drop position
- **Placeholder**: insert a widget decoration with a loading indicator during upload, replace with real image node on success

### Toolbar: Image insert button

Add image button to Row 1 of the rich mode toolbar menu (after toggleLink):
- Opens a hidden `<input type="file" accept="image/*">` dialog
- On file selection, runs the same upload+insert flow
- This covers the non-paste/non-drop use case

### CSS: Section 17 in `editor-compat.css`

- `.image-upload-placeholder` — pulsing outline placeholder during upload
- `.ProseMirror img` — max-width: 100%, display: block, border-radius, margin

## Components

| Component       | File                      | Responsibility                                           |
| --------------- | ------------------------- | -------------------------------------------------------- |
| Upload endpoint | `src/web/routes.rs`       | Accept multipart, validate, store blob, return URL       |
| Upload plugin   | `web/src/image-upload.ts` | Handle paste/drop, upload file, insert image node        |
| CSS             | `web/src/editor-compat.css` §17 | Upload placeholder, image display styling          |
| Editor wiring   | `web/src/editor.ts`       | Register plugin + toolbar button for rich/markdown modes |

## Data Flow

### Image paste/drop flow

```
User pastes/drops image file
  → image-upload.ts handlePaste/handleDrop intercepts event
  → Extracts File from clipboardData/dataTransfer
  → Validates: isImage(file.type) && file.size < MAX_SIZE
  → Inserts placeholder decoration at cursor/drop position
  → fetch("POST /api/upload", FormData { file })
  → Server: store.blobs().add_bytes(bytes) → hash
  → Server: store.tags().set("paste-{ts}.png", hash)
  → Server returns { url: "/blob/{hash}?filename=paste-{ts}.png" }
  → Plugin: dispatch transaction replacing placeholder with image node
  → Image renders inline via <img src="/blob/{hash}?filename=...">
```

### Markdown round-trip (already works, no changes needed)

```
Load .md:  ![alt](/blob/hash?f=x) → markdown_to_prosemirror() → image node {src, alt}
Save .md:  image node → prosemirror_to_markdown() → ![alt](/blob/hash?f=x)
```

## Error Handling

- **Upload fails** (network error, server error): remove placeholder decoration, log warning to console
- **File too large**: reject client-side before upload, remove from event — no network request
- **Non-image file**: ignore the file, let ProseMirror's default behavior handle it
- **Invalid MIME on server**: return 400 with error message
- **Duplicate upload**: iroh deduplicates automatically — same bytes = same hash, new tag points to existing blob

## Testing Strategy

### Unit tests (`web/src/image-upload.test.ts`)

- Plugin creation and registration
- MIME type validation (accept images, reject non-images)
- File size validation
- Placeholder decoration insertion and removal
- Image node creation with correct attributes
- FormData construction

### Rust unit tests (in `src/web/routes.rs`)

- Upload handler: valid image upload → 200 with hash/url
- Upload handler: non-image MIME → 400
- Upload handler: file too large → 413
- Upload handler: empty body → 400
- Filename generation for clipboard pastes

### E2E tests (in `e2e/tests/editor-features.spec.ts`)

- Create image blob via API, insert image node via ProseMirror, verify `<img>` renders
- Image src points to valid `/blob/` URL
- Image persists after save and reload

## Open Questions

- Should we support image resize handles in the editor? (Probably YAGNI for now — can add later)
- Should pasted images be auto-compressed/resized? (No — store originals, let browser handle display)
