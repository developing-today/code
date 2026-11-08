# Design: visual-feature cost ranking

- Date: 2026-11-08
- Status: design decision for [#6012](https://github.com/developing-today/code/issues/6012)
- Baseline: `origin/main` at `10da0c4ed2b59e53950f6dc47af35ea76f05e8ee`
- Related: [worlds program #5955](https://github.com/developing-today/code/issues/5955), [presentation assets #5970](https://github.com/developing-today/code/issues/5970), [shader asset format #6013](https://github.com/developing-today/code/issues/6013), [PR #7019](https://github.com/developing-today/code/pull/7019)
- Scope: rank and gate the cost of sprites, filters, and shaders. This does not design an asset wire format or replace #5970's world policy or fallback chain.

## Decision

Assign each visual feature a **measured performance rank for a target device profile**. A rank is a compact, viewer-legible summary of a cost vector, not an author-selected quality label and not a promise that every scene will have the same frame time. Rank the worst normalized cost dimension rather than averaging unlike resources: a small texture cannot compensate for a shader that exceeds the GPU budget.

An asset's authored declaration supplies bounded inputs and compatibility requirements; it does not supply its rank. The runtime's versioned cost evaluator combines those bounds with benchmark measurements for the target profile. A missing, invalid, or unbounded declaration is `unranked`, not silently treated as cheap; it follows the authored fallback path instead of executing as a ranked feature.

The gates are cumulative and ordered:

1. **World policy** is authoritative. A world-level deny (including a filter disabled by the author for gameplay fairness) cannot be overridden. Any world cost ceiling already defined by #5970 remains authoritative; this design adds no new world setting.
2. **Viewer minimum rank** expresses the least performant rank the viewer accepts. Features worse than that threshold use the next eligible authored fallback.
3. **Per-person override** can exempt one named person from that viewer's minimum-rank preference. It cannot override a world deny or ceiling, platform incompatibility, a runtime hard safety limit, or a rights/license rule.

The selected content is the first candidate in #5970's authored fallback chain that is compatible, world-allowed, and permitted by the viewer gate (or its narrowly scoped person exception). The chain's order and terminal behavior remain #5970's decision. A rank failure alone is never a new instruction to hide content.

## Scope boundaries and parent decisions

- #5955 owns the broader worlds program. This record is one visual-feature ranking design within it; it does not reopen that program's decisions.
- #5970 owns per-world presentation-asset enablement and graceful degradation, including its authored fallback chain. Its requirements include assets referenced by ID and named composable filters on asset parts; for example, if a world disables a filter, the authored fallback may be static sparkles on the GIF of a sprite. The precedence above makes that world policy the non-overridable outer gate. It does not change the chain, its order, or its terminal outcome.
- #6013 / PR #7019 own the shader asset format. This record defines only the cost-ranking inputs that a shader asset must provide (below), not field names, serialization, validation syntax, shader language, or the format itself.

There are two explicit seams to reconcile, not settled parent decisions: #6012 assigns per-world asset/filter enablement to #5970, while open PR #7019 §3.5 says #6012 owns per-world enablement and §4 assigns capability enablement back to #5970; and PR #7019 §3.1 proposes an unfiltered base when no declared fallback works, while §3.5 defers that case to #5970 and §6 leaves per-tier fallback requirements open. #5970 requires authored graceful degradation but does not settle an exhausted/unusable chain. This design follows #6012's stated ownership boundary and leaves the terminal fallback outcome to #5970. Both seams are explicitly listed for escalation to the #5955/#5970 owners; neither is silently re-decided here.

## Measured cost profile

The evaluator records a versioned vector for each feature and target profile. Measurements use the actual rendered feature in a repeatable reference scene and include representative visibility/coverage; source-file size or an author's estimate alone is not a cost metric.

| Dimension | Measurement used for ranking | Applies especially to |
| --- | --- | --- |
| CPU submission/setup | Incremental CPU time per frame (p50/p95 and worst tested burst), including feature setup, state changes, and draw/dispatch submission | All; sprite batching and filter setup are prominent |
| GPU execution | Incremental GPU time per frame (p50/p95), with each pass attributed to the feature | Filters and shaders; also sprite rendering |
| Pixel work | Fragment/sample invocations or the closest supported hardware counter, plus covered screen area/overdraw for the reference view | Transparent sprites, layered effects, filters, shaders |
| Geometry and submissions | Generated vertices/indices, draw/dispatch calls, material switches, and pass count | Sprites and multi-pass effects |
| Texture/sampler pressure | Peak additional resident bytes after target compression and deduplication; sampler count and maximum sampled dimensions | Sprites, filters, shaders |
| Intermediate surfaces/bandwidth | Number and dimensions of temporary render targets, peak live bytes, and measured read/write traffic | Filters and multi-pass shaders |
| Shader work | Target-specific compiled instruction/resource bounds, dynamic-flow features where supported, and measured GPU time in the reference scene | Shaders |

The evaluator stores the target profile, evaluator/table version, test-scene version, measurement method, and the measured vector with the resulting rank. It uses the **worst band across dimensions**. Where a device lacks a counter, the evaluator uses a documented conservative proxy; it must not report a proxy as a hardware measurement. Re-measure after a material asset, evaluator, or profile change.

## Rank table

Ranks describe the cost of a feature, ordered from least to greatest cost. The viewer's “minimum rank” is a **minimum acceptable performance grade**: choose `Medium` to allow `Excellent`, `Good`, and `Medium`, while replacing `Poor` and `Very Poor` features.

The percentages are fractions of the target profile's calibrated visual-feature budget **per ranked feature in the reference view**, not fractions of total device capacity. For each measured dimension, divide its measured cost by that dimension's profile budget; the feature's rank is the band containing the **largest normalized dimension**. Profile budgets and the evaluator version are calibrated against supported hardware and representative worlds; the percentage bands are the stable policy interface.

| Rank (best to worst) | Desktop / console profile | Mobile / Quest-class profile | Viewer-minimum behavior |
| --- | ---: | ---: | --- |
| **Excellent** | Largest normalized dimension ≤ 5% | Largest normalized dimension ≤ 3% | Allowed by every threshold |
| **Good** | Largest normalized dimension > 5% and ≤ 10% | > 3% and ≤ 7% | Allowed at `Good` or a worse minimum |
| **Medium** | Largest normalized dimension > 10% and ≤ 25% | > 7% and ≤ 15% | Allowed at `Medium` or `Very Poor` minimum |
| **Poor** | Largest normalized dimension > 25% and ≤ 50% | > 15% and ≤ 30% | Allowed only at `Poor` or `Very Poor` minimum |
| **Very Poor** | Largest normalized dimension > 50% and within the profile's hard limits | Largest normalized dimension > 30% and within the profile's hard limits | Allowed only at `Very Poor` minimum |

The table is separate by target profile: mobile/Quest-class devices use stricter per-feature budget shares and a profile-specific supported-capability set, while the rank names and ordering stay common so a viewer's preference remains understandable across devices. Crossing a hard platform limit or requiring an unavailable capability is **incompatible**, not merely `Very Poor`; it goes directly through fallback selection. These initial bands are versioned defaults to validate against representative hardware, not claims that every device has identical absolute millisecond or byte budgets.

Ranks are calculated per target profile. A feature can therefore be `Good` on a desktop profile and `Poor` on a mobile profile without changing its authored asset. In a world with multiple active features, the runtime may also enforce the profile's aggregate frame/memory budget; aggregate pressure can select further authored fallbacks but does not rewrite an asset's measured rank.

## Gate layers and precedence

For candidate `c`, profile `p`, viewer `v`, and optional person `s`, display `c` only when all of the following hold:

1. **World gate:** `c` is enabled by the current world's #5970 policy and does not exceed any world cost ceiling that policy already defines. A world deny is final.
2. **Compatibility/safety gate:** `c` is supported by `p` and stays within its hard safety limits. This applies even when a viewer has a person override.
3. **Viewer minimum:** `rank(c,p)` is at least as good as `v.minimum_rank[p]`. If it is worse, reject `c` and continue through the authored fallback chain.
4. **Person exception:** only for a viewer-authored exception naming `s`, treat the viewer-minimum check as satisfied for that person's content. Do not skip steps 1–2 or any rights/license checks.

The effective result is the intersection of the world and viewer permissions; a permissive viewer setting cannot make world-disabled content run. The person exception changes only that viewer's threshold decision. Model its memory on the existing user-grant shape: an explicit, revocable viewer-owned preference keyed by viewer ID and stable person ID, with **this person in this world** as the narrow default scope. A broader “this person across worlds” scope requires explicit selection. Store it with the viewer's ID state so it follows the viewer across devices; do not send it to the world or other viewers. The control reads “allow this person's lower-ranked content here,” exposes its scope and active state, and can be revoked from the same preference surface. Permission/license systems remain the source of truth for any separate authorization decision.

If the viewer has no explicit minimum, use the product default already established by the parent presentation policy; this design does not silently choose or change that default.

## Fallback substitution

The ranking system is a reason for asking #5970's resolver to substitute, not a second fallback system:

1. Evaluate the primary feature against the target profile, world policy, and viewer threshold.
2. If the feature is unranked, incompatible, over a hard runtime limit, world-disabled, or below the viewer minimum, ask the #5970 resolver for the next candidate in its authored chain.
3. Evaluate each candidate under the same world policy, profile compatibility, safety checks, and viewer/person gates. Display the first candidate that passes. Continue in the chain's declared order; do not invent a “nearest looking” substitute or skip a candidate based on an undocumented heuristic.
4. Preserve #5970's terminal outcome. In particular, a rank rejection by itself does not authorize hiding. Hide only where the world policy/fallback chain explicitly says to hide; otherwise use the authored eligible fallback. If the chain does not specify an outcome for an exhausted chain, leave that as a parent-design question (already recorded on #6012) rather than inventing one here. In particular, this design does not adopt PR #7019's proposed unfiltered-base terminal outcome as settled until #5970 owners reconcile it.

Each substitution records the rejected candidate, its rank or incompatibility reason, the selected fallback, and the gate that caused substitution. Repeated resolution must be deterministic for the same profile, world policy, viewer preferences, and chain version; malformed/cyclic chains are rejected by #5970's resolver rather than followed indefinitely.

## Viewer legibility and overrides

Whenever the viewer sees a substitute instead of the requested feature, the interface provides a persistent, accessible indication on or near the affected content and a details action. The details name the affected feature/person, the displayed fallback, the original rank (or `unranked`/`unsupported`), and the reason in viewer language—for example: “Showing the low-cost fallback: this shader is `Poor`, below your `Medium` minimum on this device.” For a world decision, say that the world disabled or capped the feature; do not misleadingly attribute it to the viewer's setting. Use text and accessibility semantics, not color alone.

The details action exposes the viewer's current minimum and, when appropriate, a direct control to allow this person below that minimum. It does not offer an override for a world deny, capability incompatibility, hard safety limit, or rights/license rejection. The control confirms that it is a viewer preference, identifies its person and world scope, shows when it is active with a persistent indicator, and can be revoked from the same place. The person exception is private viewer preference state; no other viewer learns that the exception is active.

## Shader ranking interface for #6013

A shader asset must provide enough **semantic** information for the evaluator to rank it without executing an unbounded or unsupported shader. At minimum its format needs to carry:

- target profiles/capabilities it supports and the capabilities it requires;
- conservative resource/cost bounds used for preflight (including pass count, declared channel count, sampler use, texture dimensions, intermediate-surface limits, target pixel area, and the format's target-compilable shader-work bound);
- the feature's bounded runtime inputs or declared maximum envelope needed to measure representative cost, including relevant screen-coverage assumptions and any adapter/compiler work estimate the chosen format exposes; and
- a reference into #5970's authored fallback chain.

The evaluator, not the shader author, assigns the rank from those declarations plus measured per-profile results. A declared estimate is not accepted as proof of runtime cost; materially exceeded bounds invalidate the profile result and send the asset through fallback until re-ranked. #6013 chooses the representation, field names, validation, versioning, and authoring workflow. This contract only says what information its chosen format must make available.

## Validation and open calibration

- Test the evaluator on representative sprite-only, filter-heavy, shader-heavy, and mixed-feature scenes across desktop and mobile/Quest-class profiles.
- Compare reported deltas with repeatable CPU/GPU timer and memory measurements; the oracle is the captured measurement vector and the versioned threshold table, not an author label.
- Verify gate precedence with a decision table: world deny always substitutes; viewer threshold substitutes a worse rank; person exception bypasses only that threshold; platform and rights failures always substitute; fallback follows #5970's order and terminal behavior.
- Recalibrate absolute profile budgets and review the proposed percentage bands on real target devices before implementation locks them. Updating thresholds requires a table/evaluator version change and asset re-ranking, not a silent reinterpretation of stored ranks.

No implementation is specified here. This decision supplies #6013's ranking seam and leaves #5970's world policy and fallback decisions in force.
