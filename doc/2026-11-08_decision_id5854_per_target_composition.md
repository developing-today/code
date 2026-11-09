# Decision: per-target composition — themes, overrides, assets, conditional blocks

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5854](https://github.com/developing-today/code/issues/5854),
  "id: design: per-target customisation system (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites"), decomposition
  item 6 ("Per-target customisation system: themes, override maps, asset
  substitution, conditional blocks — with the worked three-target
  example"). Programme: [#2056](https://github.com/developing-today/code/issues/2056).
- **Settled sibling:** [#5853](https://github.com/developing-today/code/issues/5853)
  (closed; `doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`).
  That record fixes presentation overrides inside the override map and
  explicitly leaves the cascade *between* style assertions, override
  maps, and raw CSS to this follow-on (§5 of that record). This record
  settles exactly that ordering, plus theme resolution, asset
  substitution, and conditional blocks.
- **Evidence baseline:** fresh GitHub reads 2026-11-08 (task 13783):
  issue #5854 OPEN, no hard-stop labels; parent #5679 OPEN; siblings
  #5849, #5850, #5851, #5852, #5855 OPEN (§7 names what each still
  owns); #5853 CLOSED with its decision on `main`.

## Decision summary

| Question | Decision |
| --- | --- |
| Composition order (low → high precedence) | (1) target platform defaults → (2) theme/token values → (3) semantic style assertions → (4) per-target override map (style overrides, then presentation overrides per #5853) → (5) raw CSS, web target only (§2) |
| Conditional blocks | Not a cascade layer: they include/exclude non-essential embellishment *before* the cascade runs; included content then cascades normally (§5) |
| Theme resolution | Assertions name roles/tokens, never values; a per-target theme resolves each token. Theme layering: programme base theme → target theme → mode variant (dark/light/high-contrast). On native targets the OS selects the mode; the programme supplies the values for every mode (§3) |
| Asset substitution | Three distinct mechanisms, never conflated: theme asset table (token assets), override-map asset substitution (content assets), #5853 asset presentations (element presentations). Substitution changes the asset, never the semantics attached to it (§4) |
| Raw HTML/CSS | Always supported (parent rule). Raw CSS is the highest layer *on the web target only*; on GUI/native targets raw CSS is not translated — it is ignored with a build-time warning, and raw HTML degrades per the parent's fallback rules (§2.3) |
| Conflict reporting | Deterministic, build-time: same-specificity override ties are errors (#5853 §2.5); raw CSS overriding an assertion-derived property is a *reported* override (lint listing), never silent; an assertion with no mapping on a target warns and falls back to that layer's default (§6) |
| Cross-target leakage | None: themes, override maps, asset tables, and conditional predicates are target-local. Nothing declared for one target changes another target's rendering (§2.4) |

## 1. What is already fixed (not re-decided here)

1. **Parent #5679:** two layers (raw HTML/CSS always supported; semantic
   markup describes intent, carries its accessibility role by
   construction); styling as semantic assertions over named roles/tokens;
   one content/semantic source per programme, per-target page copies
   explicitly forbidden; conditional blocks never carry essential
   content; native targets follow host-OS behaviour conventions.
2. **Sibling #5853:** a presentation override lives in the target's
   override map, binds by element instance or kind+variant+state,
   covers every interaction state or fails at build time, always falls
   back to the standard rendering, and never carries essential content.
   Its §2.5 specificity order *inside* the override layer (instance >
   kind+variant+state > kind default; ties are build-time errors) is
   adopted unchanged by this record.
3. Nothing in §2–§6 changes a parent or #5853 decision. §8 says so
   explicitly and names the questions that would.

## 2. The cascade

### 2.1 Layers, low to high

For one element on one target, the effective presentation is computed
by applying, in order:

1. **Platform defaults.** The target's standard rendering of the
   semantic element with no styling at all: browser/Tailwind defaults
   on web, the standard control on GUI, the OS-standard control on
   native.
2. **Theme/token values.** The target's theme (§3) supplies the value
   of every named role/token the lower and higher layers reference
   (colour roles, typography roles, spacing/density, asset tokens).
   A theme supplies *values only* — it cannot introduce, remove, or
   rebind an assertion or an override.
3. **Semantic style assertions.** The author's assertions from the one
   semantic source (e.g. "this form's primary action uses the `accent`
   colour role, `primary` emphasis"), rendered in the target's idiom
   (web: Tailwind classes per #5851's mapping; native: OS control
   tinted/honouring roles as far as the platform allows per #5852;
   GUI: the standard rendering, styled). Assertions reference roles,
   never raw values.
4. **Per-target override map.** Two entry kinds, applied in this order:
   (a) *style overrides* — element+assertion → adjusted presentation
   values for this target only; (b) *presentation overrides* (#5853) —
   replace the element's rendering wholesale. When a presentation
   override applies, style assertions and style overrides for that
   element no longer affect its appearance; they still define its
   *fallback* standard rendering (#5853 §2.4), so they are never dead
   configuration. Inside this layer, #5853 §2.5 specificity decides.
5. **Raw CSS (web target only).** Author raw CSS applies last, per
   property: a raw-CSS declaration wins over the assertion-derived
   value for the same property on the web target. This is the parent's
   sketched order ("semantic assertions < per-target overrides < raw
   CSS (web)") adopted as the decision. Raw CSS can only affect
   presentation properties — it cannot change an element's role,
   accessible name, value, state, or interaction contract, on any
   target, ever.

### 2.2 Within-layer order

- Assertions: more specific wins — instance assertion > kind+variant+
  state assertion > kind default assertion; an exact tie between two
  assertions in the source is an authoring error reported at build
  time, never resolved by source order alone. (Source order is *not*
  a cascade input; determinism is.)
- Themes: §3's layering (base → target → mode variant).
- Override map: #5853 §2.5, unchanged.
- Raw CSS (web): the web platform's own CSS cascade applies *inside*
  this layer only — including source-order resolution of
  equal-specificity declarations, which §9 invariant 2 expressly
  exempts; it does not reach back into layers 1–4.

### 2.3 Raw HTML/CSS off the web target

- Raw CSS is never silently translated to GUI/native idioms. On
  non-web targets it is ignored, and the build reports each ignored
  rule once (file/rule, per §6).
- Raw HTML follows the parent's mixing rules; where a raw-HTML
  construct has no GUI/native rendering it degrades to the
  author-supplied fallback content or the target's webview embedding,
  per element class — the per-class table belongs to #5851/#5852; this
  record fixes only that raw HTML *never* outranks the semantic
  layer's accessibility contract.

### 2.4 Target locality

Every layer above platform defaults is declared per target and applies
only there. There is no cross-target inheritance of themes, overrides,
asset tables, or conditional outcomes: a target with no declarations
renders layers 1–3 only (platform defaults, a default theme, the
author's assertions). Adding a declaration for one target is provably
a no-op for every other target — this is a conformance-test property
(#5855), not a hope.

## 3. Themes

- A **theme** is a per-target table from token names (colour roles,
  typography roles, spacing/density tokens, asset tokens, state-style
  roles) to values in the target's idiom. Assertions and overrides
  reference token names only; a raw value (hex, px, font name) in an
  assertion is a build-time error.
- **Layering:** programme base theme → per-target theme → mode
  variant. The mode variant (dark / light / high-contrast) is a
  partial table: it overrides only the tokens it names.
- **Who selects the mode:** on native targets, the OS (the parent's
  platform-wins-for-behaviour rule) — the programme must supply values
  for every mode it ships, and a missing mode value falls back to the
  target theme's value with a build-time warning. On web/GUI targets,
  the user's OS/browser preference selects the mode when expressed;
  the theme's declared default applies otherwise. An author may not
  force a mode against an expressed user/OS preference.
- A theme may substitute the whole **asset pack** for token assets
  (§4) without touching any binding — assets are content-addressed,
  themes reference packs by name.

## 4. Asset substitution

Three mechanisms, deliberately distinct:

1. **Theme asset table.** Token assets — logos, icon sets, decorative
   imagery referenced by role — resolve through the target theme's
   asset table (§3). Same role, per-target asset.
2. **Content-asset substitution.** An override-map entry binds a
   content asset (by semantic-source asset reference, or by asset
   role + instance) to a per-target replacement asset. The replacement
   declares intrinsic size/aspect so layout reserves the right box.
   The accessible name/description and any caption stay in the
   semantic source and are unchanged by substitution.
3. **Presentation assets.** Image/sprite assets that *are* an
   element's presentation are #5853 asset presentations — not asset
   substitution, and governed by #5853's state-completeness and
   fallback rules, not this section.

Rules for all three: substitution changes the asset bytes, never the
semantics attached to the asset; an asset carrying essential content
(baked-in label/value/instruction text) is forbidden (#5853 invariant
5, extended to content assets); a missing asset at build time is an
error for mechanisms 1–2 (no silent blank), and #5853's fallback rule
governs mechanism 3.

## 5. Conditional blocks

- A conditional block includes content for a subset of targets. It is
  evaluated **before** the cascade: the semantic source a target
  renders is the source minus the blocks excluded for it; included
  content then composes through §2 exactly like unconditional content.
- **Predicates** are target facts only: target name, form-factor
  class, or a declared target capability flag. No arbitrary
  expressions, no user/state data, no nesting of conditional blocks
  in v1 — evaluation stays total, deterministic, and lintable.
- **Essential-content rule (parent, restated as an invariant):** a
  block may carry embellishment only. Removing every conditional
  block must leave a complete, usable programme on every target;
  conformance checks this by rendering the stripped source. Content
  that fails this test does not belong in a conditional block.
- An unknown predicate term is a build-time error (never "treat as
  false"): a typo'd capability flag must not silently drop content.

## 6. Conflict reporting and tooling hooks

- Build-time, deterministic, per target: (a) same-specificity ties in
  assertions or the override map — **error**; (b) raw CSS overriding
  an assertion-derived property (web) — **reported override** in the
  build's composition report (allowed, but never silent); (c) an
  assertion or token with no mapping/value on a target — **warning**,
  rendering falls back to the nearest lower layer that defines it;
  (d) ignored raw CSS / degraded raw HTML off-web — **warning**, once
  per rule/construct; (e) unused override entries and unreferenced
  theme tokens — **warning**.
- The composition report (which layer won, per element/property, per
  target) is a build artefact; #5855's conformance suite consumes it.
  This record fixes the report's obligation, not its format.

## 7. Unresolved sibling dependencies (named, not guessed)

- **#5849 (semantic markup vocabulary v1, OPEN):** owns the element
  kind set and instance naming. §2's binding/specificity shapes assume
  kinds + named instances exist; if #5849 changes the naming shape,
  the key syntax changes, not the cascade order.
- **#5850 (style assertion vocabulary, OPEN):** owns the role/token
  vocabulary itself. This record composes whatever vocabulary v1
  defines; it coins no new roles.
- **#5851 (web rendering, OPEN):** owns the assertion → Tailwind
  mapping table and the raw-HTML per-class degradation table for web.
  §2 orders the layers; it does not pre-empt those tables.
- **#5852 (native rendering, OPEN):** owns assertion → OS-control
  mapping and per-platform conflict detail. §2.1 layer 3 states the
  composition-level rule the parent sketched (platform wins for
  behaviour; the assertion wins for identity/brand colour within what
  the platform allows); if #5852 refines per-platform behaviour, the
  cascade in §2 is unaffected because native has no presentation
  overrides and its mapping lives entirely inside layer 3.
- **#5855 (rendering pipeline & conformance, OPEN):** owns
  enforcement: the target-locality property (§2.4), the
  stripped-source completeness check (§5), and the composition
  report's format (§6) are requirements handed to it.

## 8. Worked example — the parent's three-target form

One semantic source (sketch, using parent vocabulary):

```
form "settings":
  field name        (text)
  field email       (email)
  action save       [primary, assertion: colour-role = accent]
  conditional(target = web): hint paragraph with a documentation link
```

- **(a) Web / Tailwind.** Layer 1: browser form defaults. Layer 2: the
  web theme resolves `accent` → the programme's brand green (Tailwind
  palette value). Layer 3: the `primary`+`accent` assertion renders,
  via #5851's mapping, as the button's Tailwind classes (accent
  background, primary emphasis). Layer 4: no override entries — the
  standard rendering stands. Layer 5: the author's raw CSS adds
  `letter-spacing` to the button; it wins for that property and the
  build's composition report lists it as a reported override (§6b).
  The conditional hint renders (target = web).
- **(b) Custom GUI.** Layers 1–3 as above, with the GUI theme
  resolving `accent` to the GUI palette. Layer 4: a #5853 asset
  presentation override binds `save` to a per-state image-button asset
  pack; the button renders as the custom image button for every
  state, with role, name ("Save"), focus order, and activation
  identical to the standard button, and the layer-3 rendering as its
  declared fallback. Layer 5: absent — raw CSS is ignored on this
  target, with §6d warnings. The conditional hint is excluded; the
  form remains complete without it (§5).
- **(c) Native (Mac/Windows/GNOME).** Layers 1–3: the OS-standard
  button; the platform owns shape, focus, keyboard, and mode
  (dark/high-contrast selected by the OS, values supplied by the
  native theme per §3); the `accent` role is honoured as the control
  tint as far as the platform allows — identity colour from the
  assertion, behaviour from the platform. Layer 4: presentation
  overrides do not apply to native standard controls (#5853 §4.6);
  a style override could adjust a token value for this target only.
  Layer 5: raw CSS ignored with warnings. Same source, same form,
  three native-feeling renderings — and at no point was the page
  copied per target.

## 9. Invariants implementation must hold

1. **One source.** Themes, override maps, asset tables, and
   conditional blocks are per-target side declarations; the semantic
   source is identical for all targets and names no target.
2. **Cascade determinism.** §2's layer order plus within-layer
   specificity is total: every element/property/target has exactly
   one winning declaration or a reported error. Within layers 1–4
   source order is never a tiebreaker. Layer 5 is exempt: inside raw
   CSS the web platform's own cascade (§2.2) applies, including its
   source-order resolution of equal-specificity declarations — that
   resolution is internal to layer 5 and never reorders the layers
   themselves.
3. **Roles, not values.** Assertions and overrides reference named
   roles/tokens only; raw values live only in themes (per target)
   and in web raw CSS (layer 5).
4. **Semantics are layer-proof.** No layer — theme, assertion,
   override, raw CSS — changes role, accessible name, value, state,
   focus order, or the interaction contract of an element. (Extends
   #5853 invariant 1 from overrides to the whole cascade.)
5. **Presentation-override precedence is bounded.** A #5853
   presentation override supersedes layers 3–4(a) for appearance
   only; layers 1–2 and the fallback standard rendering remain as
   #5853 specifies.
6. **Target locality.** §2.4: a declaration for one target is a no-op
   for all others, verified by conformance tests.
7. **No essential content in side channels.** Conditional blocks,
   substituted assets, and presentation assets carry no essential
   content (§4, §5; #5853 invariant 5).
8. **Mode respect.** An expressed user/OS mode preference (dark,
   high-contrast) is never overridden by a theme or an author
   declaration.
9. **Nothing silent.** Every tie, fallback, ignored rule, and
   raw-CSS override is reported at build time (§6); a rendering that
   differs from the author's source by an unreported decision is a
   conformance failure.

## 10. Alternatives considered and rejected

1. **Raw CSS wins everywhere, including translated to native/GUI.**
   Rejected: translation of arbitrary CSS to OS controls is
   undefined, breaks the parent's platform-wins-for-behaviour rule,
   and makes the web the secret master target.
2. **Assertions outrank override maps.** Rejected: it makes the
   per-target customisation this ticket exists to provide impossible
   without editing the shared source — i.e. it reintroduces
   per-target source forks under another name.
3. **Cross-target theme/override inheritance** (declare once, inherit
   everywhere, override per target). Rejected for v1: inheritance
   across targets with different idioms makes §2.4's locality
   property untestable and hides which target a change affects;
   per-target declaration is verbose but auditable. (Revisiting this
   needs usage evidence, not taste.)
4. **Conditional blocks as a style layer** (target-conditional
   assertions inside the cascade). Rejected: two mechanisms doing
   one job; inclusion is a source-level fact, styling is a cascade
   fact, and mixing them makes the stripped-source completeness
   check (§5) meaningless.
5. **Arbitrary predicate expressions / nested conditionals.**
   Rejected for v1: determinism and lintability (§5) beat
   expressiveness until a concrete programme needs more.
6. **CSS-style global cascade with escape hatches (`!important`
   analogues).** Rejected: the failure mode of the web cascade —
   precedence by escalation — is what a fixed layer order exists to
   prevent. The one sanctioned escape is layer 5 itself, on web,
   reported (§6b).

## 11. Relation to the parent — no parent decision changed

This record changes **no** decision in issue #5679 and none in the
#5853 record. It adopts the parent's sketched cascade order as the
decision (§2.1), reuses #5853's binding and specificity shapes, and
leaves each sibling's owned tables (§7) to that sibling. Two future
questions are named so they are not decided by accident: (a)
cross-target inheritance of themes/overrides (§10.3) and (b)
custom-drawn controls on native targets (#5853 §5) — either would
need its own decision record, and (b) would additionally need Drew's
ruling as a change to the parent's native rule.

## 12. Splits and follow-ons

The design does not split further: cascade, themes, assets, and
conditionals are one composition mechanism with one determinism
contract, and splitting them would duplicate §9. Implementation is
**not** filed by this record — issue #5854 notes implementation
timing is a separate decision; when that decision is taken, the
implementation ticket should cite §2–§6 as its shape and §9 as its
acceptance oracle, be linked from issue #5679's decomposition item 6,
and coordinate with #5855 for the conformance properties named in
§2.4, §5, and §6.
