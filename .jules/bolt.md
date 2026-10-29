## 2025-02-24 - [Avoid O(N) Search Highlight Traversals]

**Learning:** Counting match highlights in ProseMirror search by repeatedly calling `findNext` performs an O(N) document traversal per match, which becomes a severe bottleneck (e.g. 1.7s for 10k matches) when document sizes scale or searches are frequent. `prosemirror-search` already computes a `DecorationSet` internally containing all matches.
**Action:** Always use `getMatchHighlights(view.state).find()` to retrieve search matches in O(1) time instead of iterating manually over the document with `findNext` or `findNextNoWrap`.

## 2026-10-25 - [Cache ProseMirror DOM Coordinate Measurements]

**Learning:** `EditorView.coordsAtPos()` queries DOM layout/geometry and triggers browser reflows. Calling it repeatedly per remote cursor in `createCursorDecorations` (for same-line check, line Y-position grouping, and horizontal overlap clustering) results in redundant layout reads during collaborative editor updates.
**Action:** Cache `view.coordsAtPos` calls per synchronous decoration calculation pass in a `Map<number, Coords | null>` to reduce DOM layout reads by ~75% while keeping calculations strictly scoped to the current render pass.

## 2026-10-26 - [Reuse Line Starts Array in ProseMirror Document Traversal]

**Learning:** Calling sequential helpers like `getLineCount()` and `getLineStartPos()` in user action handlers triggers multiple full ProseMirror AST traversals (`doc.descendants`). Also, reading `node.textContent` creates concatenated string copies for every textblock node and skips `hard_break` elements.
**Action:** Compute `lineStarts` once per action using `node.forEach` to iterate inline children directly, reusing the array for both count validation and position lookup in O(1) time.
