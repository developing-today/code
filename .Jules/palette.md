
## 2026-10-18 - Accessible Icon Buttons in Server Templates
**Learning:** In the `id` app, structural UI elements like empty icon buttons (e.g., theme selectors, inline tag buttons) are rendered in Rust backend templates (`pkgs/id/src/web/templates.rs`). These buttons frequently rely on CSS classes (like `theme-btn`) for visual styling but omit screen-reader accessible text.
**Action:** Always verify that empty icon buttons in `templates.rs` include an `aria-label` attribute alongside any `title` attribute to ensure proper accessibility for screen readers.
