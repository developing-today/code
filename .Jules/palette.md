## 2024-05-18 - Missing ARIA labels on UI inputs/buttons

**Learning:** Found an accessibility pattern where inputs relying only on `placeholder` attributes or icon-only buttons (like `+` to add tags) lack actual accessible names, causing screen readers to struggle identifying the purpose of the elements. `aria-label`s were needed.
**Action:** Always verify that input elements without explicit `<label>` elements and buttons that only display icons or symbols have descriptive `aria-label` attributes.
