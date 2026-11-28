## 2025-10-29 - Explicit ARIA Labels for Interactive Controls and Form Inputs

**Learning:** In server-rendered HTML templates (`templates.rs`) and dynamic TS panels (`search-panel.ts`), standalone inputs and buttons (such as search replace buttons, toggle checkboxes, and session inputs) frequently lack explicit `aria-label` attributes even when visually grouped.
**Action:** Always verify that every input element and icon/action button has an explicit `aria-label` or `aria-labelledby` attribute to ensure complete screen reader accessibility across both server templates and client DOM components.

## 2026-10-30 - Server-Rendered Template Action Buttons Missing ARIA Labels

**Learning:** Server-rendered HTML templates generating compact/action buttons (like `save`, `rename`, `copy`, or `clear`) often include visual text labels or `title` tooltips but lack explicit `aria-label` attributes. Screen reader users navigating by interactive controls or relying on accessible names need `aria-label` attributes to ensure consistent screen reader announcement across browser environments.
**Action:** When inspecting or adding buttons in Rust HTML templates (`pkgs/id/src/web/templates.rs`), ensure every icon-only or shortened textual action button explicitly defines an `aria-label` describing the action.

## 2026-10-10 - [Add ARIA labels to search panel]

**Learning:** The prosemirror search panel uses custom HTML injected directly into the DOM containing icon-only buttons for previous/next and close. These inputs lack programmatic names, and the search result match count doesn't announce to screen readers.
**Action:** When adding or auditing injected DOM elements for interactive features, ensure all inputs and icon buttons have `aria-label` attributes and dynamically changing text like match counts have `aria-live="polite"`.

## 2026-11-04 - [Go to Line rejection was silent]

**Learning:** In `pkgs/id/web`, editor dialogs are module singletons that are hidden rather than destroyed and reattached across editor recreation. Transient accessibility state (`aria-invalid`, error text) therefore survives into the next open unless it is reset in both open and close — and a rejected `Ctrl+G` value previously only re-selected the text, which reads as a broken shortcut.
**Action:** For these singleton dialogs, put the reason next to the input with `role="alert"` + `aria-describedby`, and reset it on open, on input and on close (`goto-line.ts`).

## 2026-11-24 - Static dashboards ship unlabeled inputs and silent async output

**Learning:** This repo's real UI surfaces are small static dashboards under
`pkgs/*/public/` (not a component framework). Their recurring pattern is
fetch-and-dump into a `<pre>`: inputs ship without `<label>` (placeholder-only
at best) and the async result regions are invisible to screen readers because
nothing announces the text change.
**Action:** On any `pkgs/*/public/` page, check every `<input>` for a
`<label for>`, and put `aria-live="polite"` on elements whose textContent is
set from fetch/WebSocket handlers. First instance fixed: lab-ops-hub.
