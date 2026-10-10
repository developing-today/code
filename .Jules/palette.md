## 2026-10-08 - Adding aria-live to search match counts and aria-labels to editor plugins
**Learning:** Screen readers need context for dynamic updates in editor UI plugins (like prosemirror-search match counts). Text inputs and icon-only buttons often lack accessible names in minimalist editor UI.
**Action:** When building or enhancing editor plugin UIs, always include `aria-live="polite"` on elements that dynamically update (like match counts) and standard `aria-label` on text inputs/icon buttons.
