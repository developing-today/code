# MOB-04 — Accessible compact responsive interaction system

Issue: #9471 (slice MOB-04 of #9201). Status: design record. It is not
an authorization to implement. Target: WCAG 2.2 AA review target, not a
blanket compliance claim.

## 0. Governing decisions consumed

| Source                   | What this record takes from it                                                                                                                                                                                                                                                                                       |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9201 §3.4               | 360 px and 200% zoom with no horizontal scroll; 24×24 px minimum targets or a spacing exception; 44×44 px preferred for primary actions; visible focus; status announcements; reduced motion; no gesture-only or hover-only path; no accidental double submit; consequential actions confirm with target and effect. |
| #9201 MOB-04 slice text  | Owned files; red test `mobile_primary_controls_are_keyboard_and_target_sized`; oracle: DOM assertions plus measured geometry at 360 / 768 / 1280 px and 200% zoom, keyboard flow, manual checklist.                                                                                                                  |
| MOB-00 (#9467, on main)  | The pinned Playwright harness (`@playwright/test` 1.64.0), fixture mode, network isolation.                                                                                                                                                                                                                          |
| MOB-03 record D7         | The `/work` list and detail pages and their keyboard order. MOB-03 uses these components and adds none.                                                                                                                                                                                                              |
| MOB-07 record D3, D4, D5 | The panel's confirm dialog, operation-key forms and receipt stages, rendered with these components.                                                                                                                                                                                                                  |
| #1180, #1215 (on main)   | The 40 px floor on existing chips and nav links; the light palette under `prefers-color-scheme: light`.                                                                                                                                                                                                              |

## 1. Verified state on main `31fbdff2`

- `src/ui.css` (454 lines) hard-codes every colour; there are no CSS
  custom properties. A light palette (#1215) overrides colours in one
  `prefers-color-scheme: light` block.
- Nav links, chips (`.chips a`, `.qchip`, `.xchip`, `.hchip`) and
  quota labels have `min-height: 40px`; action forms (`.aform`,
  `.actbox`, `.bulkbar`) use 44 px. `test/mobile-targets.test.ts` pins
  the 40 px rules by reading the stylesheet.
- One `:focus-visible` rule exists (the epic graph's SVG control). No
  global focus style is set, so most controls rely on the browser
  default.
- `render.ts` has one `aria-live` region (quota refresh), 78
  `<button` elements and 41 `aria-label` uses. No `<dialog>` is used.
- No `prefers-reduced-motion` rule exists.
- No automated accessibility engine is a dependency. The browser
  harness has one test, covering `/queue`, `/item/<id>` and
  `/attention` at 360 and 768 px.

## 2. Intent

One small set of components, used by every mobile work page, that make
the accessible choice the default: tokens with checked contrast, a
global focus ring, sized targets, one announcement region, a native
confirm dialog, and a submit that cannot fire twice. The checks are
measured in a browser, not inferred from screenshots.

## 3. Ownership

- **Owned:** `src/ui.css` (new token block, focus, motion and
  component rules), `src/render.ts` (the component helpers of D3),
  new `test/mobile-accessibility.test.ts`, new
  `docs/mobile-accessibility.md` (the manual checklist and exception
  register).
- **Needed edit outside the owned list, named:** new steps in
  `test/browser/mobile-work-view.spec.ts` (MOB-00's file; additions
  only).
- **Sequencing:** MOB-03 owns the same two source files first. MOB-04
  lands after MOB-03's code, or MOB-03's code uses these helpers if
  MOB-04 lands first; either way one implementation of each helper.
- **Not changed:** existing pages' markup beyond adopting the global
  focus ring and tokens, which change no layout.

## 4. Decisions

### D1. Colour tokens

A `:root` block defines custom properties for every colour role:
`--bg`, `--surface`, `--border`, `--text`, `--text-muted`, `--link`,
`--accent`, `--ok`, `--warn`, `--danger`, `--focus`. The light block
redefines the same names. Existing hard-coded colours are moved onto
the tokens in the same change, value for value, so no page changes
appearance.

Contrast pairs are listed once in `docs/mobile-accessibility.md` and
checked by the bun test from the token values (relative luminance per
WCAG 2.2):

| Pair                                   | Minimum |
| -------------------------------------- | ------- |
| `--text` on `--bg`, `--surface`        | 4.5:1   |
| `--text-muted` on `--bg`, `--surface`  | 4.5:1   |
| `--link` on `--bg`, `--surface`        | 4.5:1   |
| `--focus` against `--bg`, `--surface`  | 3:1     |
| `--border` of a control against `--bg` | 3:1     |
| `--ok`, `--warn`, `--danger` as text   | 4.5:1   |

Both palettes are checked. A pair that fails is a test failure, not an
exception.

### D2. Focus, motion, targets

- Global `:focus-visible { outline: 2px solid var(--focus);
  outline-offset: 2px; }`. No rule may set `outline: none` without a
  replacement; the bun test scans the stylesheet for it.
- `@media (prefers-reduced-motion: reduce)` sets transitions and
  animations to none and scroll behaviour to auto.
- Every interactive element has a 24×24 CSS px hit area, or sits in an
  inline sentence (the WCAG 2.5.8 inline exception), recorded per
  class in the exception register.
- Primary actions (`.btn-primary`, the confirm button, the filter
  submit, the panel's operation buttons) are at least 44×44 px.
- The existing 40 px chip floor stays and is above the 24 px minimum.

### D3. Components (render helpers in `render.ts`)

| Helper                  | Renders                                                                                                     | Rules                                                                                                                                            |
| ----------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `uiCard(opts)`          | `<article>` with a heading link as the first focusable element                                              | Whole-card click handlers are not used; the link is the only target, so there is one tab stop per card.                                          |
| `uiFilter(opts)`        | `<form method="get">` with a `<label>`led `<select>` and a submit button                                    | Works with no script (GET); the selected view is in the URL.                                                                                     |
| `uiButton(opts)`        | `<button type="submit">` with visible text                                                                  | `primary` adds the 44 px class; an icon-only button must pass `label`, rendered as `aria-label`.                                                 |
| `uiOperationForm(opts)` | a POST form carrying the operation key as a hidden field and `data-once`                                    | On submit, the button is disabled and `aria-busy="true"` set; the server's operation key is the real guard (MOB-07 D4).                          |
| `uiConfirmDialog(opts)` | a native `<dialog>` opened by its trigger, with a heading naming target and effect, and Cancel then Confirm | Focus moves to the heading on open and back to the trigger on close; Escape cancels; without script the trigger posts to a confirm page instead. |
| `uiStatusRegion()`      | one `<div role="status" aria-live="polite">` per page                                                       | Action results are written here; errors use a separate `role="alert"` region.                                                                    |
| `uiResult(receipt)`     | the receipt stages as a list with text states                                                               | State is never conveyed by colour alone; each stage has words ("Delivered", "Not yet").                                                          |
| `uiSkipLink()`          | "Skip to main content" as the first focusable element                                                       | Targets `<main id="main" tabindex="-1">`.                                                                                                        |

No component needs a gesture, a hover, or a long press. Every action
reachable by pointer is reachable by keyboard in the same order as it
reads.

### D4. Reflow

- At 360 CSS px and at 640 CSS px (1280 px at 200% zoom, the WCAG
  1.4.10 equivalence), `scrollWidth - clientWidth ≤ 1` on every mobile
  work page.
- Long identifiers wrap (`overflow-wrap: anywhere` on cards and code
  spans). Tables in these pages become stacked definition lists below
  480 px; no table is wider than the viewport.

### D5. Automated engine

The V1 floor is the hand-written assertions in §6. Adding an automated
engine (`@axe-core/playwright`) is a new dependency in a lockfile, so
it is a separate, reviewed change. When added, the browser test fails
on any `critical` or `serious` violation not listed in the exception
register.

### D6. Exception register

`docs/mobile-accessibility.md` lists each exception with the class or
page, the WCAG criterion, why, and who accepted it. The bun test reads
the register and fails if an exception names a class that no longer
exists, so stale exceptions are removed with the code they excused.

## 5. Negative cases

| Case                  | Expected                                                                             |
| --------------------- | ------------------------------------------------------------------------------------ |
| stale input           | a stale-facet label is text plus icon, never colour alone                            |
| duplicate / replay    | double Enter on an operation form posts once; the server key dedupes any second post |
| ambiguous resource    | the confirm dialog names the exact target (title and locator), never "this item"     |
| unauthorized actor    | a disabled control keeps its accessible name and states the reason in text           |
| partial remote effect | `uiResult` shows each stage's own words; partial is announced as partial             |
| unknown version       | not applicable to presentation                                                       |
| no false completed    | the status region never announces "Done" for a send; it reads the receipt's words    |

## 6. Oracles and tests

`test/mobile-accessibility.test.ts` (bun, no browser):

1. `mobile_primary_controls_are_keyboard_and_target_sized` (red test).
   Every `uiButton({primary:true})`, `uiConfirmDialog` confirm and
   `uiOperationForm` button renders the 44 px class; every helper's
   interactive element is a native focusable element with a non-empty
   accessible name.
2. `mobile_tokens_meet_contrast_pairs`. D1's table, both palettes.
3. `mobile_css_never_removes_focus_without_replacement`.
4. `mobile_css_has_reduced_motion_rule`.
5. `mobile_exception_register_names_live_classes`.
6. `mobile_confirm_dialog_names_target_and_effect`.

Browser, added to `test/browser/mobile-work-view.spec.ts`, test name
`pmo_mob_04_interaction_geometry_and_keyboard`:

- At 360, 768, 1280 and 640 CSS px: every visible `button`, `a`,
  `input`, `select` on `/work` and its detail page has a bounding box at
  least 24×24, or a class in the register; primary controls at least
  44×44; no horizontal overflow.
- Keyboard: the skip link is first; Tab order matches DOM order; the
  confirm dialog traps focus while open, Escape closes it, and focus
  returns to its trigger.
- A submitted operation writes its result into the `role="status"`
  region (read back with `textContent`), and a second Enter during the
  request produces no second POST (counted with `page.on("request")`).
- The focused element's outline is visible: its computed
  `outline-style` is not `none`.

The manual checklist in `docs/mobile-accessibility.md` covers a screen
reader pass (VoiceOver on iOS, TalkBack on Android) and a 200% browser
zoom pass, each dated and signed off per release; it is not run in CI.

## 7. Compatibility and reversal

Moving colours to tokens is value-for-value, so pages look the same.
The global focus ring is additive. Removing the helpers and token block
restores the old stylesheet; `/work` would then lose its components and
must be removed with them.

## 8. Requirement trace

| ID     | How this record serves it                                    |
| ------ | ------------------------------------------------------------ |
| C-R083 | One component set for every mobile work page (D3).           |
| H-R001 | Measured reflow and targets at the four widths (D2, D4, §6). |
| H-R013 | Keyboard and status announcement checks (D3, §6).            |
| O-R013 | Exceptions are recorded and checked, never silent (D6).      |

## 9. Gates

1. This record merged.
2. MOB-03's code first, or MOB-03 adopting these helpers (shared files).
3. The automated engine (D5) is its own change with a dependency review.
4. No production rollout claim until the manual checklist has a dated
   pass.

## 10. Non-goals

- Restyling pages outside the mobile work flows.
- A component library or framework; these are string helpers in
  `render.ts`.
- Claiming WCAG conformance for the whole app.
