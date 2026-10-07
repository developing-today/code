## 2024-05-24 - Search Panel Accessibility
**Learning:** Found a specific accessibility issue pattern in the custom ProseMirror search panel components where icon-only buttons (like prev/next/close) and search inputs lacked aria-labels, relying solely on placeholder text and visual titles which are insufficient for screen readers.
**Action:** Always ensure custom Prosemirror floating panels use explicit `aria-label` attributes for inputs and icon-only buttons, as they don't inherit default accessible structures from the main editor.
