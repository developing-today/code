# Content Modes, Format Conversion & Media Viewing

See [original plan](../../.opencode/plans/prosemirror-content-modes.md)

## Implementation Status

| Phase | Status | Location |
|-------|--------|----------|
| Phase 1: Content Mode Infrastructure | ✅ Complete | `src/web/content_mode.rs` |
| Phase 2: Markdown Conversion | ✅ Complete | `src/web/markdown.rs` |
| Phase 3: Document Initialization | ✅ Complete | `src/web/collab.rs` |
| Phase 4: Client Mode Support | ✅ Complete | `web/src/editor.ts`, `web/src/collab.ts` |
| Phase 5: Media Viewer | ✅ Complete | `src/web/routes.rs`, `src/web/templates.rs` |
| Phase 6: Save Functionality | 🔜 Deferred | - |

## Overview

This feature implements intelligent content handling for the ProseMirror editor based on file type detection. The system supports multiple content modes: rich text editing (for ProseMirror JSON), markdown editing (with server-side conversion), plain text, raw code/config files, and media viewing.

### Key Principles

1. **Server as Authority**: All format conversion happens server-side in Rust. The client never sends document content that the server trusts blindly.

2. **CommonMark First**: Initial implementation supports CommonMark markdown. GFM features (tables, strikethrough, task lists) are out of scope for v1.

3. **Mode-Specific Editor Configuration**: The editor adapts its UI (toolbar, shortcuts) based on content mode.

## Content Modes

| Mode | Extensions | Description |
|------|------------|-------------|
| **Media** | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`, `.mp4`, `.webm`, `.ogg`, `.mp3`, `.wav`, `.pdf` | Native browser rendering, no editing |
| **Rich** | `.pm.json` | Full editor with native ProseMirror JSON |
| **Markdown** | `.md`, `.markdown` | Full editor, server converts markdown ↔ PM JSON |
| **Plain** | `.txt` | Full editor, lines become paragraphs |
| **Raw** | `.js`, `.ts`, `.rs`, `.py`, `.json`, `.toml`, `.yaml`, etc. | Editor with no toolbar, no formatting shortcuts |
| **Binary** | Non-UTF8 files | "Cannot display" message + download link |

## Architecture

### Server-Authoritative Flow

```
OPEN FILE:
  Server: load file from blob store
  Server: detect mode by extension
  Server: convert to ProseMirror JSON (if markdown/plain)
  Server → Client: Init [version, PM JSON doc, mode]

COLLABORATION:
  Client → Server: Steps [version, steps, clientID]
  Server: validate steps against current state
  Server: apply valid steps to authoritative PM JSON
  Server → All Clients: Update [steps, clientIDs]

SAVE (future):
  Client → Server: Save command (no content!)
  Server: convert authoritative PM JSON → original format
  Server: write to blob store
```

### Wire Protocol

Init message extended with mode:

```
Current:  [0, version, doc]
New:      [0, version, doc, mode]

mode: "rich" | "markdown" | "plain" | "raw" | "media" | "binary"
```

## Markdown Conversion (Server-Side Rust)

### Library: comrak

Using `comrak` for CommonMark parsing and serialization:
- Full CommonMark spec compliance
- Bidirectional: `parse_document()` and `format_commonmark()`
- Tree-based AST maps well to ProseMirror's tree structure

### Node Mapping

| Comrak NodeValue | ProseMirror Node/Mark |
|------------------|----------------------|
| `Document` | `doc` |
| `Paragraph` | `paragraph` |
| `Heading(level)` | `heading` {level} |
| `BlockQuote` | `blockquote` |
| `CodeBlock(info, literal)` | `code_block` {params} |
| `ThematicBreak` | `horizontal_rule` |
| `List(Ordered)` | `ordered_list` {order, tight} |
| `List(Bullet)` | `bullet_list` {tight} |
| `Item` | `list_item` |
| `Text` | `text` |
| `SoftBreak` | (space or ignored) |
| `LineBreak` | `hard_break` |
| `Emph` | mark: `em` |
| `Strong` | mark: `strong` |
| `Code(literal)` | mark: `code` |
| `Link(url, title)` | mark: `link` {href, title} |
| `Image(url, title)` | `image` {src, alt, title} |

### Mark Accumulation

ProseMirror uses marks on text nodes rather than wrapper elements. The conversion tracks active marks while traversing inline content:

```rust
// Comrak structure:
// Strong -> Emph -> Text("bold italic")

// ProseMirror structure:
// { "type": "text", "text": "bold italic", "marks": [{"type": "strong"}, {"type": "em"}] }
```

### Unsupported GFM Features (v1)

These features are parsed by comrak but stripped/passed through as text:
- Tables → preserved as pipe-delimited text
- Strikethrough → text without formatting
- Task lists → regular list items
- Footnotes → stripped
- Math → preserved as code

## Client-Side Changes

### Editor Modes

```typescript
type EditorMode = 'rich' | 'markdown' | 'plain' | 'raw';
```

### Raw Mode

For code/config files, the editor uses:
- Minimal schema: `doc` → `code_block` → `text`
- No marks allowed
- No toolbar (hidden via CSS)
- No formatting shortcuts (Ctrl+B, Ctrl+I disabled)
- Preserves whitespace exactly

### Schema

Using `prosemirror-markdown` schema for rich/markdown/plain modes (already includes lists).

## File Structure

### New Files (Implemented)

| File | Purpose | Status |
|------|---------|--------|
| `src/web/content_mode.rs` | ContentMode enum, extension detection | ✅ Complete (14 tests) |
| `src/web/markdown.rs` | markdown ↔ ProseMirror JSON conversion | ✅ Complete (20 tests) |
| `web/src/editor.test.ts` | TypeScript tests for editor mode features | ✅ Complete (39 tests) |
| `web/styles/viewer.css` | Media viewer styles | ⏳ Pending |

### Modified Files

| File | Changes | Status |
|------|---------|--------|
| `Cargo.toml` | Add `comrak`, `urlencoding` dependencies | ✅ Complete |
| `src/web/mod.rs` | Add modules | ✅ Complete |
| `src/web/routes.rs` | Add blob handler, refactor edit handler | ⏳ Pending |
| `src/web/templates.rs` | Add `data-filename` attribute to editor | ✅ Complete |
| `src/web/collab.rs` | Mode-aware document initialization, Init message with mode | ✅ Complete |
| `web/src/editor.ts` | Mode-aware editor setup, rawSchema, richSchema | ✅ Complete |
| `web/src/collab.ts` | Parse mode from Init, pass filename as query param | ✅ Complete |
| `web/src/main.ts` | Pass filename to initCollab | ✅ Complete |
| `web/styles/editor.css` | Raw mode toolbar hiding | ⏳ Pending |

## Implementation Phases

1. **Content Mode Infrastructure**: Detection, routing, templates ✅
2. **Markdown Conversion**: comrak integration, PM JSON generation ✅
3. **Document Initialization**: Mode in collab protocol, content conversion ✅
4. **Client Mode Support**: Raw schema, mode-aware initialization ✅
5. **Media Viewer**: Blob serving, native rendering ⏳
6. **Save Functionality**: (Deferred - requires hash/naming discussion) 🔜

## Phase 3 & 4 Implementation Details

### Server-Side Changes (Phase 3)

**`src/web/collab.rs`:**
- Added `ContentMode` to `Document` struct
- Created `content_to_document()` function that:
  - Detects mode from filename extension and content
  - Converts markdown files to ProseMirror JSON using `markdown_to_prosemirror()`
  - Converts plain text to paragraphs using `plain_text_to_prosemirror()`
  - Wraps raw/code files in single code_block using `raw_text_to_prosemirror()`
  - Returns empty doc for non-editable modes (media/binary)
- Extended `CollabMessage::Init` to include mode: `[0, version, doc, mode]`
- Added `WsParams` struct for parsing `?filename=` query parameter
- 7 new tests for content conversion

**`src/web/templates.rs`:**
- Added `data-filename` attribute to editor container (URL-encoded)

**`Cargo.toml`:**
- Added `urlencoding = "2"` dependency

### Client-Side Changes (Phase 4)

**`web/src/editor.ts`:**
- Added `ContentMode` type: `'rich' | 'markdown' | 'plain' | 'raw' | 'media' | 'binary'`
- Created `rawSchema`: minimal schema for code/config files
  - Only allows: `doc` → `code_block+` → `text*`
  - No marks (formatting) allowed
  - Sets `code: true` and `marks: ''` on code_block
- Renamed existing schema to `richSchema` (also exported as `schema` for compatibility)
- Added helper functions: `getSchema(mode)`, `hasToolbar(mode)`, `isEditable(mode)`
- Updated `initEditor()`:
  - Now accepts `mode` parameter
  - Selects schema based on mode (raw vs rich)
  - Configures plugins: full `exampleSetup` for toolbar modes, minimal plugins for raw
  - Adds mode-specific CSS class (`id-editor-rich` vs `id-editor-raw`)
- `EditorInstance` now includes `mode` field

**`web/src/collab.ts`:**
- Updated wire protocol docs to show `[0, version, doc, mode]`
- Added `filename` parameter to `initCollab()` - appended as `?filename=` query param
- Added `documentMode` tracking
- Updated Init handler to parse mode from `msg[3]`
- Updated Update handler to use schema from editor instance
- `CollabConnection` now includes `mode` getter
- Fixed reconnect to preserve filename param

**`web/src/main.ts`:**
- Extracts `data-filename` from editor container (URL-decodes it)
- Passes filename to `initCollab()`
- Logs editor mode on initialization

**`web/src/editor.test.ts`:** (new file)
- 39 tests covering:
  - `hasToolbar()` and `isEditable()` for all modes
  - `getSchema()` returns correct schema for each mode
  - `richSchema` structure (nodes, marks, lists)
  - `rawSchema` structure (only doc, code_block, text; no marks)
  - Schema compatibility (can create empty docs)

### Server-Side Changes (Phase 5)

**`src/web/content_mode.rs`:**
- Added `get_content_type(filename)` function for MIME type resolution
  - Comprehensive mapping for images (png, jpg, gif, webp, svg)
  - Video types (mp4, webm, ogv, mov, avi)
  - Audio types (mp3, wav, ogg, flac, aac, m4a)
  - Text types with UTF-8 charset (txt, md, html, css, js, json, xml)
  - PDF, default fallback to `application/octet-stream`
- 6 new tests for content type detection

**`src/web/routes.rs`:**
- Added `/blob/:hash` route with `blob_handler()`:
  - Serves raw file bytes from blob store
  - Sets Content-Type from filename (via `?filename=` query param or tag lookup)
  - Immutable caching: `Cache-Control: public, max-age=31536000, immutable`
- Refactored `edit_handler()` to route by content mode:
  - Media files → `render_media_viewer()`
  - Binary files → `render_binary_viewer()`
  - Editable files → `render_editor()`
- Added `BlobQuery` struct for `?filename=` query parameter
- Added `get_file_bytes()` helper returning `Result<Vec<u8>, String>`

**`src/web/templates.rs`:**
- Added `render_media_viewer(doc_id, name, media_type)`:
  - Image: `<img>` with alt text
  - Video: `<video controls>` with fallback message
  - Audio: `<audio controls>` with fallback message
  - PDF: `<embed type="application/pdf">` for native PDF.js rendering
  - Download button for all types
- Added `render_binary_viewer(doc_id, name)`:
  - "Cannot display" message
  - Download button

**`web/styles/editor.css`:**
- Added `.media-viewer` styles:
  - Centered flexbox container
  - Min-height 400px
  - Background and border styling
- Added `.media-content` styles:
  - Responsive max-width/max-height (100%/70vh)
  - Video/audio max-width 800px
  - PDF full width, 70vh height
- Added `.binary-viewer` styles:
  - Centered text, padding

## Testing Strategy

### Unit Tests (Rust) ✅ Complete
- Extension → mode detection (14 tests in `content_mode.rs`)
- Content-type detection (6 tests in `content_mode.rs`)
- Markdown → PM JSON for all CommonMark nodes (20 tests in `markdown.rs`)
- PM JSON → Markdown round-trips
- Edge cases: empty docs, nested structures, mark combinations

### Unit Tests (TypeScript) ✅ Complete
- 39 tests for mode helpers and schemas (in `editor.test.ts`)

### Integration Tests ⏳ Pending
- Load `.md` file → verify structure
- Load `.pm.json` → verify exact preservation
- Load `.js` → verify raw mode
- Load image → verify display
- Collaboration in markdown mode

## Out of Scope (v1)

- GFM tables, strikethrough, task lists
- Syntax highlighting in raw mode
- Save functionality (deferred)
- Format conversion UI ("Export as...")

## References

- [Original Plan](../../.opencode/plans/prosemirror-content-modes.md)
- [prosemirror-markdown schema](https://github.com/ProseMirror/prosemirror-markdown/blob/master/src/schema.ts)
- [comrak documentation](https://docs.rs/comrak)
- [CommonMark spec](https://spec.commonmark.org/)
