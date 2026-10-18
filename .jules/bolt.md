## 2024-05-24 - [ProseMirror Search Highlight Count]

**Learning:** prosemirror-search performs O(N) traversal if queried manually with findNext in a loop. But getMatchHighlights gives access to the pre-computed decorations.
**Action:** Use getMatchHighlights(state).find().length to count matches in O(1) instead of traversing the document manually.
