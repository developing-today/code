## 2026-10-10 - [Add ARIA labels to search panel]
**Learning:** The prosemirror search panel uses custom HTML injected directly into the DOM containing icon-only buttons for previous/next and close. These inputs lack programmatic names, and the search result match count doesn't announce to screen readers.
**Action:** When adding or auditing injected DOM elements for interactive features, ensure all inputs and icon buttons have `aria-label` attributes and dynamically changing text like match counts have `aria-live="polite"`.
