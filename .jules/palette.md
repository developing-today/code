## 2025-10-29 - Explicit ARIA Labels for Interactive Controls and Form Inputs

**Learning:** In server-rendered HTML templates (`templates.rs`) and dynamic TS panels (`search-panel.ts`), standalone inputs and buttons (such as search replace buttons, toggle checkboxes, and session inputs) frequently lack explicit `aria-label` attributes even when visually grouped.
**Action:** Always verify that every input element and icon/action button has an explicit `aria-label` or `aria-labelledby` attribute to ensure complete screen reader accessibility across both server templates and client DOM components.

## 2026-10-30 - Server-Rendered Template Action Buttons Missing ARIA Labels

**Learning:** Server-rendered HTML templates generating compact/action buttons (like `save`, `rename`, `copy`, or `clear`) often include visual text labels or `title` tooltips but lack explicit `aria-label` attributes. Screen reader users navigating by interactive controls or relying on accessible names need `aria-label` attributes to ensure consistent screen reader announcement across browser environments.
**Action:** When inspecting or adding buttons in Rust HTML templates (`pkgs/id/src/web/templates.rs`), ensure every icon-only or shortened textual action button explicitly defines an `aria-label` describing the action.
