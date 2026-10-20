## 2026-10-20 - Missing aria-labels on icon buttons in templates
**Learning:** Empty icon buttons (like theme selectors) generated within the Rust templates (`pkgs/id/src/web/templates.rs`) consistently lack `aria-label` attributes, relying solely on `title` attributes. This is a common accessibility anti-pattern in the app's UI.
**Action:** When adding or modifying interactive icon-only elements in the Rust-rendered server templates, ensure an `aria-label` is explicitly set alongside any `title` attribute.
