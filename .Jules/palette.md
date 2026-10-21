## 2025-02-14 - Add aria-labels to empty icon buttons in Rust templates

**Learning:** Empty icon-only buttons (like theme selectors) in `pkgs/id/src/web/templates.rs` often rely only on the `title` attribute. Missing `aria-label`s on empty buttons is a major accessibility anti-pattern in the server-rendered UI.
**Action:** When adding or modifying interactive UI elements in the server-rendered HTML templates (`templates.rs`), ensure they all have proper `aria-label`s, especially if they don't have text content.
