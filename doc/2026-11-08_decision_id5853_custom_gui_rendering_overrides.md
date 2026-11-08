# Decision: custom GUI rendering as first-class overrides

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5853](https://github.com/developing-today/code/issues/5853),
  "id: design: custom GUI rendering as first-class overrides (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites — semantic markup,
  style-assertion model"), decomposition item 5 ("Custom GUI rendering:
  wholly custom presentations (image buttons, drawn controls) as
  first-class overrides"). Programme: [#2056](https://github.com/developing-today/code/issues/2056).
- **Evidence baseline:** `origin/main` at `6f1efc4424`, worktree
  `~/git/fleet/13254`, 2026-11-08. Verified at that baseline, consistent
  with triage task 12345 (2026-11-08): `git grep` over `pkgs/id` for
  "first-class override" / "custom GUI rendering" / "custom presentation"
  finds no design or decision record, and issue #5853's comments carry
  only worker pickup/park notes — no recorded decision. This record is
  the first decision for this follow-on.

## Decision summary

| Question | Decision |
| --- | --- |
| Where a custom presentation lives | In the **target's override map**, never in the semantic source: a named, versioned **presentation override** bound to a semantic element (§2) |
| What it binds to | A binding key: a specific element instance, or an element kind + variant/emphasis role + state set — the same key shape the parent's override maps already use (§2.1) |
| Presentation kinds | Two, only: **asset presentation** (image / sprite / themed asset pack, with per-state assets) and **drawn presentation** (a target-side renderer registered in the override pack that implements the element's interaction contract) (§2.2) |
| Semantics | The override changes **presentation only**. Role, accessible name, value, state, focus order, and the activation/event contract come from the semantic element and are identical with and without the override (§3) |
| States | An override must cover **every** interaction state of the element (default, hover, focus, active, disabled, error where applicable), by an explicit per-state asset/renderer or a declared derivation; a partial override is a build-time error, not a runtime gap (§2.3) |
| Fallback | Every override declares the **standard rendering as its fallback**; a missing asset or renderer falls back and warns at build time — an element never renders as nothing (§2.4) |
| Essential content | **Never in the asset.** Labels, values, and text live in the semantic layer; text baked into an image is decorative only (§3, invariant 5) |
| Which targets | Custom-rendered targets (the GUI target, and the web target where the author declares it). **Native targets do not apply presentation overrides** to standard controls — they keep OS-standard controls per the parent; only content assets (icons, images that *are* content) carry over (§4) |
| Conflict order | Instance binding beats kind+variant+state binding beats kind default; two bindings at the same specificity for one target are a build-time conflict error, never a silent pick (§2.5) |

## 1. What the parent already fixed (not re-decided here)

Issue #5679 fixes, and this record takes as given:

1. **Two layers.** Raw HTML/CSS is always supported and is never gated by
   the semantic layer; semantic markup describes *intent and structure*,
   not presentation, and carries its accessibility role by construction
   on every target.
2. **Style assertions, not presentation values.** Styling is expressed
   as semantic assertions over named roles/tokens; each target renders
   an assertion in its own idiom (Tailwind on web, OS-standard controls
   on native), and the parent explicitly names the GUI case this record
   settles: "the same assertion may render as a custom image button /
   fully custom-drawn control — the design must allow a target to
   substitute a wholly custom presentation … for a semantic element +
   style assertion."
3. **One content/semantic source.** Targets customise via theme/token
   files, override maps (element + assertion → target presentation),
   asset substitution, and conditional blocks that never carry essential
   content. Copying the whole page per target is explicitly forbidden.
4. **Native behaviour is the platform's.** On native targets the control
   follows host-OS conventions for behaviour (shape, focus, keyboard,
   high-contrast, dark mode); a custom presentation may not quietly
   re-implement — and break — platform behaviour there.

Nothing in §2–§4 changes a parent decision. §5 says so explicitly, and
names the one question that *would* change a parent decision if it is
ever wanted (native custom controls) and is therefore left out.

## 2. The chosen shape: presentation overrides in the override map

### 2.1 Binding

A presentation override is an entry in a target's override map:

```
override:
  target: gui                # one target only; never inherited across targets
  bind:
    element: button          # semantic element kind …
    instance: save-button    # … or one named instance (exactly one of kind/instance forms per entry)
    variant: primary         # emphasis/variant role, optional
    states: [default, hover, focus, active, disabled]
  presentation: { … }        # §2.2
  fallback: standard         # §2.4 — the only permitted value in v1
```

The semantic source does not name the override, the asset, or the
renderer. The same source therefore renders as a standard control on a
target with no override entry, an image button on the GUI target, and
an OS-standard button on native — which is the parent's worked-example
shape, with the middle rendering now specified instead of sketched.

### 2.2 The two presentation kinds

- **Asset presentation.** A referenced asset (or asset pack) plus, for
  each covered state, the asset or derivation for that state, plus the
  intrinsic size/aspect the layout system must reserve. Assets are
  content-addressed by the override pack so a theme can substitute the
  whole pack without touching bindings.
- **Drawn presentation.** A renderer **registered by name in the target's
  override pack** — target-side code, never code carried in the content
  source — that draws the element inside its declared layout box and
  implements the element's full interaction contract (§3). Registration
  is the gate: an unregistered renderer name is a build-time error.

There is no third kind in v1. In particular there is no "inline custom
markup" kind (§4, alternative 1).

### 2.3 State completeness

A button that only skins its default state is the hack this issue
exists to replace. An override entry lists the states it covers; the
set must equal the element kind's full state set, each state satisfied
by an explicit asset/renderer behaviour or a declared derivation (for
example "disabled = default asset at the disabled token's opacity").
Anything less fails the build with the missing states named, in the
same spirit as the parent's assertion-coverage warnings — except that
here it is an error, because a half-skinned interactive control is a
functional defect, not a cosmetic one.

### 2.4 Fallback is structural

`fallback: standard` is not a courtesy field: the renderer must be able
to produce the standard rendering for every overridden element, the
build warns whenever a fallback is exercised (missing asset, failed
renderer registration), and conformance tests (§3) run every overridden
element in both modes. An override with no possible standard rendering
is not a valid override.

### 2.5 Resolution and conflicts

For one element on one target, at most one override applies: instance
bindings beat kind+variant bindings, which beat kind defaults. Two
entries tying at the winning specificity are a build-time conflict
error listing both. Unused entries (no matching element in the source)
warn. Resolution is deterministic and target-local: adding an override
for one target can never change another target's rendering.

## 3. Invariants implementation must hold

1. **Semantic identity.** An override never changes the element's role,
   accessible name, value, or state. The accessibility tree of a source
   is byte-for-byte the same with all overrides applied, with none
   applied, and in fallback mode.
2. **Interaction contract.** Activation, keyboard operation, focus
   visibility, and focus order are the standard element's. A drawn
   presentation implements the contract; it does not redefine it.
3. **State completeness.** §2.3 holds for every entry, checked at build
   time — the oracle is the element kind's declared state set.
4. **Fallback existence.** §2.4 holds for every entry; the standard
   rendering is always reachable.
5. **No essential content in presentations.** No label, value, or
   instruction exists only inside an asset or a drawn rendering. (This
   is the parent's "conditional blocks never carry essential content"
   rule applied to presentations, and it is what keeps localisation
   and accessibility possible.)
6. **One source.** Overrides live only in per-target override maps /
   packs. The semantic source is identical for all targets and contains
   no presentation references.
7. **Target locality.** An override applies to exactly the target that
   declares it. Native targets ignore presentation overrides for
   standard controls (§4).
8. **Layout honesty.** An override declares its intrinsic size/aspect;
   it may not change the element's layout semantics beyond that
   declared box, and overflow behaviour is the standard element's.
9. **Determinism.** §2.5's resolution is total and deterministic;
   conflicts and unused entries are reported, never silently resolved.

Conformance follows the parent's model: the per-target expected-output
suite gains, for each overridden element in its fixtures, the
accessibility-tree and event-sequence expectations of the *standard*
element — the invariants above are the oracle those tests check.

## 4. Alternatives considered and rejected

1. **Inline custom markup in the semantic source** (an element carries
   its own custom presentation). Rejected: it forks the content per
   presentation, breaks the parent's one-source rule, and makes the
   standard rendering — and with it the fallback — disappear exactly
   where the presentation is most custom.
2. **Raw HTML/CSS as the only escape hatch.** Rejected: the parent
   keeps raw HTML/CSS as a supported layer, but as the *only* route to
   an image button it is web-only, has no GUI/native analogue, and is
   precisely "a hack around the standard renderings" that issue #5853
   is filed to end.
3. **Per-target page copies.** Rejected: explicitly forbidden by the
   parent; overrides exist so this never becomes necessary.
4. **Arbitrary per-element renderer callbacks carried in the content
   source.** Rejected: content-carried code breaks the parent's
   markup-only non-goal, is untestable by the conformance suite, and
   cannot be checked for the §3 invariants. Drawn presentations are
   therefore target-side and registered (§2.2), which keeps the power
   and restores the checks.
5. **Skinning only** (CSS-like properties over the standard control).
   Rejected as insufficient, not as wrong: token-level styling is the
   parent's style-assertion layer and stays; it cannot express a
   wholly drawn control or a state-ful image button, which is the
   whole subject of this ticket.
6. **Applying presentation overrides on native targets too.** Rejected
   for v1: it contradicts the parent's native rule (platform wins for
   behaviour) and would re-implement focus, keyboard, and
   high-contrast behaviour per program. See §5.

## 5. Relation to the parent — no parent decision changed

This record changes **no** decision in issue #5679. The binding key
reuses the parent's override-map shape; the native exclusion (§2,
§4.6) *follows* the parent's native rule rather than revisiting it;
the cascade the parent left open is touched only inside the override
layer (specificity among override entries, §2.5), not between
assertions, overrides, and raw CSS — that ordering remains the
per-target customisation follow-on's question.

One future question is named so it is not decided by accident:
**custom-drawn controls on native targets** would change the parent's
platform-wins-for-behaviour decision and needs its own decision record
superseding that part of #5679, with Drew's ruling — it is not granted
by this record.

## 6. Splits and follow-ons

The design does not split further: the two presentation kinds are one
mechanism with one binding, state, and fallback contract, and splitting
them would duplicate that contract. Implementation is **not** filed by
this record — issue #5853 itself notes implementation timing is a
separate decision; when that decision is taken, the implementation
ticket should cite §2 as its shape and §3 as its acceptance oracle,
and be linked from issue #5679's decomposition item 5.
