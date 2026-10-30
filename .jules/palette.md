## 2026-10-30 - Server-Rendered Template Action Buttons Missing ARIA Labels

**Learning:** Server-rendered HTML templates generating compact/action buttons (like `save`, `rename`, `copy`, or `clear`) often include visual text labels or `title` tooltips but lack explicit `aria-label` attributes. Screen reader users navigating by interactive controls or relying on accessible names need `aria-label` attributes to ensure consistent screen reader announcement across browser environments.
**Action:** When inspecting or adding buttons in Rust HTML templates (`pkgs/id/src/web/templates.rs`), ensure every icon-only or shortened textual action button explicitly defines an `aria-label` describing the action.
