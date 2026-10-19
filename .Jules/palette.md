## 2026-10-19 - Missing ARIA Labels on Template Buttons
**Learning:** The server-rendered HTML templates (`pkgs/id/src/web/templates.rs`) often miss `aria-label` attributes on empty or icon-only interactive elements like theme selectors or short "+" buttons, despite having visual `title` attributes. This is a common accessibility anti-pattern in this app's UI.
**Action:** Always check server-rendered HTML blocks for missing `aria-label`s on `button` tags when they are visually minimal or empty.
