## 2024-05-18 - Add aria-labels to raw inputs and buttons
**Learning:** Found a pattern of missing `aria-label` attributes on raw `<input>` and icon-only `<button>` elements in the Rust-based server-rendered HTML templates (`pkgs/id/src/web/templates.rs`).
**Action:** Always ensure that icon-only buttons and raw inputs generated from server templates have appropriate `aria-label` attributes for accessibility.
