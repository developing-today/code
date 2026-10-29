## 2025-10-29 - Explicit ARIA Labels for Interactive Controls and Form Inputs

**Learning:** In server-rendered HTML templates (`templates.rs`) and dynamic TS panels (`search-panel.ts`), standalone inputs and buttons (such as search replace buttons, toggle checkboxes, and session inputs) frequently lack explicit `aria-label` attributes even when visually grouped.
**Action:** Always verify that every input element and icon/action button has an explicit `aria-label` or `aria-labelledby` attribute to ensure complete screen reader accessibility across both server templates and client DOM components.
