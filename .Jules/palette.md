## 2026-10-10 - [Add ARIA labels to search panel]

**Learning:** The prosemirror search panel uses custom HTML injected directly into the DOM containing icon-only buttons for previous/next and close. These inputs lack programmatic names, and the search result match count doesn't announce to screen readers.
**Action:** When adding or auditing injected DOM elements for interactive features, ensure all inputs and icon buttons have `aria-label` attributes and dynamically changing text like match counts have `aria-live="polite"`.

## 2026-11-04 - [Go to Line rejection was silent]

**Learning:** In `pkgs/id/web`, editor dialogs are module singletons that are hidden rather than destroyed and reattached across editor recreation. Transient accessibility state (`aria-invalid`, error text) therefore survives into the next open unless it is reset in both open and close — and a rejected `Ctrl+G` value previously only re-selected the text, which reads as a broken shortcut.
**Action:** For these singleton dialogs, put the reason next to the input with `role="alert"` + `aria-describedby`, and reset it on open, on input and on close (`goto-line.ts`).
