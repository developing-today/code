## 2024-10-24 - Accessibility improvements for Theme and Action Buttons
**Learning:** Empty `<button>` elements (such as those for selecting themes, adding tags, or downloading) lacked descriptive text, making them completely opaque to screen readers.
**Action:** Implemented `aria-label` attributes across all instances of these buttons in the `pkgs/id/src/web/templates.rs` server-rendered HTML templates. Whenever adding an icon-only or CSS-styled button without internal text node content, remember to attach a descriptive `aria-label`.
