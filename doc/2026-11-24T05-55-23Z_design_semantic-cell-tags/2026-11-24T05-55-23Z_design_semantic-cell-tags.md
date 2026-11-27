# Semantic cell/tag presentation model — one tagged state, many renderers

Date: 2026-11-24 05:55 UTC
Status: design record; no implementation authorized
Source issue: [developing-today/code#6010](https://github.com/developing-today/code/issues/6010)
Program: worlds design program [#5955](https://github.com/developing-today/code/issues/5955); wave-2 follow-up of the Lane B prior-art research (item 37); parent topic [#5970](https://github.com/developing-today/code/issues/5970) (asset/filter model + fallback chain); tiers in [#786](https://github.com/developing-today/code/issues/786)
Tracking: [developing-today/code#13873](https://github.com/developing-today/code/issues/13873) (queue packet 21668, slice 1)
Supersedes: none. This record upgrades the #6010 ask to the 9-point sufficient-design standard. It does not replace #5970 or #786.

## 1. Problem and context

The worlds program needs one world state that renders correctly on clients of wildly different capability: full GPU-shader clients, sprite clients, and ASCII terminals. Today the presentation contract is split across issues — #786 covers capability tiers with in-band terminal detection, #5970 covers user-supplied assets/filters with per-world gating — but nothing specifies what a cell _carries_ so that every tier can render it completely. Without a tagged-state spec, each renderer invents its own mapping, the "one tagged state" promise in #5970's fallback chain has nothing to reference, and the Dwarf Fortress rule (simulation fixed, presentation mapping per tier, ASCII tier always complete and playable) is unenforceable.

Evidence is bounded: the #6010 body states the DF rule, the scope (tag vocabulary, composition with presentation-is-data and #786/#5970, tag authorship), and the done criterion (a tagged-state spec #5970's fallback chain can reference). The #5970 body adds Drew's requirements: user assets by ID, composable named filters on asset parts, per-world gating with visible policy, authored (not improvised) graceful fallback, terminal degradation to DF-style sprites or a representative character. No measurements, prototypes, or renderer inventories are claimed here; the executing slice must inventory the shipped Screen cell buffer and actual client tiers before specifying field widths and vocabulary.

## 2. Goals and non-goals

### Goals

- Specify the cell: fields, tag vocabulary v1, and per-tier rendering obligations, at a level #5970's fallback chain can normatively reference.
- Pin authorship: world code declares tags; per-tier fallback mappings live in the client/presentation layer, never in world code.
- Preserve the shipped invariants: presentation-is-data (legacy UTF-8 text or versioned Screen cell buffer; guests never supply escapes/HTML/CSS) and the #786 capability tiers.
- Keep the ASCII tier complete and playable by construction, per the DF rule.

### Non-goals

- No asset/filter model changes: assets by ID, filter composition, per-world gating, and the fallback-chain declaration belong to #5970; this record only defines what the chain's input (the tagged cell) guarantees.
- No capability-tier renegotiation: in-band terminal detection and tier definitions stay in #786.
- No renderer implementation, no vocabulary v2, no GPU-shader interface, no conversion-tool design.
- No production data, migration, or consumer cutover is authorized by this design task.

## 3. Alternatives considered

| Alternative                                                                                            | Benefits                                                                                                | Costs / risks                                                                                                                                                       | Decision                                                                  |
| ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Do nothing; let each renderer map world state ad hoc                                                   | Zero spec work; renderers ship fast.                                                                    | Mapping drift per client; ASCII tier silently incomplete; #5970's fallback chain references an undefined input.                                                     | Rejected as the program answer; kept as the status quo until slices land. |
| Tagged cells (chosen): world state as a grid of typed cells with semantic tags; per-tier render tables | One source of truth; DF rule enforceable; ASCII completeness testable; #5970 chain has a defined input. | Vocabulary governance cost; tag bloat risk; per-tier tables must be authored and tested.                                                                            | **Chosen.** Vocabulary kept minimal (v1) with an explicit extension rule. |
| Rich-first with server-side downsampling (server renders every tier)                                   | Clients trivial; pixel-perfect control.                                                                 | Server pays for every tier on every frame; terminal clients still need semantic behavior (input mapping), not just pixels; violates presentation-is-data placement. | Rejected.                                                                 |
| Capability negotiation per cell (client requests detail level per cell)                                | Minimal bandwidth.                                                                                      | Round-trip per cell; stateful negotiation; enormous complexity for DF-style grids.                                                                                  | Rejected.                                                                 |

## 4. Chosen approach

### 4.1 The cell

A cell carries: `kind` (terrain/creature/item/effect/empty or the v1 closed set the spec slice enumerates), `identity` (owner-visible identity reference — an ID, never PII beyond what the world's visibility rules already expose), `state flags` (v1: a small fixed bitmask — e.g. selected/highlighted/blocking/animated — enumerated by the spec slice after inventorying the shipped Screen buffer), and `tags` (semantic annotations: creature/material/state such as `cape`, `sparkle`, `blue`). Tags are namespaced (`world.*` for world-declared, `sys.*` for the reserved core set) so worlds cannot collide with future core tags.

### 4.2 Vocabulary v1 and extension rule

Vocabulary v1 is the smallest set that keeps the ASCII tier complete for existing worlds: the spec slice derives it from the shipped Screen cell buffer plus the #5970 fallback examples (cape, sparkle, color). Adding a tag after v1 requires: a name, the per-tier fallback for every tier (including ASCII glyph/emoji), and the world(s) that declare it. A tag without a complete fallback row is rejected — this is what makes "authored, not improvised" mechanical.

### 4.3 Per-tier rendering obligations

Each tier declares a total function from (kind, flags, tags) to its output: rich client (sprites/shaders per #5970 asset model), sprite-sheet tier, static-substitute tier, glyph/emoji tier (the "blue letter C / cape emoji" rule: color + representative character). Tiers are ordered rich → glyph; a client renders at its tier using only the tagged cell, never world code. The fallback mapping tables live in the client/presentation layer.

### 4.4 Composition boundaries

- With #786: tier detection/selection stays in #786; this record defines what each selected tier must honor.
- With #5970: assets/filters attach at the rich tiers and above the tagged cell; when a world disables a filter, the authored static substitute (a tag-level fallback row) applies — "static sparkles on the GIF" is a fallback row, not a special case.
- With presentation-is-data: the wire form remains the versioned Screen cell buffer (or legacy UTF-8 text); tags ride in the buffer's extension fields, never as escapes/HTML/CSS from guests.

## 5. Dependency-ordered implementation slices

Each slice is sized for one lane/session. Proposed executable slices under #6010; this design enqueues none of them.

| Order | Slice and dependency                                                                                                                                                                                                                        | Acceptance criteria                                                                                                                                    |
| ----: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
|     1 | **Inventory.** No data dependency. Inventory the shipped Screen cell buffer (fields, widths, extension points), the #786 tier definitions, and the #5970 fallback-chain draft; enumerate current worlds' presentation needs.                | Checked-in inventory: buffer schema, tier list, per-world tag needs; v1 vocabulary candidates with ASCII-fallback sketches.                            |
|     2 | **Tagged-state spec v1.** Depends on slice 1. Writes the cell schema, closed kind set, flag bitmask, `world.*`/`sys.*` namespaces, vocabulary v1 with a complete per-tier fallback row per tag, and the extension rule.                     | Spec merged as a dated doc/ record; #5970's fallback chain references it; a reviewer proves every v1 tag has a total per-tier mapping including ASCII. |
|     3 | **Conformance tests.** Depends on slice 2. Property tests: every v1 tag renders at every tier (ASCII completeness = playable grid, no missing-glyph cells); unknown tags fall back to kind+flags rendering; guest-supplied markup rejected. | Tests green in CI; unknown-tag and markup-rejection cases covered.                                                                                     |
|     4 | **Pilot renderer tables.** Depends on slices 2–3. Implements the per-tier tables for two tiers (glyph tier + one rich tier) against one pilot world.                                                                                        | Pilot world playable end-to-end at both tiers; fallback rows exercised (filter-disabled path shows the authored substitute).                           |

## 6. Rollout and rollback

- Rollout: inventory → spec → tests → pilot tables. No live world, buffer format, or client changes until the spec slice is reviewed; the spec is additive (extension fields in the versioned buffer) so old clients keep working.
- Rollback: the spec is a document (revert the PR); pilot tables are per-world/per-tier additive code (revert the PR). No migration, no data change, no consumer cutover at any stage.

## 7. Verification plan

1. **Fidelity oracle:** every #6010 body requirement (tag vocabulary scope, composition with presentation-is-data/#786/#5970, world-declares-tags authorship, #5970-referenceable spec) traces to §4 or a slice; nothing dropped.
2. **Totality oracle:** machine-checked — each v1 tag has a fallback entry for every tier; the ASCII tier renders a complete playable grid for the pilot world.
3. **Boundary oracle:** fallback tables live outside world code (import graph check); guests cannot inject escapes/HTML/CSS (negative tests).
4. **Scope oracle:** no #5970/#786 behavior changed; full-repo CI status reported separately, never implied from focused tests.

## 8. Open questions and decisions

1. **Vocabulary v1 contents and field widths — decider: Drew (worlds owner), with the spec-slice lane proposing.** **Default:** derive from the shipped Screen buffer + #5970 examples; nothing wider than the buffer's extension capacity without Drew's sign-off.
2. **State-flag bitmask contents — decider: Drew.** **Default:** smallest set covering current worlds' needs per the slice-1 inventory.
3. **Whether `sys.*` core tags need any v1 entries beyond color — decider: Drew.** **Default:** color only; everything else is `world.*` until proven shared.
4. **Pilot world choice — decider: executing lane, Drew accountable.** **Default:** the smallest world exercising creature/material/state tags.

## 9. Record, tracking, and acceptance

This dated record is the slice-1 design artifact for #6010, delivered under queue packet 21668 (tracking #13873). It links from #6010 for discoverability; it closes nothing and claims no inventory, spec, test, or renderer fact — those belong to the slices.

**Design acceptance:**

- [x] Problem and evidence, goals/non-goals, honest alternatives including do-nothing, chosen contracts and capability boundary.
- [x] Dependency-ordered, lane-sized slices with individual acceptance criteria.
- [x] Rollout, rollback/recovery limits, verification oracles, and named-decider open questions with defaults.
- [x] Date-stamped `doc/` record tied to #6010 and #13873 queue packet 21668; explicitly distinguishes design evidence from slice evidence.
- [ ] After merge, add this document's permanent GitHub link to #6010 and verify the issue comment and document.

### Evidence references

- [#6010 source issue](https://github.com/developing-today/code/issues/6010)
- [#5970 parent topic (asset/filter model + fallback chain)](https://github.com/developing-today/code/issues/5970)
- [#5955 worlds design program](https://github.com/developing-today/code/issues/5955)
- [#786 presentation tiers](https://github.com/developing-today/code/issues/786)
- [#13873 Lane-6 tracking issue](https://github.com/developing-today/code/issues/13873)
