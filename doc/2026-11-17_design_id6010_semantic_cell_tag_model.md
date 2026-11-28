# Design: semantic cell/tag presentation model — one tagged state, many renderers

- Date: 2026-11-17
- Status: design decision for [#6010](https://github.com/developing-today/code/issues/6010)
- Baseline: `origin/main` at `9899c4b1a5e9438262d8604ab0c8270feb735b11`
- Related: [worlds program #5955](https://github.com/developing-today/code/issues/5955), [presentation assets #5970](https://github.com/developing-today/code/issues/5970) (umbrella decision record, 2026-11-09, task 13561), [presentation capability tiers #786](https://github.com/developing-today/code/issues/786) (`doc/2026-11-01_design_id786.md`), [terminal rendering ladder #6011](https://github.com/developing-today/code/issues/6011), [visual-feature cost ranking #6012](https://github.com/developing-today/code/issues/6012), [shader asset format #6013](https://github.com/developing-today/code/issues/6013)
- Scope: the tagged-state spec only — cell fields, tag vocabulary v1, per-tier rendering obligations, ownership. This record does not authorize implementation and designs no other issue's slice.

## Decision

World state is projected as a grid of **tagged cells**. A cell carries exactly the semantics every renderer needs — what the thing _is_ (kind), which thing it is (owner-visible identity), what it is made of and shaped like (material/form tags), and what is true of it right now (state flags) — and each presentation tier renders those tags at its own fidelity. The simulation and the projection never change per tier; only the presentation mapping does. This is the Dwarf Fortress rule from #6010: ASCII glyphs → community tilesets replacing glyphs 1:1 → full graphical tiles, one simulation throughout, and the lowest tier remains a complete, playable representation.

The model extends the shipped cell, it does not replace it. Today's `ScreenCell` (`pkgs/id/crates/id-core/src/world_presentation.rs`) already carries a glyph-tier projection (`symbol`, `fg`, `bg`, `bold`, `columns`) plus one semantic hook, `token: Option<StyleToken>` — "semantic intent retained for adapters, not guest-supplied markup". The tagged model generalises that hook into the full field set below, following the precedents the shipped code already set: adapters resolve every token/tag, guest documents reject unknown values, and unknown _future_ values fall back rather than failing.

## 1. Cell fields

A tagged cell is the shipped `ScreenCell` plus four semantic fields. The existing fields are retained unchanged and are reclassified as the cell's **authored glyph projection** — the world author's own statement of how this cell reads at the glyph tier.

| Field                                   | Type                                | Meaning                                                                                                                                                                                                                                                                                                        |
| --------------------------------------- | ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kind`                                  | one kind tag (§2)                   | The cell's primary semantic class. Exactly one per cell; never absent (`empty` is a kind).                                                                                                                                                                                                                     |
| `identity`                              | owner-visible reference or null     | Which thing this is, when the cell is a thing: an entity reference the cell's owner may see, or an asset reference `id@revision` in #5970's model. Null for terrain, fluids, and empty cells. Never an internal/secret ID — if the viewer may not know the identity, the cell carries the kind/form tags only. |
| `tags`                                  | bounded set of vocabulary tags (§2) | Material, form, colour, and effect semantics: what it is made of, what shape/part it is, its semantic colour, its visible effects.                                                                                                                                                                             |
| `flags`                                 | bounded set of state flags (§2)     | Transient truth about the cell right now: damaged, burning, hostile, selected, occluded, and the rest of the flag family. Flags change without changing identity.                                                                                                                                              |
| `symbol`, `fg`, `bg`, `bold`, `columns` | as shipped                          | The authored glyph projection. A renderer at the glyph tier may use it directly; richer tiers treat it as the final fallback, never as the source of semantics.                                                                                                                                                |
| `token`                                 | `Option<StyleToken>`, as shipped    | Text-semantic intent (emphasis, warning, link, …) for text-bearing cells. Unchanged; it is a _text_ semantic and does not merge into the tag vocabulary.                                                                                                                                                       |

Rules:

- **Semantics flow from the fields, never from the glyph.** A renderer must not infer kind, material, or state by reverse-reading `symbol`/`fg` (a blue `C` does not prove "cape"). Reverse inference is ambiguous the moment two kinds share a glyph, which the fallback rules below deliberately allow.
- **Tags are data, never markup.** Tag strings are validated against the vocabulary version the surface declares; they are never interpreted as escapes, HTML, CSS, format names, or code. This keeps the shipped presentation-is-data boundary intact: guests supply values from a closed vocabulary, adapters own all rendering.
- **Bounds follow the shipped surface.** Cells stay within the existing `MAX_SCREEN_*` bounds; a cell carries at most 8 tags and 8 flags in v1, each a short snake_case string. Validation rejects an out-of-vocabulary tag in a guest document, exactly as guest documents reject unknown `StyleToken`s today; an _adapter_ meeting a tag from a newer vocabulary version drops that tag and renders the rest of the cell — a version skew degrades one cell's fidelity, never the frame.
- **Diff parity.** Tags and flags are part of cell equality: a tag-only or flag-only change produces a `ScreenPatch`, and the existing diff/apply parity property must hold over tagged cells unchanged.

## 2. Tag vocabulary v1

The vocabulary is versioned with the surface. Families are closed in v1 except where marked open; coining a new _standard_ tag is a vocabulary version bump, not an ad-hoc guest invention. A world may coin world-local tags in the reserved `world/` family; renderers treat them as unknown (they fall back per §3), so a world-local tag can enrich a cooperating client but can never break a stock one.

**Kind (exactly one per cell):** `empty`, `terrain`, `fluid`, `vegetation`, `structure`, `creature`, `item`, `effect`, `marker`.

**Material:** `stone`, `wood`, `metal`, `cloth`, `leather`, `glass`, `organic`, `bone`, `paper`, `energy`.

**Form / part:** `wall`, `floor`, `door`, `stairs`, `container`, `weapon`, `tool`, `cape`, `garment`, `furniture`, `corpse`, `pile`.

**Colour (semantic, not a palette index):** `red`, `orange`, `yellow`, `green`, `blue`, `purple`, `white`, `black`, `grey`, `brown`. Semantic colour is what the _thing_ is; `fg` is how the glyph tier happens to paint it. The colour→palette mapping is a client table (§4), so a colour-blind-safe or monochrome client can remap without world code changing.

**Effect:** `sparkles`, `glowing`, `burning`, `frozen`, `wet`, `smoke`, `shimmer`, `corrupted`.

**State flags:** `damaged`, `destroyed`, `open`, `locked`, `lit`, `occluded`, `hidden`, `selected`, `hostile`, `friendly`, `neutral`, `owned_by_viewer`, `interactive`.

**World-local (open):** `world/<name>` — renderer-opaque, fallback-treated.

Vocabulary discipline: a tag names something a _renderer at some tier_ can act on (a glyph, a tint, a tile key, an asset part, a text word). A tag no tier can render is program metadata and does not belong in the cell. Gameplay truth that must not leak (an unseen trap's nature, a disguised creature's real kind) is never tagged: the world projects the _apparent_ cell, and the apparent cell is the only cell.

## 3. Per-tier rendering obligations

The tiers are #786's client tiers (Tier 1 enhanced, Tier 2 ANSI cell grid, Tier 3 line mode) crossed with #5970's authored fallback rungs (rich → static substitute → sprite → glyph). The obligations below bind every tier; §5970's chain governs _user-asset_ references, this section governs _cells_, and where they meet (a cell whose identity is an asset reference) both apply: the cell's chain is the asset's chain, entered at the tier the client occupies.

**Completeness (all tiers).** Every cell renders at every tier. No cell may be dropped, blanked, or replaced with a generic placeholder because a tag is unmapped, an asset is missing, or a filter is denied. The fallback tables in this section are **total over the kind family**: every kind has a glyph, a tile key, and a text name, so the last rung always exists — the same guarantee #5970 requires of its glyph rung.

**Precedence (when one output slot carries many tags).** A glyph, tile, or text slot can show only so much; what survives is decided in this order, highest first: (1) kind, (2) identity/form, (3) semantic colour, (4) state flags that change meaning (`hostile`, `burning`, `selected`), (5) material, (6) effects. Effects are always the first thing dropped and never displace identity: sparkles decorate a cape, they never become the cape.

**Glyph tier** (Tier 2 baseline; #5970 rung 4). Resolve in order: (1) the cell's authored glyph projection (`symbol`/`fg`/`bg`/`bold`), if present; (2) the client fallback table keyed by form tag, then kind tag; (3) colour tag → `fg` via the client's colour table; (4) meaning-changing flags → `bold`/token emphasis per the client table. A client-invented `?` for an unmapped _kind_ is forbidden — the kind table is total, so it cannot occur; an unmapped form/material/effect tag simply falls through to the kind glyph.

**Tileset tier** (Tier 1 sprite rendering; #5970 rung 3). Tiles replace glyphs 1:1, keyed by `(kind, form, material)` in that specificity order, world tileset mapping first, client default set second. A missing tile key falls back to the glyph-tier rendering of the same cell — never to a blank or a neighbouring tile. Colour tags tint where the tileset declares a tintable part (the asset part model of #5970); an untintable tile renders untinted rather than wrong-tinted.

**Rich tier** (Tier 1 enhanced; #5970 rungs 1–2). A cell with an asset-reference identity renders the asset with its named composed filters under #5970's rules (world policy first, cost rank per #6012, capability per #786). Denial, missing capability, or decode failure steps down the authored chain — static substitute, then tileset, then glyph — deterministically, and the selected rung is recorded in the presentation diagnostic (#5970 §4) so "why am I seeing a letter C" is answerable per cell.

**Line mode** (Tier 3). The shipped invariant already binds every surface to provide a deterministic, escape-free text projection; tagged cells strengthen it: a cell projects as its short text name (identity name if the viewer may see it, else the kind/form name), its semantic colour word, and any meaning-changing flags as words — effects and material may drop. `a blue cape`, `a hostile creature`, `burning terrain`. A flag that changes what the cell _does_ to the viewer never drops at any tier, line mode included.

**Worked example (the rule from #6010).** "Blue cape with sparkles" is one cell: `kind=item`, `identity=<asset ref>`, `tags=[cape, cloth, blue, sparkles]`, `flags=[]`. Rich tier: the cape asset with the `sparkle` filter composed. Static rung: the asset author's pre-rendered sparkled sprite. Tileset: the cape tile, tinted blue. Glyph: a blue letter `C` — form `cape`→`C`, colour `blue`→blue `fg`; `sparkles` drops by precedence and the drop is in the diagnostic. Line mode: `a blue cape`. Every tier is the same cell; nothing about the world changed.

## 4. Tag assignment ownership

- **World code declares tags.** The guest/world projection is the only authority on what a thing is: it assigns `kind`, `identity`, `tags`, and `flags` when it projects state into cells. If a tag is wrong, the bug is in world code, and no renderer may "correct" it by inference.
- **The per-tier fallback mapping is a client/presentation-layer table.** Tag→glyph, tag→tile-key, colour→palette, flag→emphasis: these live with the adapters, are versioned with the client, and are shared by every world the client renders. World code never ships renderer tables, and a renderer never reaches back into world state to decide what a cell means.
- **World authors may override presentation data, not presentation code.** Per #5970 rung 4, a world author may override glyphs (themed glyphs) and supply tileset mappings and static substitutes. Those overrides are _data in the world's presentation policy_, consumed by the client's table machinery — the world still authors no rendering logic.
- **Asset authors own the rich rungs of their assets** (asset record, part map, static substitute) under #5970 §2–§4; the client's glyph table is the floor under all of it.
- **Vocabulary governance** sits with the #5955 program: new standard tags are proposed against the "some tier can render it" test above and land as a vocabulary version bump with the fallback tables updated in the same change — a tag without a total fallback mapping does not ship.

## 5. Composition with what is already shipped and decided

- **Presentation-is-data (shipped).** Legacy UTF-8 text (`text/1`) is unchanged and remains the floor surface. The versioned Screen cell buffer gains the semantic fields as a versioned extension of the cell (or, equivalently at implementation time, as coalesced tag spans parallel to the existing `styles`/`tokens` spans in `ScreenPayload` — the record binds the _semantics and obligations_, the encoding choice is the implementer's within them). Guests still never supply escapes, HTML, or CSS; tags are validated vocabulary values, the furthest thing from markup.
- **#786 tiers.** This record consumes #786's tiers and in-band detection as-is: it says what each tier must render, never how a tier is probed or selected. Tier selection stays local to the TTY adapter; no capability or tag information enters the journal or the wire contract beyond the surface document itself.
- **#5970 asset/filter model.** Asset references, named composable filters, world policy declarations, and the four-rung authored fallback chain are #5970's; this record's cell chain _is_ that chain seen from the cell side, and the two must not drift: a change to rung order or authorship is a #5970 change that this record then re-reads.
- **#6011 / #6012 / #6013.** The terminal ladder (#6011) supplies the terminal rungs the glyph/line obligations render into; cost ranking (#6012) gates rich-tier features by measured rank exactly as it gates #5970's filters; the shader format (#6013) is one rich-tier rendering input and gains no world-state access through cells.

## Rejected alternatives

- **Per-tier world state** (the world emits different state per client tier): triples the projection surface, makes tiers disagree about the world, and is exactly what the DF rule forbids — the simulation never changed.
- **Glyph-only status quo** (keep inferring richness client-side from `symbol`/`fg`): cannot carry identity or material to the rich tier, and inference breaks on the first shared glyph.
- **Free-form unvalidated tags**: becomes markup by accretion, defeats the total-fallback guarantee (nobody can table-map an open string set), and reopens the guests-supply-presentation-code hole the shipped surface closed.
- **Renderer-owned semantics** (clients assign tags by analysing the world feed): the renderer has no authority on what a thing is, and two clients would tag the same cell differently — rung selection must be deterministic for equal capability (#5970 §4).

## Acceptance criteria for implementation

- A fixture set covering every kind tag, the worked example above, and cells exercising each precedence level renders at all tiers; tests assert the _output_ per tier, including that no cell is ever blank or `?`.
- The kind→glyph, `(kind, form, material)`→tile, and colour→palette fallback tables are total by construction (exhaustiveness checked in tests, following the shipped exhaustive `text_projection` precedent).
- Diff/apply parity holds over tagged cells, including tag-only and flag-only changes.
- Unknown-tag handling matches §1: guest documents reject out-of-vocabulary tags; adapters drop newer-vocabulary tags and render the rest of the cell.
- Rung/tag-fallback selection for the worked example is visible in the presentation diagnostic.
- Line-mode projection of a tagged screen is deterministic and escape-free, and meaning-changing flags survive it.

## Evidence and precedence

- **Issue #6010** is the source of the DF rule, the three scope questions, and "Done looks like" (a tagged-state spec #5970's fallback chain can reference). Its triage of record (task 18420, 2026-11-17) verified: no record on `main`, siblings #6011/#6012/#6013 landed, this slice outstanding.
- **#5970 umbrella record (2026-11-09)** owns the asset/filter/fallback-chain decisions this record composes with and names #6010 as the spec its glyph rung depends on.
- **Shipped code, verified at the baseline above**: `WorldSurface` offers legacy UTF-8 text and the versioned Screen buffer; `ScreenCell` is `{symbol, fg, bg, bold, columns, token}`; `StyleToken` is documented as semantic intent for adapters with per-tier projections and a plain fallback for unknown future tokens — the direct precedent for the tag fields and their fallback discipline. (The #786 record's older path `pkgs/id/src/world_presentation.rs` has since moved to `pkgs/id/crates/id-core/src/world_presentation.rs`.)
- **#786 record** owns tiers and detection; where its 2026-11-01 state notes are stale (prerequisite PRs, file paths), the current checkout wins.

<!-- agent-note:v1 agent=repo-worker-code lane=code-d state=design-record task=19943 origin=muse-scheduled-lane -->
