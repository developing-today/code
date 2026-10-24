## 2025-02-24 - [Avoid O(N) Search Highlight Traversals]

**Learning:** Counting match highlights in ProseMirror search by repeatedly calling `findNext` performs an O(N) document traversal per match, which becomes a severe bottleneck (e.g. 1.7s for 10k matches) when document sizes scale or searches are frequent. `prosemirror-search` already computes a `DecorationSet` internally containing all matches.
**Action:** Always use `getMatchHighlights(view.state).find()` to retrieve search matches in O(1) time instead of iterating manually over the document with `findNext` or `findNextNoWrap`.

## 2025-02-24 - [ProseMirror Decoration Key Generation]

**Learning:** Avoid appending global generation counters or randomized values to ProseMirror `Decoration.widget` keys, as this circumvents built-in DOM node recycling and forces continuous recreation of widgets, leading to layout thrashing.
**Action:** Use stable keys for `Decoration.widget` to enable DOM node recycling, ensuring any properties that dynamically dictate the widget's appearance (such as opacity, connectionState, or color) are explicitly included in the key string. When implementing ProseMirror decorations, prefer the factory function pattern `() => HTMLElement` for `Decoration.widget` construction.
