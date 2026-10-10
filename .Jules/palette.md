## 2026-10-09 - Adding accessibility tags to icons and inputs
**Learning:** Found that custom search/find-and-replace components often use text or icons without proper `aria-label`s or matching `for` tags, making them unfriendly to screen readers.
**Action:** When working on custom utility panels (like find-and-replace) and custom inputs, make sure they have explicitly defined `aria-label` properties (e.g. 'Match case' or 'Close search panel') and properly targeted `for` labels.
