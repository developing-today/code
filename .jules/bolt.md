## 2025-02-18 - ProseMirror Search Match Counting

**Learning:** In ProseMirror, repeatedly calling `findNext` to count search matches traverses the entire document resulting in an O(N) operation per search input change. `prosemirror-search` (v1.1.1+) maintains a pre-computed `DecorationSet` in its state which can be accessed instantly using `getMatchHighlights(view.state)`.
**Action:** Always prefer retrieving pre-computed state from ProseMirror plugins using their getter functions (like `getMatchHighlights`) instead of recalculating state by manually traversing the document or re-running search algorithms.
