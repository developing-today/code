## 2024-05-24 - Avoid global counters for ProseMirror widget keys

**Learning:** Appending global generation counters to ProseMirror `Decoration.widget` keys (e.g., `g${gen}`) circumvents built-in DOM node recycling and forces continuous recreation of widgets, leading to layout thrashing.

**Action:** When implementing ProseMirror decorations, always use the factory function pattern `() => HTMLElement` for `Decoration.widget` construction and derive the `key` string from underlying data properties (e.g., ID, position, and properties that dictate appearance) to enable DOM node recycling.
