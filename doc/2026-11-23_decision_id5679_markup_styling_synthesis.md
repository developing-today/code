# Decision: markup & styling model for programs-as-websites — parent synthesis

- **Date:** 2026-11-23
- **Status:** decision record — the parent synthesis for #5679. No
  implementation is included in this change.
- **Issue:** [#5679](https://github.com/developing-today/code/issues/5679),
  "id: design: Markup & styling model for programs-as-websites —
  semantic markup, style-override mapping". Extends
  [#2146](https://github.com/developing-today/code/issues/2146) (closed).
  Programme: [#2056](https://github.com/developing-today/code/issues/2056).
- **What this record is:** the seven decomposition children of #5679
  were each decided against this parent while it was still open, and
  each stated its assumptions against the parent rather than waiting
  (#5850 A1–A3, #5851 A1, #5852 A1/A4, #5855 A1). All seven are now
  closed with records on `main`. This record is the parent pass the
  issue's programme note promised: it ties the seven child decisions
  into one settled contract, **ratifies** the branches the children
  took on the parent's open either/ors (§5, §10), reconciles the two
  places where children stated the same rule at different resolutions
  (§9), and carries forward — by name — the questions that remain
  genuinely open (§11). It introduces no new mechanism.
- **Children consumed (all closed, records on `main`):**
  1. [#5849](https://github.com/developing-today/code/issues/5849) —
     semantic markup vocabulary v1:
     `doc/2026-11-08_decision_id5849_semantic_markup_vocabulary_v1.md`.
  2. [#5850](https://github.com/developing-today/code/issues/5850) —
     style assertion vocabulary: decision record as an issue comment
     (task 12817, 2026-11-08,
     <https://github.com/developing-today/code/issues/5850#issuecomment-6719140810>).
     The record form is the one that issue allows; §4 cites its
     sections as settled input, so the vocabulary now has a `doc/`
     anchor by reference from this record.
  3. [#5851](https://github.com/developing-today/code/issues/5851) —
     web rendering, assertion → Tailwind:
     `doc/2026-11-09_decision_id5851_assertion_tailwind_mapping.md`.
  4. [#5852](https://github.com/developing-today/code/issues/5852) —
     native rendering, assertion → OS-standard controls:
     `doc/2026-11-08_decision_id5852_native_rendering_os_standard_controls.md`.
  5. [#5853](https://github.com/developing-today/code/issues/5853) —
     custom GUI rendering as first-class overrides:
     `doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`.
  6. [#5854](https://github.com/developing-today/code/issues/5854) —
     per-target customisation system:
     `doc/2026-11-08_decision_id5854_per_target_composition.md`.
  7. [#5855](https://github.com/developing-today/code/issues/5855) —
     rendering pipeline & conformance suite:
     `doc/2026-11-08_decision_id5855_rendering_pipeline_conformance.md`.
- **Related settled contract, not part of this synthesis:**
  [#5858](https://github.com/developing-today/code/issues/5858)
  (portable theme format & precedence, from #5680;
  `doc/2026-11-10_design_id5858.md`) owns the signed portable _board_
  theme object and its per-token precedence (viewer accessibility >
  viewer preference > post > board > platform default). That is a
  different cascade over a different artefact: it resolves which
  token values a board/post/viewer supplies. This record's cascade
  (§5) resolves how values, assertions, and overrides compose when a
  programme renders. #5858's record positions its token contract as
  input the mapping records consume; nothing in either record
  overrides the other, and §9.3 names the seam.
- **Downstream gate:** [#6163](https://github.com/developing-today/code/issues/6163)
  (portable board theme rendering; queue task 10984) is hard-gated on
  "#5679 settled" and #5858. #5858 is closed; this record is the
  #5679 settlement that gate names.
- **Evidence baseline:** `origin/main` at `7e7af3f72d`, 2026-11-23
  (task 21486). Fresh reads this pass: #5679 OPEN, no hard-stop
  labels; #5849–#5855 all CLOSED; the six `doc/` child records and
  the #5850 comment record read in full; #6163 OPEN with the gate
  text above.

## Decision summary — the settled contract in one table

| Question                 | Settled answer (record of decision)                                                                                                                                                                                                       |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Layers                   | Two: raw HTML/CSS always supported, never gated; semantic markup is an addition, admitted raw only inside explicit `<raw>` regions (#5849 §3)                                                                                             |
| Semantic syntax          | HTML-shaped source from a closed registry; quoted attributes only; no presentation attributes (#5849 §2)                                                                                                                                  |
| Element set              | Closed v1 registry of **32 elements** — document/structure, inline text, forms/actions, application chrome; every element renderable on all three targets, none web-only (#5849 §§4–5)                                                    |
| Accessibility            | Role, accessible-name source, state set, and keyboard contract are properties of the element kind, checked at build time (#5849 §6)                                                                                                       |
| Binding                  | Fixed slots (`bind`, `items`, `show`, `disabled-when`, `action`, `on-*`) whose values are **names** in the programme's state/action tables — no expressions, no target-specific code (#5849 §8)                                           |
| Style assertions         | (dimension, role-value) pairs in one `assert` slot per element; closed dimension set (tone, emphasis, density, text, gap, pad, corner, elevation); **no raw values in content** (#5850 §§2–6)                                             |
| Style vocabulary         | Named colour/typography/state/spacing roles with per-target values in theme files; every role defined for light/dark/high-contrast or explicitly marked inherit (#5850 §§3–6)                                                             |
| Cascade                  | **Platform defaults → theme/token values → semantic assertions → override map (style, then presentation) → raw CSS (web only)**, per property, deterministic, source order never a tiebreaker in layers 1–4 (#5854 §2; ratified §4 below) |
| Platform conflicts       | **Platform wins for behaviour and accessibility; assertion wins for identity, subject to platform clamps** — every clamp/substitution/default reported by tooling (#5850 §8, #5852 §4; ratified §4 below)                                 |
| Unmapped assertions      | **Warning, never silent**: coverage tooling reports "no mapping on this target; platform/layer default used" (#5850 §9, #5852 §3.1, #5854 §6, #5855 §3.3)                                                                                 |
| Web rendering            | Element kind → fixed DOM element; assertion → Tailwind utilities generated from `theme.extend` keys; raw CSS is layer 5 per-property via cascade layers, never `!important` (#5851 §§2–5)                                                 |
| Native rendering         | Element kind → OS-standard control (Mac/Windows/GNOME tables); assertions change only what the platform exposes; presentation overrides never apply to native standard controls (#5852 §§2–4, #5853 §4)                                   |
| Custom GUI rendering     | Wholly custom presentations are **presentation overrides** in the target's override map: asset or registered drawn renderer, full state coverage or build error, standard rendering always the fallback (#5853 §2)                        |
| Per-target customisation | Theme/token files, override maps, three distinct asset mechanisms, conditional blocks (target-fact predicates only, embellishment only, evaluated before the cascade); per-target page copies forbidden (#5854 §§3–5)                     |
| Pipeline                 | Hybrid, compile-time-dominant: parse → validate → compose → lower per target → emit artefact + JSON report; mapping tables are lowering _data_; runtime only binds state and instantiates (#5855 §2)                                      |
| Conformance              | Versioned golden corpus per target — DOM/class expectations (web), control-kind + accessibility-tree expectations (native/GUI); pixels are never the gate (#5855 §3)                                                                      |

## 1. The model in one view

A programme has **one content/semantic source**. It is written in the
semantic markup of §3, styled by the assertions of §4, and may contain
explicit `<raw kind="html">` / `<raw kind="css">` regions — the raw
layer is always available and never gated by the semantic layer
(#5849 §3.1). Targets — web, custom GUI, and native (Mac, Windows,
GNOME) — customise only through per-target side declarations: theme
files, override maps, asset tables, and the source's conditional
blocks. Copying the page per target is forbidden, and §7's worked
example (in #5854 §8) shows one form rendered three ways from one
source. Pixel-identical rendering across targets is a non-goal, as
the parent stated: native targets look native.

## 2. Semantic markup (settled by #5849)

- **Syntax.** HTML-shaped, from the closed 32-element registry;
  element and attribute names follow HTML where HTML has the concept;
  values are always quoted attributes; content is text. Markdown-
  adjacent and s-expression syntaxes were considered and rejected
  with reasons (#5849 §§2.2, 12). The parent's open syntax question
  is therefore settled: **HTML-shaped tags**.
- **Registry.** Four groups: document/structure (`document`, `page`,
  `section`, `nav`, `heading`, `card`, `list`, `item`, `table`,
  `row`, `cell`, `image`, `link`, `master-detail`, `detail`), inline
  text (`text`, `strong`, `emphasis`), forms/actions (`form`,
  `field`, `alert`, `button`, `dialog`), application chrome (`menu`,
  `menu-item`, `toolbar`, `sidebar`, `tabs`, `tab`, `settings`,
  `preference`, `note`) — with per-element required attributes,
  child/sibling rules, field and preference kinds, and declarative
  constraints, all build-checked (#5849 §4). Menus, toolbars,
  sidebars, tabs, settings/preferences, and master-detail **are** in
  v1; trees, charts, carousels, rich-text editing, canvas, maps,
  spreadsheets, command palettes, drag-and-drop, regex validation,
  and i18n interpolation are deferred with reasons (#5849 §9). The
  parent's element-set question is settled by that split.
- **Versioning.** `document` declares `version="1"`; adding,
  renaming, or removing an element or changing a state set is a
  version change; compatible widenings are recorded in the registry
  changelog (#5849 §2.4).
- **Accessibility by construction.** Role, name source, state set,
  and keyboard contract are registry properties; an unnamed
  interactive element is a build error; `alert` is the only
  live-region element; there is no `tabindex` (#5849 §6).
- **Binding.** The fixed slot set of #5849 §8: values are names into
  the programme's declared state/action tables. No expressions, no
  code, no target identifiers in source. State shape and action
  implementation belong to the programme model (#2146), not to
  markup.
- **Data safety.** Bound values enter only text, quoted-attribute,
  or sanitized-URL contexts per #5785; emitted output is never
  parsed as source; there is no script context in the vocabulary
  (#5849 §3.3).

## 3. Style assertions (settled by #5850)

- **Grammar.** An assertion is a (dimension, role-value) pair in the
  element's single `assert` slot. The v1 dimension set is closed:
  tone, emphasis, density, text, gap, pad, corner, elevation
  (#5850 §2). Assertions never change semantics — they cannot add,
  remove, or rebind a role, name, state, or event (#5849 §7).
- **Colour roles.** Surface (base/raised/sunken/overlay/inverse),
  content (primary/secondary/muted/inverse/link), accent +
  accent-content, status pairs (success/warning/danger/info, colour
  - surface tint), border (subtle/strong/focus). State colours are
    derived by the target unless a theme overrides the state token
    (#5850 §3).
- **Typography roles.** display, heading, body, body-small, caption,
  label, code, link — bundles of typeface _category_, modular-scale
  step, weight role, and line-height role; platform text scaling
  applies to the scale as a whole (#5850 §4).
- **States.** hover, focus-visible, active, selected, disabled,
  error, loading — each a delta over base roles, with fixed
  precedence: disabled beats all; error beats selected/hover/active;
  focus-visible is always composited on top and never suppressible
  (#5850 §5).
- **Spacing/density/shape/elevation.** space-0…space-8 roles for
  `gap`/`pad`; one density dimension (comfortable/compact) scaling
  spacing, never type steps; corner-square/rounded/full and
  elevation-flat/raised/overlay as intent/ordering roles whose
  rendering is the target's (#5850 §6).

## 4. The cascade and the conflict rules (ratified here)

The children converged on one cascade, stated at two resolutions.
The settled statement is **#5854 §2.1's five-layer order**, weakest
to strongest, applied per property:

1. platform defaults → 2. theme/token values → 3. semantic style
   assertions → 4. per-target override map (4a style overrides,
   then 4b #5853 presentation overrides) → 5. raw CSS, **web target
   only**.

#5850 §7's three-layer statement (assertions < per-target overrides
< raw CSS) is the same order restricted to the author-controlled
layers; #5851 §4 already adopts #5854 verbatim. This record ratifies
the five-layer order as the parent's decision, so the children's
"assumption A3" branches are no longer assumptions. Within-layer
rules are #5854 §2.2 (specificity: instance > kind+variant+state >
kind default; exact ties are build errors; source order is never a
tiebreaker in layers 1–4) and #5853 §2.5 inside the override map.

**Platform-conflict rule (ratified):** platform wins for behaviour
and accessibility (keyboard, focus visibility, minimum contrast,
forced-colours/high-contrast modes, reduced motion, platform text
scaling); the assertion wins for identity (accent/brand colour,
typeface category) except where it would breach platform contrast
or legibility minimums — then the platform clamps and tooling
reports the clamp with both values (#5850 §8, #5852 §4). This is the
branch the parent sketch listed first, recorded by #5850 for the
vocabulary layer and decided again by #5852 §4 for native rendering;
this record makes it the parent's settled rule for all targets (on
web, "platform" means UA defaults, accessibility media features,
and forced-colors, per #5851 A6).

**Mode ownership (settled):** on native targets the OS selects the
mode (dark/light/high-contrast) and the programme supplies values
for every mode it ships; on web/GUI targets an expressed user/OS
preference selects, else the theme's declared default; an author
may not force a mode against an expressed preference (#5854 §3).
Every role carries light/dark/high-contrast values or an explicit
inherit marker visible to coverage tooling (#5850 §10.8). This
settles the parent's token-ownership question: **the OS/user owns
mode selection; the programme's theme owns the values**.

**Nothing silent (ratified):** every tie, fallback, clamp,
platform-default, ignored raw-CSS rule, degraded raw-HTML
construct, and raw-CSS reported override appears in the build's
composition/coverage reporting (#5854 §6, #5855 §§2.1/3.3). An
unreported divergence between source and rendering is a defect
under every child record; this record makes it a contract-level
invariant (§8.6).

## 5. Per-target rendering (settled by #5851, #5852, #5853)

- **Web (#5851).** One fixed DOM element per registry kind — the tag
  carries the semantics, interactive kinds keep their native
  elements. Assertions render as Tailwind utilities generated from
  `theme.extend` keys (the §3.7 key list of that record is a
  requirement on the theme file); generated classes are emitted
  explicitly as lowering data because content scanning cannot see
  classes no author wrote. Raw CSS is layer 5, per-property, via
  cascade-layer ordering — `!important` in author raw CSS is a build
  error. Raw HTML passes through verbatim at its position, still
  sanitised per #5785, and assertions do not style inside it.
- **Native (#5852).** One OS-standard control per element kind per
  platform (Mac/Windows/GNOME tables). The renderer configures
  standard controls through platform APIs; it never subclasses,
  re-paints, or substitutes a drawn control or a webview. An
  assertion changes only what the platform exposes (semantic
  colour/tint, text style, size/density class, spacing in platform
  units); anything else is an explicit **platform-default** entry in
  the theme file with a coverage warning — mapped or marked, never
  absent. Corner is intent the standard control's shape overrides;
  density is platform-default on GNOME; state behaviour is the
  control's own, checked against the #5850 §5 contract in
  conformance.
- **Custom GUI (#5853).** A wholly custom presentation (image
  button, drawn control) is a presentation override in the target's
  override map, bound by instance or by kind+variant/emphasis+state
  set. Two kinds only: asset presentations (per-state assets) and
  drawn presentations (renderers registered by name in the target's
  override pack — target-side code, never content-carried). An
  override must cover the element kind's full state set or fail the
  build; `fallback: standard` is structural, so an element never
  renders as nothing; essential content never lives in an asset.
  Presentation overrides do not apply on native targets (#5853 §4,
  #5852 §6) — only content assets carry over.

## 6. Composition and customisation (settled by #5854)

- **Themes** are per-target token→value tables, layered programme
  base theme → per-target theme → mode variant (a partial table).
  A theme supplies values only; it cannot introduce, remove, or
  rebind an assertion or override (#5854 §3).
- **Assets** move by three distinct mechanisms that must not be
  conflated: the theme asset table (token assets), override-map
  content-asset substitution, and #5853 presentation assets.
  Substitution changes bytes, never semantics; a missing asset is a
  build error for the first two (#5854 §4).
- **Conditional blocks** are evaluated before the cascade; their
  predicates are target facts only (target name, form-factor class,
  declared capability flag) — no expressions, no user/state data,
  no nesting in v1; an unknown predicate term is a build error.
  Blocks carry embellishment only: removing every block must leave
  a complete, usable programme on every target, and conformance
  renders the stripped source to prove it (#5854 §5).
- **Worked example.** #5854 §8 is the parent's required worked
  example: one settings form (name/email fields, a primary `save`
  action asserting the `accent` role, a web-only conditional hint)
  rendered to (a) Tailwind HTML with a reported raw-CSS override,
  (b) a custom GUI with a per-state image-button presentation
  override, and (c) native Mac/Windows/GNOME standard controls —
  from the same source, with no per-target copy.

## 7. Pipeline and conformance (settled by #5855)

- **Pipeline.** Five stages over one target-neutral IR: parse →
  validate → compose → lower per target → emit (artefact + JSON
  report). Compile-time-dominant hybrid: everything decidable from
  source + target tables is decided ahead of time; the runtime only
  binds state/actions and instantiates controls. The runtime never
  re-parses, re-composes, or re-decides fallbacks — what conformance
  tested is what ships (#5855 §2).
- **Tables are data.** The #5851/#5852/#5853 mapping tables are
  versioned data inputs to lowering, never code branches; changing
  a table is a versioned change with corpus re-blessing (#5855
  §§2.1, 5).
- **Conformance.** A versioned golden corpus: DOM/class expectations
  for web, control-kind + accessibility-tree expectations for
  native and GUI. Screenshot/pixel comparison is advisory only,
  never a gate. Every element kind and assertion role needs a
  corpus case or the suite's own coverage check fails (#5855 §3).
- **Tooling.** One linter (ties, unknown kinds/roles, essential
  content in conditional blocks, and per-target source copies are
  errors; unmapped assertions and reported overrides are warnings)
  and one preview command that runs the pipeline in report mode
  across all targets (#5855 §4).
- **Determinism.** Same source + table/theme versions + toolchain
  ⇒ byte-identical artefacts and reports (#5855 §5).

## 8. Invariants of the settled contract (union of the children's)

1. **One source.** The semantic source is identical for all targets;
   target variation lives only in themes, tables, override maps,
   assets, and conditional blocks. Per-target page copies are
   forbidden (#5854 §9.1, #5855 §5.1).
2. **No raw values in content.** Assertions name roles only — no
   hex, no px/pt, no concrete font names, no target class names.
   Tooling rejects them at authoring time (#5850 §10.1).
3. **Closed vocabularies.** The element registry (#5849) and the
   assertion dimension set (#5850) are closed in v1; extension is
   by version change, never per-programme custom dimensions or
   elements (#5849 §10.1, #5850 §10.2).
4. **Semantics are layer-proof.** No layer — theme, assertion,
   override, raw CSS — changes an element's role, accessible name,
   value, state set, focus order, or interaction contract
   (#5854 §9.4, #5853 §3.1).
5. **Accessibility by construction.** Every element carries its
   role, name, states, and keyboard contract from the registry on
   every target; focus-visible styling is never fully suppressed
   (#5849 §6, #5850 §10.4, #5855 §5.5).
6. **Nothing silent.** Every platform substitution, clamp,
   platform-default, missing mapping, fallback, ignored rule, and
   raw-CSS override is surfaced by tooling (§4 above).
7. **Target locality.** A declaration for one target is a no-op for
   every other target — a conformance-tested property, not a hope
   (#5854 §§2.4, 9.6).
8. **Essential content lives in the semantic layer only.** Never in
   an asset, a presentation, a conditional block, or raw CSS;
   essential appearance is always expressible in assertions alone
   (#5853 §3.5, #5854 §9.7, #5850 §10.3).
9. **Determinism.** The cascade and the pipeline are total and
   deterministic; same inputs ⇒ byte-identical outputs
   (#5854 §9.2, #5855 §5.2).
10. **Emitted output is inert.** Bound values never become markup,
    code, or URL schemes; guest/forwarded output is never parsed as
    source (#5849 §3.3, #5785).

## 9. Reconciliations performed by this synthesis

1. **Cascade resolution.** #5850 §7 (three layers) vs #5854 §2.1
   (five layers): not a conflict — #5854's is the complete order
   and #5850's is its author-controlled subset. §4 ratifies
   #5854's statement (§9 of #5851 already relies on it).
2. **Element-kind names.** #5852 was decided before #5849 settled
   and used the parent body's generic kinds (its A2), with a
   standing note that a settled #5849 renames only its §2 row
   labels. The registry has since settled, and every #5852 row has
   a registry counterpart under the same or the obvious name
   (button/action → `button`; form field → `field` kinds; settings
   → `settings`/`preference`; navigation/sidebar → `nav`/`sidebar`;
   card/media/page-section → `card`/`image`/`section`;
   master-detail → `master-detail`). No #5852 decision depends on a
   kind the registry lacks; its tables are read against the
   registry names from this record on.
3. **The #5858 seam.** #5858's per-token precedence (viewer
   accessibility > viewer preference > post > board > platform
   default) orders _theme-supplying layers for board objects_; §5's
   cascade orders _composition layers for programme rendering_.
   They operate at different points — #5858 resolves which values
   are supplied, #5854 resolves how supplied values compose — and
   #5858's record already frames its token contract as input to
   this family's mapping records. No term in one cascade is
   redefined by the other. If a future programme renders _as_ board
   content and both apply to the same token, the composition order
   is: #5858's cascade resolves the token's value first; that value
   then enters §5's layer 2. This sentence is this record's only
   addition at the seam, and it follows from each record's own
   scoping rather than changing either.
4. **#5850's record form.** The style-assertion vocabulary's
   decision of record is an issue comment, where every sibling's is
   a `doc/` file. The vocabulary is settled either way (the issue
   allowed the form, and #5851/#5852 both consume it as settled);
   this record's §3 and citations give it a `doc/` anchor. Moving
   the text into `doc/` verbatim is a clerical option for a future
   tidy-up pass, not a design question.

## 10. Disposition of the parent's open questions

| #5679 open question                                                                              | Disposition                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Syntax home: HTML-like tags, Markdown-adjacent, or s-expression/DSL?                             | **Settled** — HTML-shaped tags from a closed registry (#5849 §2; alternatives rejected §12)                                                                                                                                                                                                                                                                                             |
| How large is the v1 element set; which GUI concepts are in v1 vs deferred?                       | **Settled** — 32 elements; menus, tabs, master-detail, toolbars, sidebars, settings in; the deferred list and its reasons are #5849 §9                                                                                                                                                                                                                                                  |
| Tailwind as the web rendering: pinned version? Are generated classes author-visible/editable?    | **Carried forward** as OQ-1 (§11) — #5851 binds role names, not a Tailwind version, and is deliberately agnostic                                                                                                                                                                                                                                                                        |
| Unmapped assertion: silent platform-default or build-time warning?                               | **Settled** — warning, never silent (§4)                                                                                                                                                                                                                                                                                                                                                |
| Raw HTML on native targets: webview, subset translation, or author fallback — per element class? | **Settled in shape, one table outstanding** — the rule is author-supplied fallback content or target webview embedding per element class, never outranking the accessibility contract (#5854 §2.3); webview is rejected as _the_ native rendering (#5852 §6). The per-class table itself was assigned to the mapping records but not enumerated in them — carried forward as OQ-2 (§11) |
| Who owns token values for OS-adaptive dark/high-contrast — programme theme or OS?                | **Settled** — OS/user owns mode selection; the theme owns the values for every mode (§4)                                                                                                                                                                                                                                                                                                |

## 11. Open questions carried forward (named, with owners)

- **OQ-1 — Tailwind pinning and generated-class visibility.**
  Whether the web target pins a Tailwind version, and whether the
  generated class set is ever author-visible or author-editable.
  Owner: the web-lowering implementation ticket, when filed (the
  #5851 record's filing note governs timing). Nothing in the
  settled contract depends on the answer: the mapping binds role
  names to theme keys, not to a Tailwind release.
- **OQ-2 — Per-element-class raw-HTML degradation table.** For each
  raw-HTML construct class: author fallback, webview embedding, or
  subset translation, per target. The _rules_ are settled (§10);
  the table is mapping data the #5851/#5852 records assigned but
  did not enumerate. Owner: the native/GUI lowering implementation
  work, which must produce it as table data under #5855's
  tables-are-data rule before raw HTML can render off-web.
- **OQ-3 — Custom-drawn controls on native targets.** Named by
  #5853 §5 and #5854 §11: this would change the parent's
  platform-wins-for-behaviour rule, so it is **not** granted by the
  settled contract and would need its own decision record
  superseding that part of #5679, with Drew's ruling. Recorded
  here so the exclusion is a decision, not an oversight.
- **OQ-4 — Cross-target inheritance of themes/overrides.**
  Rejected for v1 (#5854 §10.3) because it makes target locality
  untestable; revisit only with usage evidence, by a new decision
  record. Not an open design question — a closed one with a stated
  revisit condition.

## 12. Implementation slices of record

No new issues are filed by this record. The design's implementation
follow-ups already exist, filed from #5855, and their timing
remains a separate decision per the children's filing notes:

- [#7918](https://github.com/developing-today/code/issues/7918) —
  rendering pipeline compiler core (**closed**).
- [#7919](https://github.com/developing-today/code/issues/7919) —
  conformance corpus and golden-test harness (open).
- [#7920](https://github.com/developing-today/code/issues/7920) —
  author tooling: linter and multi-target preview (open).

Per-target lowering implementation (the #5851/#5852/#5853 tables as
data, plus OQ-2's table) has no tickets yet; when filed, each
ticket should cite the relevant child record as its oracle and be
linked from the corresponding #5679 decomposition item, as the
child records instruct. Downstream, #6163's design gate is
satisfied by this record together with #5858's.

## 13. Relation to the parent — the close decision

This record changes no child decision; it ratifies the two
parent-level branches the children had to assume (cascade order,
§4; platform-conflict rule, §4), reconciles the statements that
needed it (§9), and names what remains open with owners (§11).
With the seven children closed and this synthesis on `main`, the
design #5679 asked for exists as one settled contract: the markup
vocabulary, the assertion vocabulary, the style-override mapping
for all three targets, the composition system, and the pipeline
that renders and verifies them. The PR landing this record
therefore says `Closes #5679`.
