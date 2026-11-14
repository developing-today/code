# Decision: web rendering — assertion → Tailwind mapping

- **Date:** 2026-11-09
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5851](https://github.com/developing-today/code/issues/5851),
  "id: design: web rendering: assertion → Tailwind mapping (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites — semantic markup,
  style-override mapping"), the web-target bullet of its style-override
  mapping section. Programme:
  [#2056](https://github.com/developing-today/code/issues/2056).
- **Vocabularies consumed:**
  [#5849](https://github.com/developing-today/code/issues/5849) — semantic
  markup vocabulary v1, landed as
  `doc/2026-11-08_decision_id5849_semantic_markup_vocabulary_v1.md`
  (PR #7923). Element kinds here are that registry's names; a later
  vocabulary version that renames kinds re-issues §2's row labels only.
  [#5850](https://github.com/developing-today/code/issues/5850) — the
  style-assertion vocabulary, settled in that issue's decision content
  (task 12817, §§1–12). Its §12 names this ticket as the consumer of
  its §§3–9 for the web mapping table. This record is that table, plus
  the cascade, raw-CSS, and raw-HTML decisions the parent left to it.
- **Sibling precedents:**
  [#5852](https://github.com/developing-today/code/issues/5852)
  (`doc/2026-11-08_decision_id5852_native_rendering_os_standard_controls.md`,
  PR #7912) and
  [#5853](https://github.com/developing-today/code/issues/5853)
  (`doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`,
  PR #7684) — each mapping table states its assumptions against
  unsettled inputs explicitly rather than parking; this record follows
  that precedent (§1).
  [#5854](https://github.com/developing-today/code/issues/5854)
  (`doc/2026-11-08_decision_id5854_per_target_composition.md`, PR #7864)
  — the five-layer cascade this record adopts verbatim (§4).
- **Pipeline boundary:**
  [#5855](https://github.com/developing-today/code/issues/5855), whose
  decision record (`doc/2026-11-08_decision_id5855_rendering_pipeline_conformance.md`)
  is open in review (PR #7921) at the time of writing, owns the
  five-stage pipeline (parse→validate→compose→lower→emit), the
  compile-time-dominant split, and the conformance corpus shape. Mapping
  tables are _data_ to its lowering stage. This record defines that
  data for the web target; it does not redefine the pipeline, the
  report format, or the corpus gates (§1, assumption A4).
- **Evidence baseline:** `origin/main` at `1cd22df899`, worktree
  `/home/user/git/fleet/12822`, 2026-11-09. Verified at that baseline,
  consistent with triage tasks 10254 (2026-11-05) and 12343
  (2026-11-08): issue #5851 is open with no hard-stop labels, no PR
  references it, and no assertion→Tailwind decision record exists in
  `doc/`. The in-tree web styling infra is
  `pkgs/web-gen-api-axum/css/tailwind.config.js` with an empty
  `theme.extend`, no plugins, and content globs over `*.html` plus
  `**/*.rs` (with an `rs` extractor mapping Rust `C::…`/`M::…` consts
  to class spellings).

## Decision summary

| Question                               | Decision                                                                                                                                                                                                    |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What a semantic element becomes on web | A fixed DOM element per registry kind (§2) — the element's own tag carries semantics; styling never changes the tag                                                                                         |
| What an assertion becomes on web       | Tailwind utility classes generated from theme keys, one source of truth in the theme file (§3). Content never names a utility, colour, or size (§7.1)                                                       |
| Which theme keys must exist            | The §3.7 key list under `theme.extend` (colours, type steps, spacing, radii, shadows); missing keys are coverage warnings, never silent fallbacks (§7.6)                                                    |
| How generated classes reach Tailwind   | The build emits the generated class set explicitly (safelist/data path, §3.8) — content-scanning alone cannot see classes no author wrote                                                                   |
| Cascade position of everything         | Adopted verbatim from #5854: platform defaults → theme/token values → semantic assertions → override map → raw CSS web-only, per-property (§4). Nothing here re-orders it                                   |
| Raw CSS on web                         | Layer 5, wins per-property over assertion-derived utilities, every override listed in the composition report; cascade layering, never `!important` escalation (§5)                                          |
| Raw HTML on web                        | `<raw kind="html">` passes through verbatim inside the web artefact, sanitised per #5785 contexts; `<raw kind="css">` is appended as layer 5. Both are greppable and enumerated in the coverage report (§6) |
| States                                 | #5850 §5 state set and precedence, composed as Tailwind variants; focus-visible is never suppressible (§3.6)                                                                                                |
| Dark / high-contrast                   | `dark:` variant plus forced-colors variants keyed to the #5850 §10.8 variant requirement (§3.1)                                                                                                             |

## 1. Assumptions against unsettled inputs (stated, not silent)

Parent issue #5679 is open with no settled decision record, #5850's
decision lives in an issue comment rather than a `doc/` record, and
#5855's record is open in review (PR #7921). Following the packet for
this ticket and the #5852/#5853/#5854 precedent, this record states its
assumptions instead of waiting:

- **A1 — the parent model as written today.** Semantic markup describes
  intent; styling is semantic style assertions over named roles; each
  target renders assertions in its own idiom. Where the parent posed
  an open either/or that this ticket must close (raw-CSS cascade
  position, raw-HTML mixing), the sections below pick a branch and say
  so. Anything that would change a parent decision is named out of
  scope in §9, not decided here.
- **A2 — element kinds are the landed #5849 registry.** The 32-element
  closed registry (§2 row labels) is fixed input. A vocabulary version
  change that renames kinds re-issues §2's rows, not its decisions.
- **A3 — the #5850 vocabulary as written.** Dimensions are the closed
  v1 set (tone, emphasis, density, text, gap, pad, corner, elevation);
  colour/type/state/spacing roles are #5850 §§3–6; one `assert`
  attribute per element; no raw values in source. No role is added,
  removed, or renamed here. If #5850's rows change, §§2–3 are
  re-issued against the new names.
- **A4 — the #5855 pipeline shape as written.** Parse→validate→
  compose→lower→emit, compile-time-dominant, deterministic
  (same source + tables + toolchain ⇒ byte-identical output), mapping
  tables as lowering data, JSON composition/coverage report. This
  record supplies the web mapping data; it redefines nothing about
  the pipeline or its reports. If PR #7921 lands with a changed
  shape, only the data-format binding in §3.8 is re-issued.
- **A5 — the in-tree Tailwind setup as found.** `theme.extend` is
  empty today: §3.7's key list is a requirement on the theme file,
  not a description of it. The `rs` extractor convention (Rust
  `C::…`/`M::…` consts → class spellings) is the existing mechanism
  generated class names must remain compatible with.
- **A6 — no parent or sibling decision is changed.** The cascade is
  #5854's; the platform-conflict rule is #5850 §8's (platform wins
  for behaviour/accessibility; assertion wins for identity subject
  to platform clamps — on web, "platform" means UA defaults,
  accessibility media features, and forced-colors); native/GUI
  mappings are #5852's/#5853's and untouched here.

## 2. Element kind → DOM element (web)

One fixed DOM element per registry kind. The tag carries the semantic
role; classes carry only the assertion rendering from §3. Interactive
kinds keep their native element (never a `<div>` with handlers) so
keyboard behaviour and the #5849 §6 contracts come from the platform.

| Element                                      | DOM                                                                       | Notes                                                                                                             |
| -------------------------------------------- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `document`                                   | `<html>` shell (head + body)                                              | Host page; theme/dark-mode hooks live here                                                                        |
| `page`                                       | `<main>`                                                                  | One per routable screen                                                                                           |
| `section`                                    | `<section>` + `aria-labelledby`                                           | Title element provides the label                                                                                  |
| `nav`                                        | `<nav>`                                                                   |                                                                                                                   |
| `heading` (`level` 1–6)                      | `<h1>`–`<h6>`                                                             | Level maps 1:1; no level-skipping fixups                                                                          |
| `card`                                       | `<section>`                                                               | Distinguished from `section` by card surface role, not tag                                                        |
| `list` (`unordered`/`ordered`/`description`) | `<ul>` / `<ol>` / `<dl>`                                                  |                                                                                                                   |
| `item`                                       | `<li>` (or `<dt>`+`<dd>` under `description`)                             | `selected` → `aria-selected`                                                                                      |
| `table` / `row` / `cell` (`header`/`data`)   | `<table>` / `<tr>` / `<th scope>` / `<td>`                                |                                                                                                                   |
| `image`                                      | `<img>`                                                                   | `src`/`alt` pass through; decorative `alt=""` kept                                                                |
| `link`                                       | `<a href>`                                                                | `visited` state is `:visited`-limited by the platform (A6)                                                        |
| `master-detail` / `detail`                   | `<div>` regions with labels                                               | Layout between panes is structural, not an assertion                                                              |
| `text`                                       | `<p>`                                                                     |                                                                                                                   |
| `strong` / `emphasis`                        | `<strong>` / `<em>`                                                       | Never `<b>`/`<i>`; meaning, not pixels                                                                            |
| `form`                                       | `<form>`                                                                  | `invalid` → `aria-invalid` on the form; `submitting` → `aria-busy`                                                |
| `field` (`text`/`password`/`number`/`date`)  | `<input type=…>` + `<label>`                                              | Label association is structural and mandatory                                                                     |
| `field` (`checkbox`)                         | `<input type=checkbox>` + `<label>`                                       |                                                                                                                   |
| `field` (`choice`)                           | `<select>` (+ `<option>`) or radio group for small fixed option sets      | Single vs multiple from `multiple`                                                                                |
| `field` (`file`)                             | `<input type=file>`                                                       | `accept` passes through                                                                                           |
| `alert` (`error`/`warning`/`info`/`success`) | `<div role=alert>` (assertive) for `error`; `<div role=status>` otherwise | Live-region politeness follows the kind                                                                           |
| `button`                                     | `<button type>`                                                           | `disabled`/`busy` → `disabled` + `aria-disabled`; never an `<a>` styled as a button                               |
| `dialog` (`open`/`closed`)                   | `<dialog>`                                                                | Closed renders nothing; open uses modal semantics                                                                 |
| `menu` / `menu-item`                         | `<div role=menu>` / `<div role=menuitem>`                                 | `checked` → `aria-checked` on toggling items                                                                      |
| `toolbar`                                    | `<div role=toolbar>`                                                      |                                                                                                                   |
| `sidebar` (`collapsed`/`expanded`)           | `<aside>`                                                                 | `aria-expanded` on the toggle; collapsed keeps content in the accessibility tree unless the design says otherwise |
| `tabs` / `tab`                               | `<div role=tablist>` / `<button role=tab>` + panel                        | `selected` → `aria-selected` + `tabindex` management                                                              |
| `settings` / `preference`                    | `<section>` / labelled control per `kind`                                 | Same control mapping as `field` kinds                                                                             |

## 3. Assertion role → Tailwind utilities

Every mapping below is keyed by #5850 role names and resolves through
`theme.extend` keys (§3.7). The lowerer emits utilities; authors never
write them.

### 3.1 Colour roles → `theme.extend.colors`

| Role(s)                                        | Colour key(s)                        | Utilities                                                    |
| ---------------------------------------------- | ------------------------------------ | ------------------------------------------------------------ |
| `surface-base/raised/sunken/overlay/inverse`   | `surface-*` (6 keys)                 | `bg-surface-raised` etc.                                     |
| `content-primary/secondary/muted/inverse/link` | `content-*` (5 keys)                 | `text-content-primary` etc.                                  |
| `accent`, `accent-content`                     | `accent`, `accent-content`           | `bg-accent text-accent-content`                              |
| status `success/warning/danger/info`           | 4 pairs (`<name>`, `<name>-surface`) | `text-danger`, `bg-danger-surface`, `border-danger`          |
| `border-subtle/strong/focus`                   | `border-*` (3 keys)                  | `border-border-subtle`, `focus-visible:outline-border-focus` |

Each key carries light, dark, and high-contrast values or an explicit
`inherit` marker visible to coverage tooling (#5850 §10.8). Dark mode
resolves via the `dark:` variant; high-contrast resolves via
forced-colors variants (e.g. `forced-colors:border-[CanvasText]`),
keeping the semantic key as the single source: the variant changes
the value binding, never the class the assertion emits.

### 3.2 Type roles → `fontSize` / `fontWeight` / `lineHeight`

| Role                                 | Utilities (from theme keys)                              |
| ------------------------------------ | -------------------------------------------------------- |
| `display`                            | `text-step-plus-5 leading-tight font-bold`               |
| `heading`                            | `text-step-plus-2 leading-tight font-semibold`           |
| `body`                               | `text-step-0 leading-normal`                             |
| `body-small`                         | `text-step-minus-1 leading-normal`                       |
| `caption`                            | `text-step-minus-1 leading-tight text-content-secondary` |
| `label`                              | `text-step-0 leading-tight font-medium`                  |
| `code`                               | `font-mono text-step-minus-1 leading-normal`             |
| `link` (delta over surrounding text) | `text-content-link underline`                            |

Face categories are `font-sans` / `font-serif` / `font-mono` theme
bindings; `text-step-*` are `fontSize` keys (`minus-1`…`plus-5`);
weights are `regular/medium/semibold/bold`. No role emits an
arbitrary `text-[…]` value.

### 3.3 Spacing roles → `spacing`

`space-0`…`space-8` are `spacing` keys on a rem scale anchored to a
4px base. `pad-*` emits `p-*`; `gap-*` emits `gap-*`. Density is a
container-level scale factor on spacing/padding utilities only
(`comfortable` = 1×, `compact` ≈ 0.75× via a density-scoped theme
multiplier); density never changes type steps (#5850 §6). Layout
itself (flex/grid placement, pane splits) comes from semantic
structure, not spacing assertions.

### 3.4 Corner / elevation → `borderRadius` / `boxShadow`

`corner-square/rounded/full` → `rounded-none` / `rounded` /
`rounded-full` (theme `borderRadius` keys, so a theme may reinterpret
them per §3.7). `elevation-flat/raised/overlay` → `shadow-none` /
`shadow` / `shadow-lg` (theme `boxShadow` keys: ordering only, the
rendering is the theme's).

### 3.5 Emphasis → class bundles

Emphasis composes existing role bindings; it never introduces new
colours or sizes:

| Emphasis                | Bundle                                                                                   |
| ----------------------- | ---------------------------------------------------------------------------------------- |
| `primary`               | `bg-accent text-accent-content`                                                          |
| `destructive`           | `text-danger border-danger` (+ `bg-danger-surface` tint where the element has a surface) |
| `subtle`                | `bg-transparent` (inherits surrounding content roles)                                    |
| default (none asserted) | element's base roles only                                                                |

### 3.6 States → variant composition

States in v1: `hover`, `focus-visible`, `active`, `selected`,
`disabled`, `error`, `loading` (#5850 §5). Composition rules:

- `hover` → `hover:` prefixed state utilities (colour shifts use the
  same role keys at a darker step defined in the theme, not new keys).
- `focus-visible` → `focus-visible:outline-border-focus` (or the
  theme's focus ring); it is composited on top of every other state
  and no combination of states, themes, or overrides may suppress it
  (#5850 §10.4).
- `active` → `active:` utilities; `selected` → `aria-selected:` /
  `data-[state=selected]:` utilities plus `aria-selected="true"`.
- `disabled` → `disabled:` utilities (muted content/surface roles,
  no elevation) plus the `disabled` attribute where the DOM element
  supports it. Disabled beats all other states in the emitted order.
- `error` → danger-role swap (`text-danger`, `border-danger`) plus
  `aria-invalid` where applicable; beats `selected`/`hover`/`active`.
- `loading` → `aria-busy` plus the theme's pending treatment
  (skeleton/progress per element); content stays in the tree.
- Precedence is encoded in emit order and selector specificity
  following #5850 §5 (`disabled` > `error` > `selected`/`hover`/
  `active`, focus-visible always on top). Raw CSS (layer 5) may not
  re-order state precedence — see §5.

### 3.7 Required `theme.extend` keys

The theme file must define (or explicitly mark `inherit`/platform-
default with a coverage warning): the 6 `surface-*`, 5 `content-*`,
`accent` + `accent-content`, 4 status pairs, 3 `border-*` colours;
`fontSize` steps `minus-1`…`plus-5`, `fontWeight`
`regular/medium/semibold/bold`, `lineHeight` `tight/normal/loose`;
`spacing` `space-0`…`space-8` plus the `compact` multiplier;
`borderRadius` for three corner roles; `boxShadow` for three
elevation roles. A missing key is a build-time coverage warning
naming the assertion and the target ("this assertion has no web
mapping; platform default will be used"), never a silent fallback.

### 3.8 Emitting classes the scanner never sees

Content-scanning (`content` globs, including the `rs` extractor)
cannot discover classes no author wrote, so the lowerer emits the
generated class set explicitly as lowering data (per the #5855
compile-time model): either a checked-in generated module the globs
cover, or a safelist derived from the mapping tables. Generated
names must remain compatible with the `rs` extractor spelling rules.
Determinism follows #5855: same source + tables + toolchain ⇒
byte-identical class output.

## 4. Cascade (adopted, not decided)

The cascade is #5854 §2.1, adopted verbatim — weakest to strongest:

1. platform defaults (UA stylesheet, user preferences, media features);
2. theme/token values;
3. semantic assertions (this record's tables);
4. override map (4a style overrides, then 4b #5853 presentation
   overrides — a presentation override supersedes layers 3–4a for
   appearance, with layer-3 rendering as fallback);
5. raw CSS, **web-target only**, per-property (§5).

Within-layer precedence (instance > kind+variant+state > kind
default; ties are build errors; source order is never a tiebreaker),
target locality (declarations for one target are no-ops elsewhere),
and the conflict rule shared with #5850 §8 (platform wins for
behaviour/accessibility; assertion wins for identity subject to
platform clamps; every substitution, clamp, or default is reported
by tooling, never silent) all apply unchanged. This record adds no
layer, re-orders none, and grants no new tiebreaker.

## 5. Raw CSS on web (layer 5)

- Raw CSS comes only from `<raw kind="css">` blocks and the web-only
  edge of override maps. It wins **per-property** over
  assertion-derived utilities: a raw `color` declaration beats the
  assertion's `color` utility without disturbing the assertion's
  `padding` utilities.
- Mechanically this is cascade-layer ordering (author raw layer above
  the utilities layer), **never `!important` escalation** (rejected,
  following #5854 §10.6): `!important` in author raw CSS is a build
  error, not a bigger hammer.
- Every raw-CSS win is listed in the composition report (#5854 §6b):
  property, winning rule, overridden assertion role. Unlisted wins
  are a defect in the lowerer, not author freedom.
- Raw CSS may not re-order state precedence (§3.6), suppress
  focus-visible styling, or set properties outside the element's own
  subtree (no global selectors leaking across elements).
- Raw CSS does not exist on native/custom targets: a raw-CSS rule
  attached to content is ignored there and the coverage tooling warns
  (assertion has web-only styling); essential appearance must be
  expressible as assertions alone (#5850 §10.3, invariant §7.3).

## 6. Raw-HTML mixing on web

- `<raw kind="html">` (the only raw-HTML form; there is no
  `inner-html`-equivalent element in v1, #5849 §3) passes through
  **verbatim** into the web artefact at the element's position.
- Pass-through is still sanitised per #5785's text/attribute/URL
  contexts: raw HTML is an authoring escape hatch, not a sanitiser
  bypass. A sanitiser finding is a build error naming the block.
- Raw HTML carries no assertions of its own; surrounding assertions
  do not style inside it (no inheritance across the boundary —
  the boundary is greppable precisely so tooling can see it).
- Fallback on other targets: raw HTML has no native/custom meaning;
  per the parent's open per-class question, the web-side contract is
  pass-through, and the per-target fallback (webview / translate /
  author-supplied fallback) is owned by the native/GUI mapping
  records (#5852/#5853), not decided here.
- Every raw block (html or css) is enumerated in the per-target
  coverage report, so `grep -c '<raw'` over sources plus the report
  gives the complete escape-hatch inventory.

## 7. Invariants implementation must hold

1. No raw values in semantic assertions: no hex, no px numbers, no
   concrete font names, no Tailwind class names in source. Tooling
   rejects them at authoring time (#5850 §10.1).
2. The dimension set is closed in v1; extension happens by theme
   tokens or a vocabulary version change, never per-program custom
   dimensions in content (#5850 §10.2).
3. Essential appearance is always expressible in assertions alone;
   raw CSS is enhancement-only (#5850 §10.3).
4. Focus-visible styling is never fully suppressed by any state
   combination, theme, override, or raw CSS (§3.6).
5. Typography and spacing respond to platform scaling/density as a
   system (rem-anchored steps, density multiplier on spacing only);
   no assertion pins an absolute size (#5850 §10.5).
6. Every platform substitution, clamp, missing mapping, and raw-CSS
   win is surfaced by tooling (coverage/lint/composition reports);
   silent divergence is a defect (#5850 §10.6; §3.7; §5).
7. State precedence is fixed by the vocabulary (§3.6); themes and
   raw CSS may change values but not the order (#5850 §10.7).
8. Every role resolves for light, dark, and high-contrast variants
   or carries an explicit inherit marker visible to coverage
   tooling (#5850 §10.8; §3.1).
9. Same source + tables + toolchain ⇒ byte-identical web output
   (determinism, via #5855).
10. No `!important` in author raw CSS; no cross-element global
    selectors in raw blocks (§5; §6).

## 8. Alternatives considered and rejected (with cost accepted)

- **Tailwind class names as assertions.** Rejected: web-only,
  presentational rather than semantic, meaningless to native/custom
  targets (#5850 §11). Cost accepted: the lowerer and theme file
  carry the mapping burden authors would otherwise hand-write.
- **CSS property names/values as the assertion vocabulary.**
  Rejected: leaks the web platform into every target; native targets
  would need a CSS engine (#5850 §11).
- **Free-form per-element theme JSON in content.** Rejected: forks
  appearance into content and makes coverage tooling impossible
  (#5850 §11). Cost accepted: novel one-off styling must go through
  raw blocks (visible, reported) or a vocabulary change.
- **Per-target assertion sets in content.** Rejected: doubles the
  authoring surface and guarantees divergence (#5850 §11). Targets
  differ in theme/override files, not in content.
- **Content-scanning discovery of generated classes.** Rejected:
  the scanner cannot see classes no author wrote; relying on it
  silently drops assertion rendering whenever the scanner's globs
  miss. Explicit emission (§3.8) costs a generated artefact but is
  observable and deterministic.
- **`!important` as the raw-CSS mechanism.** Rejected (following
  #5854 §10.6): escalates every conflict into an arms race and
  defeats per-property resolution. Cost accepted: the lowerer must
  implement real cascade-layer ordering.
- **Styling inside raw HTML from surrounding assertions.**
  Rejected: inheritance across the raw boundary would make the
  escape hatch ungreppable and its rendering unpredictable. The
  boundary is absolute (§6).

## 9. Relation to parent, splits, and follow-ons

- **No parent decision is changed.** The parent's semantic-markup +
  style-assertion model is consumed as fixed input (A1). Two open
  parent questions are closed here, both in the direction the parent
  sketch listed first and siblings already adopted: cascade order
  (#5854, adopted in §4) and warn-never-silent tooling (adopted in
  §§3.7/5/7.6). Would-be amendments are explicitly out of scope:
  Tailwind version pinning and whether generated classes are
  author-visible/editable (parent's open question — this record is
  agnostic: the tables bind role names, not a Tailwind version);
  raw-HTML fallback on native targets (owned by #5852/#5853, §6);
  new roles or dimensions (a #5850 vocabulary change, §8).
- **No further design split is needed.** The per-target mapping
  tables for web are complete in §§2–3; native and custom mappings
  belong to #5852/#5853 and are not revisited here.
- **No implementation ticket is filed by this record.**
  Implementation timing is a separate decision per the filing note on
  #5851. When a lowering/theme implementation ticket is filed, it
  should link this record as its mapping source; its acceptance
  oracle is the §7 invariant list plus byte-identical determinism
  (§7.9) checked against the #5855 conformance corpus shape.
