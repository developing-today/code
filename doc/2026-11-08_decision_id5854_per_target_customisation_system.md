# Decision: per-target customisation — themes, override maps, asset substitution, conditional blocks, and the cross-layer cascade

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5854](https://github.com/developing-today/code/issues/5854),
  "id: design: per-target customisation system (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites — semantic markup,
  style-assertion model"), decomposition item 6 ("Per-target customisation
  system: themes, override maps, asset substitution, conditional blocks —
  with the worked three-target example"). Programme: #2056.
- **Landed sibling taken as given:** [#5853](https://github.com/developing-today/code/issues/5853),
  `doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`. Its §5
  settles custom GUI presentation overrides and explicitly leaves "the
  cascade between style assertions, target override maps, and raw CSS" to
  this follow-on. That cascade is decided here (§4).
- **Evidence baseline:** `origin/main` at `5c1a6b2c0b`, worktree
  `~/git/fleet/13783`, 2026-11-08. Verified at that baseline, consistent
  with triage task 12346 (2026-11-08): issue #5854 is OPEN with no
  hard-stop labels and no PR/commit resolving its remaining scope;
  `git ls-tree` over `doc/` on origin/main finds a decision record for
  #5853 only — none for #5849, #5850, #5851, #5852, #5854, or #5855.

## Decision summary

| Question | Decision |
| --- | --- |
| How customisation composes | One fixed, deterministic **pipeline per target** (§2): conditional inclusion → content-asset substitution → presentation selection → property cascade → platform adaptation. A later stage never re-opens an earlier stage's question. |
| The cross-layer cascade (the question #5853 §5 left open) | Two separate decisions, never conflated (§4): **presentation selection** is decided only by the override map (per #5853 §2.5 — assertions and raw CSS cannot change which presentation renders); **property values** within the standard rendering cascade, lowest to highest: platform/UA default < base theme < style assertions < per-target theme delta < override-map property entries < raw CSS (web target only). |
| Themes | One **base theme** per program (token → value) plus a **sparse per-target delta** per target (§3). A delta names only the tokens it changes; a full per-target theme copy is a fork, and tooling treats it as one (§3, invariant 8). |
| Override maps | Per-target, sparse, keyed exactly as #5853 §2.1 keys presentation entries. Two entry kinds share one resolution rule (§5): **property entries** (assertion/property → target value) and **presentation entries** (#5853). Specificity: instance > kind+variant+state > kind; ties at the winning specificity are a build-time error. Override maps are **never inherited across targets**. |
| Asset substitution | The semantic source references **content assets by role**, never by path (§6). A per-target substitution map may replace the asset bound to a role; the accessible name/alt stays with the source. A missing substituted asset falls back to the base asset with a build warning — an element never renders as nothing (the #5853 fallback principle applied to content assets). |
| Conditional blocks | A **closed condition vocabulary** (target class, OS family, form factor, named capability) evaluated at build time wherever the target is known (§7). Blocks may carry **embellishment only** — never essential content, and never a second copy of essential content. A block whose condition is false is simply absent; absence needs no fallback precisely because nothing essential may live there. |
| Platform-owned properties | On native targets, **behaviour properties** (focus behaviour, keyboard operation, shape conventions, high-contrast response) are platform-owned and are removed from the cascade entirely (§4.3); **identity properties** (accent/brand colour roles, typography identity) cascade normally and reach the platform as tint/identity inputs. The per-platform detail of that split is #5852's to record; the boundary — behaviour out, identity in — is fixed here. |
| Raw HTML/CSS | Contained (§4.4): raw CSS is the highest property layer **on the web target only**, applies to standard-rendered elements only, and can never change presentation selection, semantics, or another target's rendering. Raw HTML keeps the parent's always-supported status; nothing here gates it. |
| Conflict reporting | Deterministic and build-time (§4.5): specificity ties, duplicate bindings, and unregistered references are errors; unused override/substitution entries and unmapped assertions are warnings. No conflict is ever resolved by file order or by silently picking a winner. |

## 1. What the parent and #5853 already fixed (not re-decided here)

Issue #5679 fixes, and this record takes as given:

1. **Two layers.** Raw HTML/CSS is always supported and never gated by the
   semantic layer; semantic markup describes intent and structure and
   carries its accessibility role by construction on every target.
2. **Style assertions, not presentation values.** Styling is expressed as
   assertions over named roles/tokens ("this form is green" — a colour
   *role*, not a hex value); each target renders an assertion in its own
   idiom (Tailwind classes on web, OS-standard controls on native, and —
   per #5853 — wholly custom presentations on custom-rendered targets).
3. **One content/semantic source.** Targets customise via theme/token
   files, override maps, asset substitution, and conditional blocks that
   never carry essential content. Copying the whole page per target is
   explicitly forbidden.
4. **Native behaviour is the platform's.** Controls on native targets
   follow host-OS conventions for behaviour; §4.3 below only draws the
   cascade boundary that follows from this, it does not revisit it.

Issue #5853 fixes, and this record builds on without revisiting:
presentation overrides live in the target's override map, bind by
element instance or kind+variant+state, come in exactly two kinds
(asset, drawn), must be state-complete, and always declare the standard
rendering as fallback. Its within-map specificity rule (§2.5 there) is
adopted unchanged and extended to property entries (§5 here).

Nothing in §2–§8 changes a parent or #5853 decision. §11 says so
explicitly and names the one boundary this record deliberately leaves
to a sibling (#5852's per-platform conflict detail).

## 2. The composition pipeline

Customisation is not a bag of independent mechanisms that happen to
apply in some order; it is one pipeline, run once per target artefact,
in exactly this order:

1. **Conditional inclusion** (§7) decides which embellishment blocks
   exist for this target. Output: the source, plus this target's
   embellishments, with essential content untouched.
2. **Content-asset substitution** (§6) resolves every asset *role* in
   the result to a concrete asset for this target.
3. **Presentation selection** (§4.1, #5853) decides, per element,
   whether the standard rendering or a presentation override renders.
4. **Property cascade** (§4.2) resolves every style property of every
   standard-rendered element (and the layout box of every overridden
   element) to one value.
5. **Platform adaptation** (§4.3) applies the target platform's owned
   behaviour and variant selection (dark / light / high-contrast on
   native is the OS's choice, per the parent).

The order is the decision. Each stage consumes only the previous
stage's output, so: a conditional block cannot smuggle in a styling
rule that outranks the cascade (its contents are styled by stages
3–5 like everything else); asset substitution cannot change which
presentation renders (stage 2 resolves content assets; presentation
assets belong to stage 3 and to #5853's packs); and platform
adaptation cannot resurrect a property the cascade resolved away —
it owns only the properties §4.3 assigns to it.

**Determinism.** Every stage is a total function of (source, target
artefact inputs): same inputs, same artefact, on any machine. Any
input that would make a stage non-deterministic (a specificity tie, a
duplicate role binding, an unregistered renderer or asset role) is a
build-time error, never a coin flip (§4.5).

**Target locality.** The pipeline runs per target with that target's
inputs only. Adding or changing one target's theme delta, override
map, substitution map, or conditional blocks can never change another
target's artefact. This is #5853's target-locality invariant generalised
to all four mechanisms.

## 3. Themes

- A **theme** is a named map from style *tokens* (the vocabulary itself
  is #5850's; this record treats token names as opaque) to values.
- A program has exactly **one base theme**. Each target may carry one
  **theme delta**: a sparse map naming only the tokens whose values
  differ on that target. Token resolution for a target is
  `delta[token] ?? base[token]`; a token defined in neither is a
  build-time error (an unmapped assertion is a warning instead — see
  §4.5 — because an assertion may legitimately have no value on a
  target that renders it by platform default).
- **Variant dimensions** (dark / light / high-contrast) are theme
  *dimensions*, not separate themes: a theme may give a token one
  value per variant. Who selects the active variant is target-class
  business — the OS on native targets (parent rule), author or OS
  media on web, the target in custom-rendered artefacts — never a
  conditional block or a per-element choice.
- Theme selection is a **build/deploy-time input** to the pipeline,
  not author content: the semantic source never names a theme, so the
  same source can be re-themed for a new target without an edit.
- A delta that redefines (nearly) every token is not a delta; it is a
  forked theme wearing a delta's clothes. Tooling warns when a delta
  crosses that line (threshold is #5855's tooling detail), because a
  forked theme has the same maintenance shape as the per-target page
  copy the parent forbids.

## 4. The cascade

The parent sketched "semantic assertions < per-target overrides < raw
CSS (web) — or a different order; specify it". The specification is
below. Its load-bearing move is to refuse the question as posed in
one part: "which presentation renders" and "what are its property
values" are **different decisions with different winners**, and any
single ladder that tries to answer both is what produced the open
question in the first place.

### 4.1 Presentation selection (no ladder)

Which presentation renders for an element is decided **only** by the
override map's presentation entries, resolved by #5853 §2.5
specificity (instance > kind+variant+state > kind default; ties are
errors). Style assertions do not vote: an assertion is input to the
standard rendering and to a presentation's declared derivations, not
a competing presentation claim. Raw CSS does not vote: on the web
target it styles the standard rendering, and where a presentation
override has substituted an asset or drawn presentation there is no
standard rendering for it to style (the override's layout box is
still a property-cascade matter, §4.2). This is what makes #5853's
fallback meaningful: fallback is exercised by the renderer's
structural rule (§2.4 there), never by a CSS rule outbidding the
override.

### 4.2 Property cascade (the ladder)

For one property of one standard-rendered element on one target,
exactly one value wins, from the highest layer that sets it:

1. **Platform / UA default** — the target's standard rendering default.
2. **Base theme** — the token value from the program's base theme.
3. **Style assertions** — the author's assertions on the element
   (assertion → token → value). Assertions outrank the base theme
   because they are the author's per-element intent; the theme is
   the program-wide default they refine.
4. **Per-target theme delta** — the target's token value (§3). The
   delta outranks assertions *as values*: an assertion names a role,
   and the delta is how that role is realised on this target, which
   is precisely the parent's "each target renders the assertion in
   its own idiom". An author who needs a value the delta cannot
   bend uses layer 5 or 6, deliberately and locally.
5. **Override-map property entries** (§5) — element-targeted property
   values for this target, resolved by the §5 specificity rule.
6. **Raw CSS** — **web target only**, author-written CSS applying to
   the rendered element. Highest because the parent guarantees raw
   CSS as the always-available escape hatch; containment is in §4.4.

Two consequences are deliberate and worth stating. First, the ladder
is **per property, not per element**: different properties of one
element routinely come from different layers, and that is normal, not
a conflict — a conflict exists only when two entries in the *same*
layer claim the same property of the same element at the same
specificity (§4.5). Second, **layer 6 does not exist on GUI/native
targets** and layers are never "made up" there by another mechanism;
a property nobody sets on those targets is the platform default,
by design.

### 4.3 Platform-owned properties (native targets)

On native targets, a property is one of two kinds, and the kind —
not the ladder — decides:

- **Behaviour properties** — focus behaviour and visibility rules,
  keyboard operation, control shape conventions, high-contrast and
  reduced-motion response: **platform-owned**. They are removed from
  the cascade: assertions, deltas, and override entries that name
  them on a native target are build-time warnings ("platform-owned;
  ignored on this target"), never silent no-ops and never errors —
  the same assertion may be legitimate on another target.
- **Identity properties** — accent/brand colour roles, typography
  identity, density identity where the platform exposes it: cascade
  normally through §4.2 and reach the platform as the tint/identity
  inputs it accepts. Where the platform accepts no such input, the
  value is dropped with the same warning, per the parent's
  assertion-coverage model.

This *is* the parent's unresolved "platform wins for behaviour;
assertion wins for identity/brand colour — or the reverse" question,
answered: **both, each in its own domain**. What remains for #5852
is the per-platform enumeration (which properties Mac / Windows /
GNOME own, and which identity inputs each accepts) — detail that
must be written against the platforms, not against this contract.
If #5852 finds a property that is neither, it names it here by
amendment rather than quietly re-laddering it.

### 4.4 Raw HTML/CSS containment

Raw CSS being the top property layer on web is safe only with its
boundaries stated, and they are part of the decision:

- Raw CSS applies to the **web artefact only**. It is not an input to
  any other target's pipeline, and no mechanism translates it.
- It styles **standard-rendered elements**. It cannot select, remove,
  or restyle a presentation override (§4.1), and it cannot change an
  element's role, accessible name, or state (the semantic layer owns
  those by construction, on web exactly as elsewhere).
- Raw HTML keeps the parent's status — always valid, never gated —
  and degrades on non-web targets by the parent's own rules, which
  are not this record's to change. Raw HTML in a conditional block
  is still raw HTML: §7 decides whether the block exists, §4 decides
  how its contents render.

### 4.5 Conflict reporting

| Condition | Verdict |
| --- | --- |
| Two entries in one layer, same property/element, same specificity | **Build error**, both entries named (§5) |
| Override or substitution entry matches no element/role | Warning (unused entry) |
| Assertion has no mapping/value on a target | Warning (platform default will be used) — the parent's coverage warning |
| Assertion/delta/entry names a platform-owned property on a native target | Warning (ignored on this target), §4.3 |
| Token referenced but defined in neither base theme nor delta | **Build error** |
| Substitution or presentation references an unregistered asset/renderer | **Build error** (#5853 §2.2 rule, applied to both maps) |
| Registered asset/renderer exists but fails to load at build time | Fallback exercised + warning (#5853 §2.4; §6 here) |

No row in this table is resolved by source order, file order, or
recency. That is the whole point of having the table.

## 5. Override maps

- One override map **per target**, sparse: an absent entry means
  "standard rendering, cascaded properties", never "inherit another
  target's entry". There is no cross-target inheritance and no
  program-level override map; a change meant for every target is
  made in the base theme or the assertions, where the parent put
  program-wide intent.
- Two entry kinds share the map and the binding key of #5853 §2.1
  (element instance, or element kind + variant/emphasis role +
  state set):
  - **Property entries** set one or more properties for the bound
    element(s) at cascade layer 5.
  - **Presentation entries** are #5853's overrides, unchanged.
- **Resolution.** For one element on one target, property entries
  resolve per property by specificity — instance > kind+variant+state
  > kind default — and presentation entries resolve per #5853 §2.5.
  The two resolutions are independent: an element can take its
  presentation from one entry and a surviving property (its layout
  box, §4.1) from another.
- **Conflicts.** Two entries of the same kind tying at the winning
  specificity for the same element (and, for property entries, the
  same property) are a build-time error naming both. Unused entries
  warn. These are #5853 §2.5's rules stated for the whole map, so
  the map has one conflict discipline, not two.

## 6. Asset substitution

- The semantic source references **content assets by role**
  (`asset: brand-mark`, `asset: icon/save`), never by file path, so
  substitution never requires a source edit. (Presentation assets —
  a button's image states — are not content assets; they belong to
  #5853's override packs and follow its rules, including native
  exclusion.)
- A per-target **substitution map** binds a role to a concrete asset
  for that target: a different format (vector on web, raster set on
  GUI), a locale or theme variant, a higher-density rendering. One
  binding per role per target; a second binding is the §4.5
  duplicate-binding error.
- The **accessible name / alt text stays with the source**. A
  substituted asset inherits it unchanged; an asset that would need
  different alt text is a different role, not a substitution. This
  is invariant 5 of #5853 (no essential content in presentations)
  stated for content assets: substitution changes the rendering of
  the content, never the content.
- **Constraints and fallback.** A role may declare format/size/aspect
  constraints; a substituted asset violating them is a build error.
  A substituted asset that is missing or fails to load falls back to
  the base asset with a build warning — the same structural fallback
  as #5853 §2.4, because "renders as nothing" is never an acceptable
  outcome for content either. A role with no base asset and no
  binding on a target is a build error (the role itself is
  unregistered there).
- Native targets **do** receive content-asset substitution (a
  platform-appropriate icon is still the program's content asset) —
  the opposite of presentation overrides, and deliberately so:
  #5853's native exclusion is about *controls*, not content.

## 7. Conditional blocks

- A conditional block wraps a fragment of the source in a condition
  over a **closed vocabulary**: `target:` (web / gui / native),
  `os:` (mac / windows / gnome / …), `form-factor:` (compact /
  regular / …), `capability:` (a named capability from the target's
  declared capability set). Author-invented condition strings are a
  build error; a new condition kind is a vocabulary amendment, not
  a local improvisation. (The vocabulary's exact spelling is #5849's
  authoring-syntax business; the closedness is this record's.)
- Conditions are evaluated at **build time** wherever the target
  artefact fixes them (target, OS family); form-factor and
  capability conditions that genuinely vary at runtime evaluate at
  runtime against the target's declared values — the block's
  *contents* are still fixed at build time.
- **Embellishment only.** A block may contain decorative or
  presentational content, target-specific chrome that
  duplicates essential guidance available elsewhere in the source,
  and assets. It may never contain the only copy of anything:
  not the only label, instruction, action, or data. The conformance
  oracle is mechanical: delete every conditional block from the
  source and the program must lose nothing essential — every action
  still reachable, every field still labelled, every datum still
  present. (This is the parent's rule; the deletion test is how it
  is checked, and it belongs to #5855's suite.)
- **No duplicate-essential blocks.** Writing the same essential
  content once per target inside three blocks is the forbidden
  per-target page copy with extra steps, and is rejected by the
  same deletion test (the content is essential, therefore it may
  not live in a block at all).
- A false condition needs **no fallback**: the block is absent, and
  by the rule above nothing essential is absent with it.

## 8. Worked example — the parent's three-target form

One source, walked through the pipeline for all three targets. The
source is a preferences form with an email field, a primary submit
action carrying an `accent` assertion, a brand-mark content asset,
and two embellishment blocks. (Element and assertion spellings are
illustrative; the vocabularies are #5849's and #5850's.)

```text
<form id="prefs" title="Preferences">
  <field name="summary-email" kind="email" label="Email for summaries"/>
  <action id="save-button" kind="submit" variant="primary"
          assert="role:accent; density:comfortable">Save preferences</action>
  <media asset="brand-mark" alt="Northwind mark"/>
  <when target="web"> <decor asset="hero-strip"/> </when>
  <when target="gui"> <decor asset="mascot-animated"/> </when>
</form>
```

Program inputs:

- **Base theme:** `accent → #0B6E4F`, `surface → #FFFFFF`,
  `density:comfortable → 8pt grid` (values illustrative).
- **Web delta:** `accent →` the web theme's emerald-700 equivalent.
  **GUI delta:** none for `accent` (the GUI button is not coloured by
  the cascade — see below). **Native delta:** `accent →` the OS
  accent/tint input where the platform accepts one.
- **Override maps:** web — a property entry binding
  `action + primary` → `radius-role: large`. GUI — a presentation
  entry binding the `save-button` instance to the "brushed" asset
  pack (per-state assets, #5853 §2.2). Native — empty.
- **Substitution maps:** web — `brand-mark → brand.svg`;
  GUI — `brand-mark → brand@2x.png`; native — `brand-mark →` the
  platform icon asset.
- **Raw CSS (web artefact only):** one author rule setting
  `letter-spacing` on the form's rendered actions.

Resolution, target by target:

- **(a) Web / Tailwind HTML.** Stage 1 keeps the web block (hero
  strip), drops the GUI block. Stage 2 binds `brand-mark` to the
  SVG. Stage 3: no presentation entry for web — the button is
  standard-rendered. Stage 4 for the button's colour: platform
  default < base theme `#0B6E4F` < the `accent` assertion (selects
  the role) < **web delta** (the role's web value wins) — rendered
  as the corresponding Tailwind classes per #5851's mapping; its
  radius comes from the layer-5 property entry; its letter-spacing
  from layer-6 raw CSS. Each property has exactly one winner, and
  the raw CSS — although the "strongest" layer present — could not
  have recoloured the button past the delta or replaced it with an
  image: layers answer different questions (§4.1–4.2).
- **(b) Custom GUI app.** Stage 1 keeps the mascot block. Stage 2
  binds the PNG brand-mark. Stage 3: the presentation entry wins
  for `save-button` — it renders as the brushed asset pack's image
  button, state-complete per #5853 §2.3, standard rendering as
  fallback. Stage 4 still runs: it resolves the button's **layout
  box** (from the assertion's density role and the pack's declared
  intrinsic size, #5853 invariant 8), but its surface colour
  properties have no standard rendering to land on and are moot —
  not overridden, *unapplied*, which is why the GUI delta needs no
  `accent` entry. Accessibility tree, focus order, and activation
  are the standard button's (stage 5 and #5853 invariant 1).
- **(c) Native app (Mac / Windows / GNOME).** Stage 1 drops both
  blocks — and loses nothing essential, by §7's deletion test: the
  field, its label, the action, and the brand-mark remain. Stage 2
  binds the platform icon. Stage 3: presentation entries do not
  apply to native standard controls (#5853). Stage 4/5: the button
  is the OS-standard control; its shape, focus, and keyboard
  behaviour are platform-owned (§4.3, removed from the cascade);
  the `accent` assertion survives as the identity input — the OS
  tint where the platform accepts one — and the raw CSS rule from
  the web artefact simply does not exist here. Same source, three
  correct renderings, zero per-target copies.

## 9. Invariants implementation must hold

1. **One source.** The semantic source is identical for all targets
   and contains no theme names, asset paths, or presentation
   references — only asset *roles*, assertions, and conditional
   blocks.
2. **Pipeline order.** §2's stage order is fixed; a later stage
   never re-opens an earlier stage's decision.
3. **Determinism and totality.** Every stage resolves every input or
   fails the build naming why (§4.5). No resolution depends on file
   order, map order, or recency.
4. **Target locality.** One target's delta, maps, and blocks never
   change another target's artefact.
5. **Presentation/property separation.** Presentation selection is
   override-map-only (§4.1); property values come only from the
   §4.2 ladder. Neither mechanism may do the other's job.
6. **Accessibility invariance.** Theme deltas, override maps,
   substitutions, and conditional blocks never change any element's
   role, accessible name, value, state, or focus order.
7. **No essential content at the edges.** Essential content lives
   only in the unconditional semantic source — never only in a
   conditional block, a substituted asset, or a presentation
   (§6, §7; #5853 invariant 5).
8. **Sparseness.** Per-target inputs are deltas: theme deltas,
   override maps, and substitution maps name only what differs.
   A per-target input that redefines everything is a fork and is
   reported as one (§3).
9. **Structural fallback.** Every substitution and every
   presentation has a reachable fallback (base asset / standard
   rendering); nothing ever renders as nothing (§6; #5853 §2.4).

## 10. Alternatives considered and rejected

1. **One global cascade ladder for everything** (assertions <
   overrides < raw CSS, applied to presentation and properties
   alike). Rejected: it makes raw CSS able to outbid a presentation
   override and assertions able to veto one, which dissolves
   #5853's state-completeness and fallback guarantees exactly where
   they matter. Splitting selection from properties (§4.1–4.2)
   keeps both guarantees and still gives raw CSS its parent-promised
   place — at the top of the property ladder on web.
2. **Assertions outrank per-target theme deltas.** Rejected: an
   assertion names a role, and realising that role per target is
   the delta's entire job (the parent's "each target renders the
   assertion in its own idiom"). If assertions carried values that
   beat deltas, the delta would be dead code and per-target
   realisation would move into per-element source — a fork of the
   styling, if not of the page.
3. **Full theme per target** (no base/delta split). Rejected: it is
   the per-target page copy applied to themes — every token change
   must be made N times, and targets silently drift. The sparse
   delta (§3) keeps one definition per token and one named
   difference per target.
4. **Cross-target override inheritance** (a GUI override applies to
   native unless excluded, etc.). Rejected: it violates target
   locality, and #5853 already decided native targets do not take
   presentation overrides at all. Shared intent belongs in the base
   theme and assertions, not in inheritance between targets.
5. **Conditional blocks carrying essential content, once per
   target.** Rejected: that is per-target page copy in disguise;
   §7's deletion test exists to make the rejection mechanical
   rather than a matter of taste.
6. **Asset substitution by path rewriting in the source** (build
   scripts that swap file paths per target). Rejected: the source
   would name paths it is forbidden to name (invariant 1), alt text
   would travel with files instead of content, and a missing file
   would surface at runtime. Role-based binding (§6) keeps every
   failure at build time.
7. **Runtime theme/override switching as author content** (the
   program picks its theme in markup). Rejected for v1: theme and
   target inputs are build/deploy-time (§3), which is what makes
   the pipeline deterministic and conformance-testable per target
   artefact (#5855's model). Runtime *variant* selection (dark /
   high-contrast) is unaffected — it is a theme dimension, owned
   per §3, not a theme switch.

## 11. Dependencies, and relation to the parent

**No parent decision changed.** The cascade in §4 specifies an order
the parent explicitly left open ("or a different order; specify
it"); §4.3's behaviour/identity split answers the parent's platform-
conflict question in the direction the parent sketched, and leaves
its per-platform enumeration to #5852 as that ticket intends.
#5853's override contract is adopted, not amended: §5 extends its
binding and conflict rules to property entries in the same map, and
§4.1 keeps its presentation decisions outside the property ladder
precisely so nothing there is re-decided.

**Unresolved sibling dependencies — named, not guessed:**

- **#5849 (semantic markup vocabulary v1) — OPEN.** Binding keys,
  element kinds, instance names, and the conditional-block syntax
  assume its vocabulary. If #5849 names things differently, the
  keys in §5–§7 rename mechanically; no decision here depends on
  the spellings.
- **#5850 (style assertion vocabulary) — OPEN.** Token and assertion
  names are treated as opaque throughout. The cascade (§4) is over
  *roles*, so #5850 may size the vocabulary freely; if it splits a
  role this record treats as one (e.g. accent into accent/brand),
  §4.3's identity class gains a member, nothing more.
- **#5851 (web rendering: assertion → Tailwind) — OPEN.** It owns
  the web mapping mechanics and must *implement* §4.2's ladder and
  §4.4's containment on web, not re-decide them; where its mapping
  work finds a property the ladder cannot place, it amends this
  record rather than inventing a web-only order.
- **#5852 (native rendering: assertion → OS controls) — OPEN.** It
  owns the per-platform enumeration §4.3 defers: which properties
  each platform owns, and which identity inputs each accepts.
- **#5855 (rendering pipeline & conformance suite) — OPEN.** It
  owns the tooling oracles for this record's invariants: the §7
  deletion test, the §4.5 warning/error table, delta-sparseness
  reporting (§3), and per-target expected-output fixtures —
  including §8's worked example as a fixture in all three
  renderings.

## 12. Splits and follow-ons

The design does not split further: themes, override maps, asset
substitution, conditional blocks, and the cascade are one contract
(the §2 pipeline), and splitting them would reproduce exactly the
unowned seams — between assertions, overrides, and raw CSS — that
#5853 §5 handed to this record. Implementation is **not** filed by
this record — issue #5854 itself notes implementation timing is a
separate decision; when that decision is taken, implementation
tickets should cite §2 and §4 as their shape and §9 as their
acceptance oracle, be linked from issue #5679's decomposition
item 6, and land in step with the #5855 conformance work that can
check them.
