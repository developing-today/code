# Plan: Content Modes, Format Conversion & Media Viewing

**Status**: In Progress
**Created**: 2026-03-21
**Last Updated**: 2026-03-21

## Progress

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Content Mode Infrastructure | ✅ Complete | `content_mode.rs` with 14 tests |
| Phase 2: Markdown Conversion | ✅ Complete | `markdown.rs` with 20 tests, comrak integration |
| Phase 3: Document Initialization | ✅ Complete | `collab.rs`: mode in Document, content_to_document(), Init message with mode |
| Phase 4: Client Mode Support | ✅ Complete | `editor.ts`: rawSchema, richSchema, mode-aware init; `collab.ts`: parse mode; 39 tests |
| Phase 5: Media Viewer | ✅ Complete | Blob serving, media/binary viewers, content-type detection, CSS styles |
| Phase 6: Save Functionality | 🔜 Deferred | User has thoughts on hash/naming |

## Summary

Implement intelligent content handling for the editor with **server-authoritative format conversion**:

1. Server detects file types by extension and determines content mode
2. Server converts formats (markdown ↔ ProseMirror JSON) - **client never sends untrusted content**
3. Wire protocol always uses ProseMirror JSON (steps) for collaboration
4. Media files render natively in browser (images, video, audio)
5. Raw mode for code/config files (no toolbar, no formatting shortcuts)
6. Save triggers server-side conversion back to original format

## Key Principle: Server as Authority

**The client cannot be trusted.** All format conversion happens server-side:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        SERVER-AUTHORITATIVE FLOW                            │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  OPEN FILE:                                                                 │
│    Server: load file from blob store                                        │
│    Server: detect mode by extension                                         │
│    Server: convert to ProseMirror JSON (if markdown/plain)                  │
│    Server → Client: Init [version, PM JSON doc, mode]                       │
│                                                                             │
│  COLLABORATION:                                                             │
│    Client → Server: Steps [version, steps, clientID]                        │
│    Server: validate steps against current state                             │
│    Server: apply valid steps to authoritative PM JSON                       │
│    Server → All Clients: Update [steps, clientIDs]                          │
│                                                                             │
│  SAVE:                                                                      │
│    Client → Server: Save command (no content!)                              │
│    Server: convert authoritative PM JSON → original format                  │
│    Server: write to blob store                                              │
│    Server → Client: Save confirmation                                       │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Finalized Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Format conversion location | **Server-side (Rust)** | Client cannot be trusted; server is authority |
| Markdown library | **comrak** | Full GFM, bidirectional (AST↔markdown), tree-based AST |
| Schema | **prosemirror-markdown compatible** | Must match PM schema for valid JSON |
| File type detection | Extension-based, server determines | Predictable; server controls behavior |
| Binary file handling | Native browser rendering for media | Images/video/audio shown natively |
| ProseMirror JSON extension | `.pm.json` | Clear intent |
| Unknown text files | Raw mode (editable) | Users can edit, no formatting |
| Save behavior | Server converts PM JSON → original format | Round-trip preserves format |

## Content Modes

| Mode | Extensions | Server Behavior | Client Behavior |
|------|------------|-----------------|-----------------|
| **Media** | `.png`, `.jpg`, `.gif`, `.webp`, `.svg`, `.mp4`, `.webm`, `.mp3`, `.wav`, `.pdf` | Serve blob with Content-Type | Render with `<img>`, `<video>`, etc. |
| **Rich** | `.pm.json` | Validate & load JSON directly | Full editor |
| **Markdown** | `.md`, `.markdown` | Parse markdown → PM JSON | Full editor |
| **Plain** | `.txt` | Convert lines → paragraphs | Full editor |
| **Raw** | `.js`, `.ts`, `.rs`, `.json`, `.toml`, `.yaml`, etc. | Wrap in single `code_block` | No toolbar, no shortcuts |
| **Binary** | Non-UTF8 files | Return mode + metadata only | Show "cannot display" + download |

## Server-Side Markdown Conversion (Rust)

### Library: comrak

**Why comrak:**
- 100% GitHub Flavored Markdown spec compliant
- Bidirectional: `parse_document()` and `format_commonmark()`
- Tree-based AST (easier to map to ProseMirror than event-based)
- Battle-tested: used by crates.io, docs.rs, GitLab, Reddit
- Supports: tables, task lists, strikethrough, footnotes, math

### Markdown → ProseMirror JSON

```rust
use comrak::{parse_document, Arena, Options};
use comrak::nodes::{AstNode, NodeValue};
use serde_json::{json, Value};

/// Convert markdown text to ProseMirror JSON document.
pub fn markdown_to_prosemirror(markdown: &str) -> Value {
    let arena = Arena::new();
    let options = gfm_options();
    let root = parse_document(&arena, markdown, &options);
    convert_node(root)
}

fn convert_node<'a>(node: &'a AstNode<'a>) -> Value {
    let data = node.data.borrow();
    match &data.value {
        NodeValue::Document => json!({
            "type": "doc",
            "content": children_to_json(node)
        }),
        NodeValue::Paragraph => json!({
            "type": "paragraph",
            "content": children_to_json(node)
        }),
        NodeValue::Heading(h) => json!({
            "type": "heading",
            "attrs": { "level": h.level },
            "content": children_to_json(node)
        }),
        NodeValue::Text(text) => json!({
            "type": "text",
            "text": text.as_ref()
        }),
        NodeValue::Emph => with_mark(node, "em"),
        NodeValue::Strong => with_mark(node, "strong"),
        NodeValue::Code(c) => json!({
            "type": "text",
            "text": c.literal.as_ref(),
            "marks": [{ "type": "code" }]
        }),
        NodeValue::CodeBlock(cb) => json!({
            "type": "code_block",
            "content": [{ "type": "text", "text": cb.literal.as_ref() }]
        }),
        NodeValue::BlockQuote => json!({
            "type": "blockquote",
            "content": children_to_json(node)
        }),
        NodeValue::List(list) => json!({
            "type": if list.list_type == ListType::Ordered { "ordered_list" } else { "bullet_list" },
            "content": children_to_json(node)
        }),
        NodeValue::Item(_) => json!({
            "type": "list_item",
            "content": children_to_json(node)
        }),
        NodeValue::Link(link) => json!({
            "type": "text",
            "text": get_text_content(node),
            "marks": [{ "type": "link", "attrs": { "href": link.url.as_ref() } }]
        }),
        NodeValue::Image(img) => json!({
            "type": "image",
            "attrs": { "src": img.url.as_ref(), "alt": img.title.as_ref() }
        }),
        NodeValue::SoftBreak | NodeValue::LineBreak => json!({
            "type": "hard_break"
        }),
        NodeValue::ThematicBreak => json!({
            "type": "horizontal_rule"
        }),
        // ... handle remaining node types
        _ => json!(null)
    }
}
```

### ProseMirror JSON → Markdown

```rust
use comrak::{format_commonmark, Arena, Options};
use comrak::nodes::{Ast, AstNode, NodeValue};

/// Convert ProseMirror JSON document to markdown text.
pub fn prosemirror_to_markdown(doc: &Value) -> Result<String, ConversionError> {
    let arena = Arena::new();
    let root = json_to_ast(&arena, doc)?;
    let mut output = Vec::new();
    format_commonmark(root, &gfm_options(), &mut output)?;
    Ok(String::from_utf8(output)?)
}

fn json_to_ast<'a>(arena: &'a Arena<AstNode<'a>>, json: &Value) -> Result<&'a AstNode<'a>, ConversionError> {
    let node_type = json["type"].as_str().ok_or(ConversionError::MissingType)?;

    let node_value = match node_type {
        "doc" => NodeValue::Document,
        "paragraph" => NodeValue::Paragraph,
        "heading" => {
            let level = json["attrs"]["level"].as_u64().unwrap_or(1) as u8;
            NodeValue::Heading(NodeHeading { level, setext: false })
        },
        "text" => {
            let text = json["text"].as_str().unwrap_or("");
            NodeValue::Text(text.to_string().into())
        },
        "code_block" => {
            let text = get_text_from_content(&json["content"]);
            NodeValue::CodeBlock(Box::new(NodeCodeBlock {
                literal: text.into(),
                info: String::new().into(),
                ..Default::default()
            }))
        },
        // ... handle remaining types
        _ => return Err(ConversionError::UnknownType(node_type.to_string())),
    };

    let ast_node = arena.alloc(AstNode::new(Ast::new(node_value)));

    // Recursively convert children
    if let Some(content) = json["content"].as_array() {
        for child_json in content {
            let child = json_to_ast(arena, child_json)?;
            ast_node.append(child);
        }
    }

    Ok(ast_node)
}
```

## ProseMirror Schema Alignment

The server must generate ProseMirror JSON that matches the client's schema. The `prosemirror-markdown` schema defines:

### Nodes

| Node | Content | Attributes |
|------|---------|------------|
| `doc` | `block+` | - |
| `paragraph` | `inline*` | - |
| `heading` | `inline*` | `level: 1-6` |
| `blockquote` | `block+` | - |
| `code_block` | `text*` | - |
| `horizontal_rule` | - | - |
| `bullet_list` | `list_item+` | - |
| `ordered_list` | `list_item+` | `order: number` |
| `list_item` | `paragraph block*` | - |
| `image` | - | `src, alt, title` |
| `hard_break` | - | - |
| `text` | - | - |

### Marks

| Mark | Attributes |
|------|------------|
| `em` | - |
| `strong` | - |
| `code` | - |
| `link` | `href, title` |

## Wire Protocol

No changes to the wire protocol structure. Mode is added to Init message:

```
Current:  [0, version, doc]
Proposed: [0, version, doc, mode]

mode: "rich" | "markdown" | "plain" | "raw" | "media" | "binary"
```

Client uses `mode` to configure:
- Toolbar visibility (`raw` = hidden)
- Keyboard shortcuts (`raw` = formatting shortcuts disabled)
- Save button behavior (`media`/`binary` = no save)

## Implementation Plan

### Phase 1: Content Mode Infrastructure ✅ COMPLETE

**1.1 Add comrak dependency** (`Cargo.toml`) ✅
```toml
[dependencies]
comrak = { version = "0.51", optional = true }

[features]
default = ["web"]
web = ["comrak", ...]
```

**1.2 Create content mode module** (`src/web/content_mode.rs`) ✅
- `ContentMode` enum with `Rich`, `Markdown`, `Plain`, `Raw`, `Media(MediaType)`, `Binary`
- `MediaType` enum with `Image`, `Video`, `Audio`, `Pdf`
- `detect_mode(filename: &str) -> ContentMode`
- `detect_mode_with_content(filename: &str, content: &[u8]) -> ContentMode`
- `is_valid_utf8()` helper
- 14 unit tests covering all file types

**1.3 Update routes** (`src/web/routes.rs`) ⏳ (Phase 3)
- Refactor `edit_handler` to use `detect_mode()`
- Add `blob_handler` for media files
- Route to appropriate template based on mode

### Phase 2: Markdown Conversion (Server-Side) ✅ COMPLETE

**2.1 Create markdown module** (`src/web/markdown.rs`) ✅
- `markdown_to_prosemirror(text: &str) -> serde_json::Value`
- `prosemirror_to_markdown(doc: &Value) -> Result<String, ConversionError>`
- `plain_text_to_prosemirror(text: &str) -> serde_json::Value`
- `raw_text_to_prosemirror(text: &str) -> serde_json::Value`
- Full CommonMark support (headings, bold, italic, code, links, images, lists, blockquotes, horizontal rules)
- Mark accumulation for nested inline formatting

**2.2 Create conversion tests** (`src/web/markdown.rs` tests) ✅
- 20 unit tests covering:
  - All node types (paragraph, heading, blockquote, code_block, lists, images, horizontal_rule)
  - All mark types (bold, italic, code, link, bold+italic combined)
  - Round-trip tests (md → json → md)
  - Plain text and raw text conversion
  - Edge cases (empty document)

### Phase 3: Document Initialization ✅ COMPLETE

**3.1 Update Document struct** (`src/web/collab.rs`) ✅
```rust
pub struct Document {
    // ... existing fields
    pub mode: ContentMode,
}
```

**3.2 Create content_to_document()** (`src/web/collab.rs`) ✅
- Detects mode from filename and content
- Converts to appropriate ProseMirror JSON format
- Returns (doc, mode) tuple

**3.3 Update Init message** (`src/web/collab.rs`) ✅
- Include mode in Init message: `[0, version, doc, mode]`

**3.4 Add filename query param** (`src/web/collab.rs`) ✅
- `WsParams` struct for `?filename=` parameter
- Filename passed through WebSocket handler

### Phase 4: Client Mode Support ✅ COMPLETE

**4.1 Create raw mode schema** (`web/src/editor.ts`) ✅
```typescript
export const rawSchema = new Schema({
  nodes: {
    doc: { content: 'code_block+' },
    code_block: {
      content: 'text*',
      marks: '',
      code: true,
      defining: true,
    },
    text: {},
  },
  marks: {},
});
```

**4.2 Mode-aware editor initialization** (`web/src/editor.ts`) ✅
```typescript
export type ContentMode = 'rich' | 'markdown' | 'plain' | 'raw' | 'media' | 'binary';

export function initEditor(
  container: HTMLElement,
  initialContent?: string,
  collabVersion?: number,
  mode: ContentMode = 'raw',
  sendCursor?: SendCursorFn
): EditorInstance {
  const schema = mode === 'raw' ? rawSchema : richSchema;
  const plugins = hasToolbar(mode) ? exampleSetup({ schema }) : minimalPlugins;
  // ...
}
```

**4.3 Update collab.ts** ✅
- Parse mode from Init message `msg[3]`
- Pass filename as `?filename=` query param
- Pass mode to `initEditor()`

**4.4 Update main.ts** ✅
- Extract `data-filename` from editor container
- Pass filename to `initCollab()`

**4.5 Add TypeScript tests** (`web/src/editor.test.ts`) ✅
- 39 tests for mode helpers, schema selection, schema structure

### Phase 5: Media Viewer ✅ COMPLETE

**5.1 Add get_content_type() function** (`src/web/content_mode.rs`) ✅
- Comprehensive MIME type detection by file extension
- Supports images (png, jpg, gif, webp, svg), video (mp4, webm, ogv), audio (mp3, wav, ogg), PDF
- Text types with charset (txt, md, html, css, js, json, xml)
- Default fallback to `application/octet-stream`

**5.2 Add blob route** (`src/web/routes.rs`) ✅
```rust
async fn blob_handler(
    Path(hash): Path<String>,
    Query(query): Query<BlobQuery>,
    State(state): State<AppState>,
) -> Response {
    // Serve raw bytes with proper Content-Type and cache headers
    // Cache-Control: public, max-age=31536000, immutable (content-addressed)
}
```

**5.3 Update edit_handler** (`src/web/routes.rs`) ✅
- Detects content mode from filename and content bytes
- Routes to `render_media_viewer()` for Media modes
- Routes to `render_binary_viewer()` for Binary mode
- Routes to `render_editor()` for editable modes

**5.4 Create media viewer template** (`src/web/templates.rs`) ✅
- `render_media_viewer(doc_id, name, media_type)` - renders `<img>`, `<video>`, `<audio>`, or `<embed>` for PDF
- Download button for all media types
- Back navigation link

**5.5 Create binary viewer template** (`src/web/templates.rs`) ✅
- `render_binary_viewer(doc_id, name)` - shows "cannot display" message
- Download button for binary files

**5.6 Add viewer styles** (`web/styles/editor.css`) ✅
- `.media-viewer` - centered flexbox container
- `.media-content` - responsive max-width/max-height
- `.media-pdf` - full width/height embed
- `.binary-viewer` - centered message container

### Phase 6: Save Functionality 🔜 DEFERRED

> **Note**: Deferred per user discussion. Files/URLs are based on hash, not filename. User has thoughts on how to manage this - implement last.

**6.1 Add save endpoint** (`src/web/routes.rs`)
```rust
async fn save_handler(
    Path(doc_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let doc = state.documents.get(&doc_id)?;
    let content = match doc.original_format {
        OriginalFormat::Markdown => prosemirror_to_markdown(&doc.doc.read().await)?,
        OriginalFormat::ProseMirror => serde_json::to_string(&doc.doc.read().await)?,
        OriginalFormat::Plain | OriginalFormat::Raw => extract_text(&doc.doc.read().await),
    };

    let new_hash = store_blob(&state.store, content.as_bytes()).await?;
    update_tag(&state.store, &doc_id, &new_hash).await?;

    Ok(Json(SaveResponse { hash: new_hash }))
}
```

**6.2 Add save wire message** (optional, or use HTTP)
- Could add `[7]` Save message to wire protocol
- Or use separate HTTP POST endpoint

**6.3 Wire up save button** (`web/src/main.ts`, `templates.rs`)
- Replace `alert('Save functionality coming soon')`
- Call save endpoint, show success/error

## File Changes Summary

### New Files

| File | Purpose | Status |
|------|---------|--------|
| `src/web/content_mode.rs` | ContentMode enum, extension detection, `get_content_type()` | ✅ Complete |
| `src/web/markdown.rs` | markdown↔ProseMirror conversion (comrak) | ✅ Complete |
| `web/src/editor.test.ts` | TypeScript tests for editor mode features | ✅ Complete |

### Modified Files

| File | Changes | Status |
|------|---------|--------|
| `Cargo.toml` | Add `comrak`, `urlencoding` dependencies | ✅ Complete |
| `src/web/mod.rs` | Add `content_mode`, `markdown` modules | ✅ Complete |
| `src/web/routes.rs` | Add `blob_handler`, refactor `edit_handler` with mode routing | ✅ Complete |
| `src/web/templates.rs` | Add `render_media_viewer`, `render_binary_viewer`, `data-filename` | ✅ Complete |
| `src/web/collab.rs` | Add mode to Document, content_to_document(), Init msg with mode | ✅ Complete |
| `web/src/editor.ts` | Mode-aware initialization, rawSchema, richSchema | ✅ Complete |
| `web/src/collab.ts` | Parse mode from Init, filename query param | ✅ Complete |
| `web/src/main.ts` | Pass filename to initCollab | ✅ Complete |
| `web/styles/editor.css` | Media viewer and binary viewer styles | ✅ Complete |

## Testing Plan

### Unit Tests (Rust)

- [x] `content_mode.rs`: Extension detection for all categories (14 tests)
- [x] `markdown.rs`: Markdown → PM JSON for all node types
- [x] `markdown.rs`: PM JSON → Markdown for all node types
- [x] `markdown.rs`: Round-trip tests (md → json → md)
- [ ] `markdown.rs`: GFM features (tables, task lists, strikethrough) - **Out of scope v1, CommonMark only**
- [x] `markdown.rs`: Edge cases (empty doc, nested lists, complex marks)
- [x] `collab.rs`: content_to_document() for all modes (7 tests)

### Unit Tests (TypeScript)

- [x] `editor.test.ts`: hasToolbar() for all modes (6 tests)
- [x] `editor.test.ts`: isEditable() for all modes (6 tests)
- [x] `editor.test.ts`: getSchema() returns correct schema (6 tests)
- [x] `editor.test.ts`: richSchema has all expected nodes and marks (7 tests)
- [x] `editor.test.ts`: rawSchema has minimal nodes, no marks (10 tests)
- [x] `editor.test.ts`: Schema compatibility - can create empty docs (4 tests)

### Integration Tests

- [ ] Load `.md` file → verify correct PM JSON structure
- [ ] Load `.pm.json` file → verify exact structure preserved
- [ ] Load `.js` file → verify raw mode (single code_block)
- [ ] Load `.png` file → verify image displays
- [ ] Save markdown file → verify markdown output matches
- [ ] Collaboration in markdown mode → verify steps work correctly

### Manual Testing

- [ ] Edit markdown file, add heading via toolbar, save, reload → verify persisted as markdown
- [ ] Open raw file, try Ctrl+B → verify nothing happens
- [ ] Open image, verify displays correctly
- [ ] Two users edit same markdown file → verify collaboration works
- [ ] Save during collaboration → verify both users see save succeed

## Complexity Estimate

| Phase | Effort | Risk |
|-------|--------|------|
| Phase 1: Content Mode Infrastructure | Low | Low |
| Phase 2: Markdown Conversion | **High** | Medium (schema alignment) |
| Phase 3: Document Initialization | Medium | Low |
| Phase 4: Client Mode Support | Medium | Low |
| Phase 5: Media Viewer | Low | Low |
| Phase 6: Save Functionality | Medium | Low |

**Total estimate**: 5-7 days

**Highest risk**: Phase 2 (markdown conversion) - ensuring the generated ProseMirror JSON exactly matches the client's schema. Extensive testing required.

## Future Enhancements (Out of Scope)

- Syntax highlighting in raw mode
- GFM table editing UI
- Format conversion menu ("Export as...")
- Conflict resolution on save (if file changed externally)
- WASM file execution
