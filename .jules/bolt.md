## 2025-02-24 - [Avoid O(N) Search Highlight Traversals]

**Learning:** Counting match highlights in ProseMirror search by repeatedly calling `findNext` performs an O(N) document traversal per match, which becomes a severe bottleneck (e.g. 1.7s for 10k matches) when document sizes scale or searches are frequent. `prosemirror-search` already computes a `DecorationSet` internally containing all matches.
**Action:** Always use `getMatchHighlights(view.state).find()` to retrieve search matches in O(1) time instead of iterating manually over the document with `findNext` or `findNextNoWrap`.

## 2026-10-25 - [Cache ProseMirror DOM Coordinate Measurements]

**Learning:** `EditorView.coordsAtPos()` queries DOM layout/geometry and triggers browser reflows. Calling it repeatedly per remote cursor in `createCursorDecorations` (for same-line check, line Y-position grouping, and horizontal overlap clustering) results in redundant layout reads during collaborative editor updates.
**Action:** Cache `view.coordsAtPos` calls per synchronous decoration calculation pass in a `Map<number, Coords | null>` to reduce DOM layout reads by ~75% while keeping calculations strictly scoped to the current render pass.

## 2026-10-27 - [Leverage Sorted Line Starts for O(log N) Range Slicing]

**Learning:** Line start position arrays in editors are strictly monotonically increasing. Using `lineStarts.includes()` or `Array.prototype.filter()` iterates over the entire document's lines (O(N) time and O(N) allocations), and nested calls like `filter` with `indexOf` create O(N^2) bottlenecks.
**Action:** Use binary search to locate line boundaries in O(log N) time and `Array.prototype.slice()` to extract selected line ranges in O(M) time without full array scans.
