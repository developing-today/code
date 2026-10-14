
## 2026-10-14 - Missing ARIA labels on empty icon buttons
**Learning:** Missing ARIA labels on empty icon buttons (like theme selectors and inline tools) is a common accessibility anti-pattern in this app's components, making it difficult for screen readers to interpret the buttons' purpose.
**Action:** Always verify that buttons lacking textual content have an explicit `aria-label` alongside the `title` attribute when reviewing or adding new interactive components.
