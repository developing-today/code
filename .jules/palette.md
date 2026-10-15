## 2024-05-17 - Add aria-label to theme toggle buttons

**Learning:** Found that empty icon buttons (like theme selectors) generated via Rust server-side templates (`pkgs/id/src/web/templates.rs`) were missing `aria-label` attributes, though they had `title` attributes. Missing these on icon-only interactive elements is a common accessibility anti-pattern.
**Action:** When working on server-rendered templates, especially for interactive elements like theme buttons or inline tools, always verify that `aria-label` attributes are included alongside `title` to ensure they are fully accessible to screen readers.
