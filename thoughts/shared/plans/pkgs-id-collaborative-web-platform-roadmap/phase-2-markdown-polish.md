# Phase 2: Markdown Polish

## Problem Statement

The editor currently supports only basic CommonMark — no tables, strikethrough, or task lists. GFM content pasted or loaded from `.md` files silently degrades: tables become empty paragraphs, strikethrough text loses formatting, task list checkboxes vanish. Additionally, images inserted via paste/drop have no way to edit alt-text after insertion, no gallery to browse previously uploaded images, and no resize handles for adjusting dimensions.

## Architecture Changes

### Current flow (markdown ↔ ProseMirror)

```
.md file → comrak (CommonMark only, GFM disabled) → PM JSON (basic nodes/marks)
PM JSON → comrak → .md file (GFM features lost on round-trip)
```

**Schema**: `prosemirror-schema-basic` + `prosemirror-schema-list`
- Nodes: doc, paragraph, text, blockquote, horizontal_rule, heading, code_block, image, hard_break, ordered_list, bullet_list, list_item
- Marks: link, em, strong, code

### Target flow

```
.md file → comrak (GFM extensions enabled) → PM JSON (GFM nodes/marks)
PM JSON → comrak (GFM extensions enabled) → .md file (full round-trip)
```

**Schema additions**:
- Nodes: `table`, `table_row`, `table_cell`, `table_header`, `task_list`, `task_list_item`
- Marks: `strikethrough`
- Modified: `image` node gains `width`, `height` attrs

Both sides (Rust `markdown.rs` + TypeScript `editor.ts`) must stay in sync — any new PM node/mark needs conversion logic in `markdown.rs` AND a schema definition in `editor.ts`.

---

## Part 1: GFM Strikethrough

**What**: Add `~~text~~` support — the simplest GFM extension. A new mark type, comrak option, bidirectional conversion, and a toolbar button.

**Files**:
- `pkgs/id/src/web/markdown.rs` — Enable comrak strikethrough extension, add conversion in both directions
- `pkgs/id/web/src/editor.ts` — Add `strikethrough` mark to `richSchema`, add toolbar button, add keymap

**Changes**:

### markdown.rs

1. In `commonmark_options()` (~line 64-70), enable strikethrough:
   ```rust
   options.extension.strikethrough = true;
   ```

2. In `convert_node()`, replace the strikethrough fallback (~line 332-335) with mark-aware conversion:
   ```rust
   NodeValue::Strikethrough => {
       let mut child_marks = parent_marks.to_vec();
       child_marks.push(json!({"type": "strikethrough"}));
       let children = convert_children(node, &child_marks);
       return children; // marks attach to text nodes, not wrapper
   }
   ```

3. In `json_to_ast()` (~line 481+), add PM→comrak for the `strikethrough` mark. In `create_marked_text()` (~line 595), handle `"strikethrough"` → wrap in `NodeValue::Strikethrough`.

### editor.ts

1. Extend `richSchema` to include a `strikethrough` mark:
   ```typescript
   const strikethroughMark = {
     strikethrough: {
       parseDOM: [{ tag: "s" }, { tag: "del" }, { tag: "strike" }, { style: "text-decoration=line-through" }],
       toDOM() { return ["s", 0] as const; },
     },
   };
   ```
   Merge into schema marks alongside the basic marks.

2. Add toolbar button in row 1 (after `code`): `markItem(schema.marks.strikethrough, { title: "Strikethrough", icon: { text: "S̶", css: "" } })`

3. Add keymap: `Mod-Shift-s` → `toggleMark(schema.marks.strikethrough)`

**Test spec**:
- E2E: Create `.md` file with `~~deleted~~`, open in editor, verify strikethrough renders. Toggle strikethrough via toolbar, save, verify `~~deleted~~` in markdown output.
- Unit (Rust): `markdown_to_prosemirror("~~deleted~~")` produces text node with `strikethrough` mark. Round-trip preserves `~~text~~`.

---

## Part 2: GFM Task Lists

**What**: Add `- [ ] todo` / `- [x] done` checkbox support. A new node type that extends list behavior with a `checked` attribute.

**Files**:
- `pkgs/id/src/web/markdown.rs` — Enable comrak tasklist extension, convert TaskItem with `checked` attr
- `pkgs/id/web/src/editor.ts` — Add `task_list` and `task_list_item` nodes, checkbox nodeView, click-to-toggle

**Changes**:

### markdown.rs

1. In `commonmark_options()`, enable tasklist:
   ```rust
   options.extension.tasklist = true;
   ```

2. Replace TaskItem fallback (~line 319-330). TaskItem is a variant of list_item — comrak represents it as `NodeValue::TaskItem` with a `checked: bool` field:
   ```rust
   NodeValue::TaskItem(checked) => {
       let children = convert_children(node, parent_marks);
       json!({
           "type": "task_list_item",
           "attrs": { "checked": checked },
           "content": children
       })
   }
   ```

3. For the parent list: when a list's children are all TaskItems, emit `task_list` instead of `bullet_list`. Check inside the `List` handler — if first child is `TaskItem`, use `"type": "task_list"`.

4. In `json_to_ast()`, handle `"task_list"` → `NodeValue::List` (bullet) and `"task_list_item"` → `NodeValue::TaskItem(checked)`. Read `attrs.checked` boolean.

### editor.ts

1. Add `task_list` node to schema (group: `block`, content: `task_list_item+`):
   ```typescript
   task_list: {
     group: "block",
     content: "task_list_item+",
     parseDOM: [{ tag: "ul.task-list" }],
     toDOM() { return ["ul", { class: "task-list" }, 0]; },
   }
   ```

2. Add `task_list_item` node (content: `paragraph block*`, attrs: `{ checked: { default: false } }`):
   ```typescript
   task_list_item: {
     content: "paragraph block*",
     attrs: { checked: { default: false } },
     defining: true,
     parseDOM: [{
       tag: "li.task-list-item",
       getAttrs(dom) {
         const checkbox = dom.querySelector("input[type=checkbox]");
         return { checked: checkbox?.checked || false };
       },
     }],
     toDOM(node) {
       return ["li", { class: `task-list-item ${node.attrs.checked ? "checked" : ""}` }, 0];
     },
   }
   ```

3. Create a `taskListItemNodeView` that renders a checkbox before the content. Clicking the checkbox dispatches a transaction toggling the `checked` attr. The checkbox is `contentEditable: false` to avoid ProseMirror selection issues.

4. Add CSS for `.task-list` (no list-style) and `.task-list-item` (checkbox alignment).

5. Register the nodeView in the editor plugins array inside `initEditor()`.

**Test spec**:
- E2E: Create `.md` with `- [ ] buy milk\n- [x] write code`, verify checkboxes render. Click unchecked item, verify it toggles. Save, verify `- [x] buy milk` in output.
- Unit (Rust): Round-trip `- [ ] unchecked\n- [x] checked` preserves checkbox state.

---

## Part 3: GFM Tables

**What**: Add pipe-table support. This is the most complex part — requires a new npm dependency (`prosemirror-tables`), 5 new node types, table editing commands, and bidirectional comrak conversion.

**Files**:
- `pkgs/id/web/package.json` — Add `prosemirror-tables` dependency
- `pkgs/id/web/src/editor.ts` — Add table nodes to schema, table plugins, table menu items
- `pkgs/id/src/web/markdown.rs` — Enable comrak table extension, convert Table/TableRow/TableCell/TableHeader
- `pkgs/id/web/src/table-commands.ts` (new) — Table insertion/editing commands for toolbar

**Changes**:

### package.json

```
npm install prosemirror-tables
```

### markdown.rs

1. In `commonmark_options()`, enable table:
   ```rust
   options.extension.table = true;
   ```

2. Replace Table fallback (~line 314-317). Comrak table structure: `Table` → `TableRow` children → `TableCell` children. The `Table` node carries column alignments. The first row is the header row.

   ```rust
   NodeValue::Table(alignments) => {
       let children = convert_children(node, parent_marks);
       // First child row is header, rest are body rows
       json!({
           "type": "table",
           "content": children
       })
   }
   NodeValue::TableRow(is_header) => {
       let children = convert_children(node, parent_marks);
       json!({
           "type": "table_row",
           "content": children
       })
   }
   NodeValue::TableCell => {
       let children = convert_children(node, parent_marks);
       // Determine if this is in a header row to pick table_header vs table_cell
       // Use context or check parent's is_header
       let cell_type = if is_header_context { "table_header" } else { "table_cell" };
       json!({
           "type": cell_type,
           "content": if children.is_empty() {
               vec![json!({"type": "paragraph"})]
           } else {
               children
           }
       })
   }
   ```

   Note: `prosemirror-tables` requires cell content to be block-level (paragraph), but comrak table cells contain inline content. Wrap inline content in a paragraph node during conversion.

3. In `json_to_ast()`, handle `"table"`, `"table_row"`, `"table_header"`, `"table_cell"` → corresponding comrak `NodeValue` variants. Extract alignments from cell attrs if present, or default to `None`.

### editor.ts

1. Add table nodes to schema. The `prosemirror-tables` package provides `tableNodes()` helper but we define manually for control:
   ```typescript
   table: { content: "table_row+", group: "block", tableRole: "table", ... }
   table_row: { content: "(table_cell | table_header)*", tableRole: "row", ... }
   table_cell: { content: "block+", attrs: { colspan, rowspan, colwidth }, tableRole: "cell", ... }
   table_header: { content: "block+", attrs: { colspan, rowspan, colwidth }, tableRole: "header_cell", ... }
   ```

2. Add `prosemirror-tables` plugins: `columnResizing()`, `tableEditing()` in `initEditor()` plugins array.

3. Add table commands to toolbar row 3: Insert Table (3×3 default), then rely on `prosemirror-tables` context menu / keyboard for add row/col, delete row/col, merge cells.

4. Add `prosemirror-tables` CSS import for cell selection styling and resize handles.

### table-commands.ts (new file)

Wrapper commands for the toolbar:
- `insertTable(rows, cols)` — creates a table node and inserts at cursor
- Re-export relevant `prosemirror-tables` commands: `addColumnAfter`, `addRowAfter`, `deleteColumn`, `deleteRow`, `deleteTable`

**Test spec**:
- E2E: Create `.md` with a pipe table, open in editor, verify table renders with cells. Edit cell content, add a row via command, save, verify table markdown output.
- Unit (Rust): Round-trip a 2×3 table with header row preserves structure and cell content. Round-trip table with inline formatting (bold, links) in cells.

---

## Part 4: Image Alt-Text Editing

**What**: Add a UI to edit the `alt` attribute on existing image nodes. Currently alt-text is set at upload time but cannot be changed afterward.

**Files**:
- `pkgs/id/web/src/editor.ts` — Custom image nodeView with alt-text popover
- `pkgs/id/web/src/image-node-view.ts` (new) — Image nodeView class

**Changes**:

### image-node-view.ts (new file)

Create an `ImageNodeView` class implementing ProseMirror's `NodeView` interface:

1. **Render**: `<figure>` wrapper containing `<img>` element.
2. **Selection UI**: When the image node is selected (or clicked), show a floating toolbar/popover above/below the image with:
   - An "Alt text" input field, pre-populated with current `node.attrs.alt`
   - A "Save" button (or blur/Enter to commit)
3. **Update**: On commit, dispatch a transaction that sets the `alt` attr on the image node:
   ```typescript
   const tr = view.state.tr.setNodeMarkup(getPos(), null, { ...node.attrs, alt: newAlt });
   view.dispatch(tr);
   ```
4. **Destroy**: Clean up popover DOM on node view destruction.

### editor.ts

1. Register the `ImageNodeView` for the `image` node type in the editor's `nodeViews` option:
   ```typescript
   nodeViews: {
     image(node, view, getPos) { return new ImageNodeView(node, view, getPos); },
   }
   ```
2. This replaces ProseMirror's default image rendering, so the nodeView must also handle basic display (src, alt as tooltip, title).

**Test spec**:
- E2E: Upload image, click on it, verify alt-text popover appears. Change alt text, blur, verify the node's alt attr updated. Save as `.md`, verify `![new alt text](url)` in output.

---

## Part 5: Image Resize Handles

**What**: Allow users to drag image corners/edges to resize. Persist `width` and `height` in the ProseMirror document and markdown output.

**Files**:
- `pkgs/id/web/src/editor.ts` — Extend image node schema with `width`/`height` attrs
- `pkgs/id/web/src/image-node-view.ts` — Add resize handles to the nodeView from Part 4
- `pkgs/id/src/web/markdown.rs` — Serialize/deserialize image dimensions (HTML `<img>` tag in markdown)

**Changes**:

### editor.ts

1. Override the `image` node from `prosemirror-schema-basic` with a custom definition that adds `width` and `height` attrs:
   ```typescript
   image: {
     inline: true,
     group: "inline",
     draggable: true,
     attrs: {
       src: {},
       alt: { default: null },
       title: { default: null },
       width: { default: null },
       height: { default: null },
     },
     parseDOM: [{
       tag: "img[src]",
       getAttrs(dom) {
         return {
           src: dom.getAttribute("src"),
           alt: dom.getAttribute("alt"),
           title: dom.getAttribute("title"),
           width: dom.getAttribute("width") ? Number(dom.getAttribute("width")) : null,
           height: dom.getAttribute("height") ? Number(dom.getAttribute("height")) : null,
         };
       },
     }],
     toDOM(node) {
       const attrs: Record<string, string> = { src: node.attrs.src };
       if (node.attrs.alt) attrs.alt = node.attrs.alt;
       if (node.attrs.title) attrs.title = node.attrs.title;
       if (node.attrs.width) attrs.width = String(node.attrs.width);
       if (node.attrs.height) attrs.height = String(node.attrs.height);
       return ["img", attrs];
     },
   }
   ```

### image-node-view.ts

Extend the `ImageNodeView` (from Part 4) with resize handle behavior:

1. Add 4 corner handles (small squares positioned at corners of the `<img>` via CSS absolute positioning).
2. On mousedown on a handle, start tracking drag. On mousemove, compute new width/height maintaining aspect ratio.
3. On mouseup, dispatch transaction:
   ```typescript
   const tr = view.state.tr.setNodeMarkup(getPos(), null, { ...node.attrs, width: newW, height: newH });
   view.dispatch(tr);
   ```
4. During drag, apply temporary inline styles for visual feedback (don't commit until mouseup).
5. CSS: `.image-resize-handle` positioned absolutely, cursor styles for each corner.

### markdown.rs

For images with `width`/`height`, comrak doesn't natively support sized images in markdown syntax. Two strategies:

**Strategy: HTML img fallback**. When `width` or `height` is set, serialize as an HTML `<img>` tag instead of `![alt](src)`:
```rust
// In json_to_ast(), when handling "image" node:
if has_width_or_height {
    // Emit as HtmlInline: <img src="..." alt="..." width="..." height="..." />
} else {
    // Standard ![alt](src "title") syntax
}
```

On parse, comrak's `HtmlBlock`/`HtmlInline` handling already captures `<img>` tags — add a check in the HTML handler to detect `<img>` tags and convert them to image nodes with size attrs.

**Alternative considered**: Custom markdown syntax like `![alt](src =WxH)`. Rejected because it's non-standard and won't render in other markdown viewers.

**Test spec**:
- E2E: Upload image, drag a corner handle to resize, verify image dimensions change. Save as `.md`, verify `<img>` tag with width/height in output. Reload file, verify dimensions preserved.
- Unit (Rust): Round-trip image with width/height through `<img>` tag serialization. Image without dimensions uses standard `![alt](src)` syntax.

---

## Part 6: Image Browser

**What**: A dialog/panel that lists all images previously uploaded to the current document's blob store, allowing the user to pick and insert one.

**Files**:
- `pkgs/id/src/web/routes.rs` — Add `GET /api/images` endpoint listing uploaded image blobs
- `pkgs/id/web/src/image-browser.ts` (new) — Image browser UI component
- `pkgs/id/web/src/editor.ts` — Add "Browse images" toolbar button that opens the browser

**Changes**:

### routes.rs

Add a new endpoint `GET /:name/api/images` that:
1. Lists blobs from the iroh blob store that have image MIME types (or image file extensions in their filename metadata).
2. Returns JSON array: `[{ hash, filename, url, size }]`
3. The blob store already tracks uploads — this queries existing data, no new storage needed.

### image-browser.ts (new file)

1. **Dialog UI**: Modal overlay with a grid of image thumbnails. Each thumbnail shows the image (loaded from `/blob/{hash}?filename={name}`) and the filename below.
2. **Fetch**: On open, `GET /api/images` to get the list.
3. **Selection**: Click an image to select it. "Insert" button inserts the selected image at the current cursor position:
   ```typescript
   const node = schema.nodes.image.create({ src: selectedImage.url, alt: selectedImage.filename });
   const tr = view.state.tr.replaceSelectionWith(node);
   view.dispatch(tr);
   ```
4. **Close**: Click outside, press Escape, or click X to close.
5. **Empty state**: "No images uploaded yet. Paste or drag an image to upload."

### editor.ts

1. Add "Browse images" button to toolbar row 1 (after the existing "Insert image" upload button). Icon: grid/gallery icon.
2. Button opens the image browser dialog.

**Test spec**:
- E2E: Upload 2 images via paste. Click "Browse images" button. Verify both images appear in the browser grid. Click one, click "Insert", verify image inserted at cursor. Close browser, verify it disappears.

---

## Implementation Order

```
Part 1 (Strikethrough) → Part 2 (Task Lists) → Part 3 (Tables) → Part 4 (Alt-Text) → Part 5 (Resize) → Part 6 (Image Browser)
```

**Rationale**:
- Parts 1-3 are GFM extensions ordered by complexity (strikethrough is simplest, tables most complex). Each builds familiarity with the markdown.rs ↔ editor.ts sync pattern.
- Part 1 must come first because enabling comrak GFM extensions affects options shared by all parts.
- Parts 4-5 are image improvements. Part 4 creates the image nodeView that Part 5 extends with resize handles — they must be sequential.
- Part 6 is independent and can be done last.

Parts 1 and 2 could potentially be parallelized since they touch different node/mark types, but the shared `commonmark_options()` function makes sequential safer.

## Validation Criteria

1. **Round-trip fidelity**: A `.md` file with GFM tables, task lists, and strikethrough survives open → edit → save without data loss.
2. **Schema sync**: Every PM node/mark type defined in `editor.ts` has corresponding conversion logic in `markdown.rs`, and vice versa.
3. **Collaborative editing**: New node types work correctly with the collab protocol — concurrent edits to table cells, checkbox toggles, image resizes all merge correctly (ProseMirror OT handles this if schema is correct).
4. **Toolbar completeness**: All new features accessible from the toolbar (strikethrough button, table insert, image browse). Keyboard shortcuts for strikethrough.
5. **Image persistence**: Alt-text edits and resize dimensions survive save/reload cycle for both `.pm.json` and `.md` files.
6. **No regressions**: Existing CommonMark features (headings, lists, blockquotes, code blocks, links, images, bold, italic) continue to work identically.
7. **E2E test coverage**: Each part has at least one end-to-end test covering the primary user flow.

---

## Status

| Part | Description | Status | Commit |
|------|------------|--------|--------|
| 1 | GFM Strikethrough | ✅ Done | (this commit) |
| 2 | Task Lists | ⏳ Pending | — |
| 3 | Tables | ⏳ Pending | — |
| 4 | Image Alt-Text | ⏳ Pending | — |
| 5 | Image Resize | ⏳ Pending | — |
| 6 | Image Browser | ⏳ Pending | — |
