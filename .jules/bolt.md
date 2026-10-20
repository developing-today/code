## 2024-05-24 - Initial Bolt Journal

## 2024-05-24 - ProseMirror DOM Recycling Optimization
**Learning:** The codebase intentionally bypassed ProseMirror's built-in DOM node recycling by using a global `decorationGeneration` counter in the `key` for `Decoration.widget`, which caused unnecessary widget recreation on every state update and resulted in layout thrashing.
**Action:** When creating ProseMirror widgets, always use the factory function pattern `() => HTMLElement` for `Decoration.widget` and ensure `key` strings are derived from stable and relevant visual data properties (like ID, opacity, color, and connectionState) instead of using random/incremental generation values.
