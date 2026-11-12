## 2025-02-24 - [Avoid O(N) Search Highlight Traversals]

**Learning:** Counting match highlights in ProseMirror search by repeatedly calling `findNext` performs an O(N) document traversal per match, which becomes a severe bottleneck (e.g. 1.7s for 10k matches) when document sizes scale or searches are frequent. `prosemirror-search` already computes a `DecorationSet` internally containing all matches.
**Action:** Always use `getMatchHighlights(view.state).find()` to retrieve search matches in O(1) time instead of iterating manually over the document with `findNext` or `findNextNoWrap`.

## 2026-10-25 - [Cache ProseMirror DOM Coordinate Measurements]

**Learning:** `EditorView.coordsAtPos()` queries DOM layout/geometry and triggers browser reflows. Calling it repeatedly per remote cursor in `createCursorDecorations` (for same-line check, line Y-position grouping, and horizontal overlap clustering) results in redundant layout reads during collaborative editor updates.
**Action:** Cache `view.coordsAtPos` calls per synchronous decoration calculation pass in a `Map<number, Coords | null>` to reduce DOM layout reads by ~75% while keeping calculations strictly scoped to the current render pass.

## 2026-10-27 - [Leverage Sorted Line Starts for O(log N) Range Slicing]

**Learning:** Line start position arrays in editors are strictly monotonically increasing. Using `lineStarts.includes()` or `Array.prototype.filter()` iterates over the entire document's lines (O(N) time and O(N) allocations), and nested calls like `filter` with `indexOf` create O(N^2) bottlenecks.
**Action:** Use binary search to locate line boundaries in O(log N) time and `Array.prototype.slice()` to extract selected line ranges in O(M) time without full array scans.

## 2026-10-31 - [Lazy Subject Kind Resolution in Event Store Searches]

**Learning:** `search_store_terms` eager-loaded `store.event_by_id` for every unique subject in live claims up front even when no `Term::SubjectKind` query term was present. In addition to O(subjects) redundant database reads per search, unrelated unreadable or unindexed events aborted pure name/author searches with errors.
**Action:** Always check whether query terms actually require subject kind resolution before opening store transactions, and lazily resolve and memoize kinds on demand within kind-filtering closures.

## 2026-11-04 - [Per-cell Intl.Segmenter validation dominates the world screen renderer]

**Learning:** `renderWorldSurface` in `pkgs/id/web/src/world-surface.ts` validated every cell of a 120x50 world frame with `Intl.Segmenter.segment()` plus the unicode-width regexes (`\p{Mn}`, `\p{Grapheme_Extend}`) — ~120k segmentations per second at 20 frames/s. On the ASCII path this is pure overhead: a single code unit in U+0020..U+007E is always exactly one grapheme and one column wide. Measured in Bun/JSC, per-frame validation + colour CPU for a full 6000-cell frame fell from 60–169 ms to 2–3 ms (30–55x). Full-render timings in happy-dom are useless for judging this: element creation dominates (hundreds of ms with 4x run-to-run noise) and is unchanged by the fix.
**Action:** In hot render loops, fingerprint the common value class (ASCII / single code unit) and take a constant-time fast path before calling Intl or unicode-regex machinery; keep the general path for everything else. Benchmark the CPU component with `document.createElement` stubbed, not happy-dom's DOM.
