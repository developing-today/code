## 2024-05-19 - Efficient Match Counting in ProseMirror
**Learning:** Using `prosemirror-search`'s `findNext` iteratively to count matches causes full O(N) string traversal over the entire document string, which becomes a bottleneck on large documents with many matches.
**Action:** When calculating total match counts in ProseMirror, use `getMatchHighlights(view.state)` (available in `prosemirror-search` >= 1.1.1) to retrieve the internally cached `DecorationSet` and check its length. This turns an O(N) string search into an O(1) retrieval.
