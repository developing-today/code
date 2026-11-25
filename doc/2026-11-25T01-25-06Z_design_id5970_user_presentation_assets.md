# Design: user presentation assets (umbrella) — assets by ID, named filters, authored fallback, world policy

- Date: 2026-11-25
- Status: design decision for [#5970](https://github.com/developing-today/code/issues/5970), landed as a `doc/` record
- Baseline: `origin/main` at `00abab2981ebff1b8c84f1ea151f3af9281bfe0e`
- Source: the umbrella decision record posted on #5970 on 2026-11-09 (queue task 13561). This file carries that decision into the tree, reconciled with the sibling records that have landed since. Where the two differ, this file is current.
- Related: [worlds program #5955](https://github.com/developing-today/code/issues/5955), [tiers #786](https://github.com/developing-today/code/issues/786) (`doc/2026-11-01_design_id786.md`), [semantic cell/tag model #6010](https://github.com/developing-today/code/issues/6010) (`doc/2026-11-17_design_id6010_semantic_cell_tag_model.md`), [terminal ladder #6011](https://github.com/developing-today/code/issues/6011), [cost ranking #6012](https://github.com/developing-today/code/issues/6012) (`doc/2026-11-08_design_id6012_visual_feature_cost_ranking.md`), [shader format #6013](https://github.com/developing-today/code/issues/6013) (`doc/2026-11-08T00-57-08Z_design_shader_asset_format/`), [graphical client tooling #5782](https://github.com/developing-today/code/issues/5782) (`doc/2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md`)
- Scope: the umbrella only. Each sibling owns its slice; this record states the invariants that bind them and the parts no sibling owns (asset-by-ID, named filters, the fallback chain's authorship, world policy, the abuse surface). Design only: no implementation is authorized here.

## Sibling state at the baseline

| Issue | State | What this record takes from it |
|---|---|---|
| #6010 cell/tag model | closed, record on main | The tagged cell, the total kind→glyph table, tier obligations and the precedence order. The glyph rung (§4) is that table. |
| #6011 terminal ladder | closed | The terminal rungs the glyph and line tiers render into. |
| #6012 cost ranking | closed, record on main | Measured rank per device profile, worst-dimension rule, per-world and per-viewer gates. Filters expose measurable inputs to it. |
| #6013 shader format | closed via PR #7019, record on main | Closed code-owned format registry, `shadertoy.glsl.fragment` v1, declared inputs, asset revision separate from format version. |
| #786 tiers | open (implementation in review) | Consumed as-is: tiers and in-band detection. This record never says how a tier is probed. |

The 2026-11-09 record named two blockers before an implementation plan could be filed: #6013's record (PR #7019) and #6010's record. Both have landed.

## 1. Umbrella invariants

1. **Presentation is a projection, never authoritative state.** Assets, filters and shaders change only pixels or cells in a bounded presentation target. They never change world data, identity, hit areas, permissions, chat or outcomes.
2. **Every visual capability is declarable and deniable.** A world declares which asset and filter capabilities it enables; a client declares which it can render. Neither side assumes the other.
3. **Degradation is authored, never improvised.** Every rung a viewer can fall to is declared by an author before it is needed. No client invents a substitute.
4. **Assets are referenced by stable ID; effects are named and composable.** An appearance is asset references plus named filters, not a per-app code path.
5. **Cost and format are contracts, not claims.** Rank comes from measurement (#6012). A format name is valid only for the profile a registered adapter renders (#6013).
6. **A missing capability is a documented outcome, not an error.** Unknown format, denied policy, missing channel or renderer failure selects the next authored rung and keeps a usable presentation.

## 2. Assets by ID

**Decision.** An uploaded asset is stored once and referenced by an immutable `asset-id@revision`. A presentation reference is `{ asset: id@revision, parts?, filters: [name@version, ...] }`.

- **Revisions are immutable.** New artwork is a new revision. An appearance keeps rendering the revision it names, so nothing changes under a viewer by silent replacement. This matches #6013 §3.1, where a shader asset's revision is separate from its format version.
- **Parts are declared metadata** on a revision (named regions or layers, such as `cape`). A filter targets a declared part. Targeting a part the revision does not declare degrades the reference to the next rung; it is never applied to the whole image instead.
- **IDs are per deployment.** The asset service that owns uploads issues them. Whether one world may see another world's asset is an authorization question, not a naming one.
- **Relation to graph object names.** The ID apps programme now gives D3 graph objects self-certifying URIs (`doc/2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme/`). If assets later become D3 objects, they inherit that scheme's rule that naming never grants access and that unauthorised holders cannot tell "absent" from "denied". Until then, asset references stay per-deployment and this record does not adopt global content-addressed asset names: the 2026-11-09 concern that such names leak cross-world existence is answered only by that coalescing rule, which lives in D1 slice 3, not here.

**Rejected.** Filters as per-app hard-coded drawing calls (the failure mode this replaces). Mutable asset slots resolved at render time (appearance changes with no viewer action, abuse response becomes unauditable).

## 3. Named composable filters

**Decision.** A filter is a named, versioned transform with a declared contract, not free-form code.

| Field | Meaning |
|---|---|
| `name@version` | Reference identity; world policy permits names, versions or families. |
| `applies-to` | Declared parts or the whole asset. |
| `capabilities` | What the client must have: `raster`, `gpu-fragment`, `animation`, `blend-mode:<x>`, a minimum #786 tier. A shader-backed filter's capability is its #6013 registry format. |
| `cost` | Measurable inputs for #6012's ranker: target area, channels, passes. |
| `fallback` | The authored rung this filter falls to when unavailable. |
| `authority` | Who authored it: client built-in, world (in world policy), or asset author (shipped with the asset). |

Composition is an ordered list per part, `cape: [sparkle@1, tint@2]`; order is part of the reference. Chain length is bounded per tier so composition cannot become an unbounded per-frame cost. A filter whose capability is absent falls to its authored rung and the presentation diagnostic says so; it never silently no-ops while claiming to be applied.

**Rejected.** A monolithic per-app "effect" string (no capability declaration, so no honest gating). Arbitrary user expressions (that is user code, which belongs to #5955's function trust path).

## 4. The authored fallback chain

Evaluated top-down against world policy, then client capability:

| Rung | What renders | Author |
|---|---|---|
| 1. Rich | Asset plus composed filters, shader-backed where #6013's registry and #6012's rank allow | Asset author |
| 2. Static substitute | A pre-rendered image (for example "static sparkles on the sprite") | Asset author; a world may override with a themed substitute |
| 3. Tileset | The cell's tile, keyed by `(kind, form, material)` per #6010 §3 | World author's tileset mapping, client default set second |
| 4. Glyph | The cell's glyph per #6010's total kind→glyph table, coloured by its semantic colour tag | Client table; a world may override with themed glyphs as data |

Rules:

- The glyph rung always exists, because #6010 makes the kind table total. Presentation never becomes empty, and a client `?` cannot occur.
- Selection is deterministic for equal (world policy, client tier, reference), so two equal viewers see the same thing.
- The selected rung, and each dropped tag or filter, is recorded in the presentation diagnostic (#5782's inspector), so "why do I see a blue letter C" is answerable per cell.
- #6010 is the same chain seen from the cell side. Rung order or authorship changes here are changes #6010 must re-read; #6010's precedence order decides what survives inside one rung.

**Rejected.** Automatic down-conversion of the rich asset (an improvised substitute). A client-invented placeholder (loses identity and hides the missing author's work).

## 5. World presentation policy

**Decision.** Capability enablement is declared by the world and published in its join and preview metadata, visible before joining:

```
presentation:
  assets:   { user-uploads: allow | deny, max-rung: rich | static | tileset | glyph }
  filters:  { allowed: [name@version | family/*], denied: [...], disguise: allow | deny }
  shaders:  { enabled: bool, formats: [shadertoy.glsl.fragment/v1] }
  fallback: { world-glyphs: {...}, world-substitutes: {...}, tileset: <ref> }
```

- User shaders and disguise-class filters are denied by default; a world opts in.
- The policy composes with #5955's permission and registry model rather than adding a second capability system. It is enforced by the presentation adapter, not trusted to client UI.
- Because it is visible before joining, a user can decline a world that will not render their appearance as authored.
- A policy that denies a capability names the rung it falls to; denial is a presentation decision, never a blank.
- The shader `formats` list names #6013 registry entries only. A world cannot name a format the registry does not hold.

**Rejected.** Per-asset allowlists of individual users' uploads (unmanageable, invites exclusion mechanics). Silent per-world client filtering (users cannot predict what they will look like).

## 6. Abuse and code surface

- **Disguise is a gameplay decision.** Shape change, near-invisibility and false name plates are `disguise`-class filters, gated by world policy and denied by default. No filter capability can mean "bypass policy". A world that wants invisibility grants a game ability; the world then projects the apparent cell, which per #6010 §2 is the only cell.
- **User shaders are code, handled as assets.** Per #6013, a shader is a versioned asset run by a trusted adapter through a closed registry, not a #5955 user function, and it reads only #6013's declared inputs. The umbrella rule added here: no shader path may widen the capability or authorization surface of the presentation layer.
- **Uploads are untrusted bytes.** Decode is bounded; a failed decode steps down the chain and is never retried as another format.

**Rejected.** Shaders through the #5955 user-function pipeline (gives a presentation concern world-function reach). Trusting a world's stated policy on the client (policy is enforced where rendering happens).

## 7. Implementation plan

The design is complete and both named blockers have landed, so the implementation is filed as one plan issue, [#14731](https://github.com/developing-today/code/issues/14731), in this order:

1. **Immutable asset references and part maps**: `asset-id@revision` resolution, declared parts, degrade-on-missing-part.
2. **World presentation policy**: the declaration above, adapter-side enforcement, pre-join exposure.
3. **Named filter registry**: declared capabilities, cost inputs to #6012, bounded ordered composition.
4. **Four-rung fallback resolver**: deterministic selection over #6010 tagged cells, diagnostic reporting per cell.

Each slice carries its own tests. Slices 3 and 4 run shader-backed filters only through #6013's adapter boundary, and slice 4 is acceptance-tested with #6010's worked example (the blue sparkled cape at every tier). #786's tier detection is an input, not a slice of this plan.

## Acceptance for this record

- Every "Done looks like" item on #5970 is decided above: asset-by-ID (§2), filters as named composable transforms with capability needs (§3), the authored fallback chain with an author per rung (§4), per-world policy visible before joining (§5), and the shader trust boundary (§6).
- No sibling slice is re-decided; each is consumed by reference.
