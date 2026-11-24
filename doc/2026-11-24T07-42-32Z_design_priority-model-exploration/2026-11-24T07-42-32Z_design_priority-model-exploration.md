# Priority model exploration — urgency vs importance, temporal context, and the rot problem

- **Kind:** design exploration (amendment/follow-up to an existing design family). Design only — no schema, code, or queue changes are made or authorized by this record.
- **Owner design:** [#9199 — distributed priority model: urgency, importance, effective pressure](https://github.com/developing-today/code/issues/9199), under epic [#9196](https://github.com/developing-today/code/issues/9196) (PM-orchestration design mill, 2026-11-12/13).
- **Cross-links:** [#14170](https://github.com/developing-today/code/issues/14170) (queue metadata/history schema design — the temporal fields proposed here are framed as an amendment to it), [#14158](https://github.com/developing-today/code/issues/14158) (2026-11-24 queue reprioritization programme), [#14100](https://github.com/developing-today/code/issues/14100) (reprioritize verb + named priority scale).
- **Asked by:** Drew, 2026-11-24 01:37 CST. His framing, in substance: today's single priority number does double duty as a heuristic of both urgency and importance. Unblocking trivia rightly rockets up (flow), but then the number can't say what actually matters; high-urgency/low-importance items are fine to drop yet sometimes briefly useful, and left at the bottom they rot until obsolete; reviewing everything constantly is a blunt instrument, and so is blanket-very-high. He also asked whether an older issue discussed expanding the priority scale ~10x, and whether temporal context (proposed target datetime, due datetime, expired/stale datetime) could layer on top of the general ranking. He is not sure anything should change — this record exists so there is a clean option to consider.

## 1. Verdict up front

The separation Drew is asking for **already exists as a design** — the #9199 family designed intrinsic urgency, nullable importance, and derived effective pressure as separate things in November, with slices, oracles, and a completed research slice on exactly the urgency/importance nuance. What that family does **not** have is (a) first-class temporal fields on queue items, and (b) a queue-level answer to the rot problem. Those two are the genuinely new material here, and both are small. The scale question has a definite answer: the ~10x expansion Drew half-remembers was the mill charter's 0–2000+ profile, and the design already ruled on it — do not transplant it (§3). The decision page is §7.

## 2. What already exists — the recovered model

### 2.1 Live today (verified 2026-11-24)

- One integer per task. Claim order is `(priority IS NULL), priority ASC, created_at ASC` — urgency plus age, nothing else. There are no durable fields for importance, effective pressure, aging, or dependency-derived pressure (mill census, confirmed by the #14170 inventory tonight: the `tasks` table has no metadata, resolution, or labels columns either).
- The human-facing scale is the named one (amendment on #14100, 2026-11-23): `immediate` 5 / `now` 10 / `critical` 20 / `urgent` 30 / `very high` 40 / `high` 50 / `medium` 60 / `low` 70 / `very low` 80 / `minimal` 90 — higher number = lower priority.
- A known producer/consumer defect rides along: the dispatch adapter's profile `[10,20,30,40,50,70,90]` silently snaps off-profile integers to the nearest value, so the ratified #7030 values `expedite=15` / `next-up=25` change meaning at that boundary (`15→10`, `25→20`). The mill flagged this as a defect to fix explicitly (PRI-01), never as authority to renumber.

### 2.2 The #9199 model (design of record: `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9199-distributed-priority.md`)

Five things that today are conflated into one number are kept separate:

| Dimension | Meaning | Persistence |
| --- | --- | --- |
| `intrinsicUrgency` | Durable numeric urgency, lower = sooner. Null means explicitly untriaged, never a fabricated default. | Canonical in the broker; authorized edits only. |
| `importance` | Expected impact/value — a separate axis. | Nullable/provisional at first; **never derived from urgency**. |
| `effectivePressure` | Temporary execution pressure derived from intrinsic urgency + hard-dependency critical path + deadlines/opportunity + phase/resources. | Calculated from a pinned policy version + as-of + snapshot. Never rewrites intrinsic urgency. |
| peer rank | Exact-integer relative order *within* equal effective urgency (before/after moves, bounded local rebalance). | Stored explicitly; never overrides urgency. |
| `directive` | One-shot requests (`do_now`, `direct_session`). `hold` is lifecycle, not a directive. | Distinct typed field; the #7030 `expedite=15` stored band is **not** a directive. |

Eligibility (capabilities, hard blockers, leases, `manual_only`) is a **gate**, never a sortable score. The proposed claim comparator is: eligibility gate → effective urgency ascending → peer rank → created_at → canonical ID (repairing today's missing final tie-break).

Triage mechanics designed in the same document — these matter for §5:

- Every adjustment carries reason, evidence, confidence, as-of, before/after, policy version, and a **next-review/retriage time**.
- **Aging is bidirectional and evidence-based**: age alone never escalates an item to Critical; a pivot can lower a formerly High item; stagnant useful work may rise only with evidence.
- **Hysteresis**: a bounded buffer/cooldown/confidence threshold at band boundaries (slice #9483), with manual/high-confidence bypass — anti-churn, not a wall.
- **Temporary boosts** are separate from base urgency, carry explicit start/expiry, and expire to `expiryAction=retriage` — expiry removes the temporary factor and triggers reassessment; it does not silently drop or silently keep the boost.
- **Clump refinement**: progressive refinement of a coarse pile (50 → rough 25/10/5 → singles) is the intended shape of triage, with at most one ordinary automatic movement per item per sweep.
- **Protected-owner gate**: Drew's protected expedite values/placement are not editable by routine writes, aging, clump moves, or label normalization.

### 2.3 Dependency pressure (#9480, PRI-04)

Cycle-safe hard-dependency pressure: only true hard edges gate or inherit; a blocker keeps its own intrinsic urgency while its *effective pressure* derives from its dependents' urgency/critical path, bounded by effort/alternatives, fully explainable (source items, edge type, graph revision, path, cap). Never an unconditional promotion of a large speculative blocker to Emergency. This is the designed form of tonight's working rule "blocked items sit behind their blockers; the biggest unlockers move up" — #14158's pass 2 applied it by hand from a client-side graph; #14170's `task_deps` normalisation makes the graph itself a recursive SQL query.

### 2.4 Distribution and provenance (#9200)

Every priority producer and consumer traced (human rulings, labels, comments, dispatch, broker, claimant, PM projections); precedence when producers disagree; provenance + reason codes on every change; label edits normalize into the canonical score under explicit rules (a removed final label preserves the score and expedites triage — never a silent reset); replay/reconciliation when views diverge.

### 2.5 Selection (#9204)

Normal selection is deterministic urgent-first. The weighted sampler (`w_i = r^(−(u_i−u_ref)/d) × count(bucket)^(−alpha)`, defaults r=1.25, d=200, alpha=0.5, bucket 200, epsilon 0.05) is for **bounded exploration only** — eligible only when no Emergency/Critical/Do-Now work needs the lane, all dispositions persisted. Its text is explicit: importance is null/unavailable until distinct data exists; any future axis mixture is a separate versioned policy. Never fabricate importance from urgency.

### 2.6 The importance research (#9486, PRI-08 — closed, done)

This slice researched exactly Drew's nuance and selected **no weights**. Its durable conclusions:

- Candidate independent signals, scored for independence/evidence/failure mode: expected-value judgement recorded at triage (high independence, no field exists); dependency fan-out / unblock count (computable, but measures *leverage*, not value — labelled as such); risk/deadline exposure (belongs in effective pressure, **not** importance — counting it in both double-counts); strategic-alignment tag (needs scarcity to carry information); realized-value backtests (calibrates classes, not items).
- Mixing options A–F compared: A urgency-only (baseline), B lexicographic tie-break within equal urgency, C categorical importance bands + within-band rank, D weighted mixture (hardest to explain, easiest to game), E importance-weighted exploration only, F pressure-only. **Recommendation: sequence B/C/E before D** — options that cannot displace urgent work and degrade exactly to the baseline earn the right to collect the data D would need.
- Null behaviour is normative: null importance is a first-class value, never imputed from urgency, band medians, age, or labels; explanations must render `importance: null (unavailable)` so "never assessed" is distinguishable from "assessed as average".
- **The binding constraint is data existence, not formula choice.** No live item carries an independently sourced importance value today, so every mixing option is currently untestable. Calibration starts with signal collection: a distribution census, then a collection pilot (on the order of hundreds of independently assessed items, spanning every band, more than one assessor), then offline simulation against the urgency-only control, then shadow mode. A mixture that delays Emergency/Critical/`do_now` work in simulation is rejected outright, not tuned.

### 2.7 Where the family stands (2026-11-24)

#9199 stays open as the programme tracker (recorded decision, 2026-11-23: children must not depend on the parent's closure; child dep sets were re-pointed to slice-specific prerequisites). PRI-08 (#9486) is done. The build slices are designed with named oracles and currently blocked-labelled: PRI-01 versioned PriorityProfile (#9478), PRI-02 dimension types (#9479), PRI-04 dependency pressure (#9480), PRI-05 hysteresis/cooldown (#9483; its sibling scope covers aging/expiry in the component doc). Tonight's reprioritization programme is, in effect, operating the model by hand: named bands, blocker chains escalated in order, questions resolved from recorded decisions.

## 3. The scale question — was a ~10x expansion discussed?

**Yes — verified, with one correction to the memory.** There is no standalone scale-expansion issue; the discussion lives in the #9196 mill's intake sources (charter/proposal) behind #9199. That **charter profile** used a broad 0–2000+ scale: bands from Emergency 0–99 to Cold backlog 2000+, Standard default **1000**, High at 400–799 (the charter's own running example is `450/High`). Against the live 10–90 taxonomy that is roughly a twenty-fold wider range — the same idea Drew remembers as "say 10x".

The component design then ruled on it explicitly, in a section titled **"Taxonomy mapping; do not transplant the historic scale"**:

- The charter numbers are a *historical profile* for interpreting source examples. They may map to a current band only by named intent under a dated policy — never by affine conversion, never by adapter rounding, never by silently rewriting live integers.
- Profiles stay separately versioned so historical examples remain reproducible.
- The sampler's `d=200` is calibrated in the charter profile; on the live 10–90 range the exponent nearly collapses distinctions — i.e. the wide scale was load-bearing in the original math, which is precisely why its constants cannot be carried across by rescaling.

So the expansion was considered at design depth and deliberately **not** adopted for the live scale; what was adopted instead is profile versioning (PRI-01, #9478), which lets a second, finer profile exist later without renumbering anything live. §6 answers whether a finer scale would buy anything now.

## 4. Temporal context as first-class fields (new)

The mill touches time in three places — deadlines/opportunity as unnamed inputs to effective pressure, boost expiry, and a next-review time recorded on adjustments — but no queue item carries a target, due, or expiry datetime today, and #14170's schema design has none either (it derives *staleness* from `task_history`, a different thing: how long since the item last moved, not when its value ends). Drew's three datetimes are therefore a genuine addition. Proposed as an amendment to #14170 (its split rule puts anything filtered, sorted, or joined into columns — and every planning query below filters on exactly these):

| Field | Meaning | Set by | Column or derived |
| --- | --- | --- | --- |
| `target_at` | **Proposed target datetime** — when the proposer aims to have this done. Ours; advisory. | Whoever packets/triages the item; revisable with a history row. | Column. |
| `due_at` | **Due datetime** — when the world needs it: an external deadline, an event, a dependency window, an opportunity's end. | Only with evidence attached (the deadline is a fact about the world, not a wish). | Column. |
| `expires_at` | **Expired/stale datetime** — after this, the item's value is presumed decayed or void unless re-triaged. | Triage, for time-boxed work (briefly-useful items, soak windows, version-bound work). | Column. |
| urgency-from-due | Distance-to-`due_at` mapped by policy into effective pressure. | Derived at selection/explanation time. | **Derived — never stored.** It feeds #9199's pressure input "deadlines/opportunity", giving that designed input a concrete field to read. |
| staleness age | Now − last meaningful transition. | Derived from #14170 `task_history`. | **Derived — never stored.** |

Interaction rules (the separation discipline applied to time):

1. **The clock never rewrites intrinsic urgency.** Due-proximity raises *effective pressure* only. The number that means "how much this matters / how soon in itself" stays a human/policy judgement; time pressure is a derived, explainable overlay — exactly the mill's pressure contract, now with a named input.
2. **`target_at` does not affect ordering in v1 at all.** It is planning and reporting data (what did we aim at, what did we hit). That is what makes it cheap to adopt: no selection semantics to calibrate.
3. **`expires_at` passing is an event, not a verdict.** It writes a state-neutral history row and surfaces the item in the last-chance view (§5); resolution `obsolete` with substatus `expired` — a value #14170's taxonomy already carries — is applied by a sweep decision or by an explicit per-queue auto-expire policy, never by the mere passage of time.
4. All three are nullable, and null is honest: most items have no due date, and inventing one would reproduce the fabricated-default defect the mill forbids for urgency.

## 5. The rot problem — mechanisms, costed

Drew's population: high-urgency/low-importance items that are fine to drop but sometimes briefly useful; parked at the bottom, they are noticed only when their turn comes, by which time they are redundant or obsolete. The two blunt instruments he named — review everything constantly (unbounded human attention) and blanket-very-high (destroys the signal for everything else) — are the costs to beat.

| Mechanism | What it does | Verdict and cost |
| --- | --- | --- |
| **Decaying urgency** (score auto-worsens with age) | Items sink unless touched. | **Rejected — the mill already ruled this out.** Aging is bidirectional and evidence-based; automatic drift is uncontrolled drift, silently renumbers the estate, and cannot distinguish "rotting" from "patiently waiting". |
| **Expiry + last-chance surfacing** (§4 `expires_at`) | Time-boxed items declare their horizon at triage; before it passes they appear once in an expiring view (stewardship / PM surface); at expiry they go to retriage, not to the void. | **Recommended — the core rot mechanism.** Cost: one column, one query, one surfacing surface. No ordering change, no score change. This converts "noticed when its turn comes" into "noticed while it can still matter". |
| **Staleness sweep as system behaviour** | A periodic query over `task_history` (last transition older than a per-band threshold) produces a **bounded** retriage list, under the mill's sweep discipline (at most one ordinary automatic movement per item per sweep; cooldowns; hysteresis at band edges). | **Recommended.** Cost: a scheduled job plus the bounded triage labour it surfaces — bounded by design, unlike constant review. The system brings you the stale ones; humans still decide. |
| **Bounded exploration** (#9204 sampler) | When no urgent work needs a lane, low items get a small, fixed-mass draw with persisted dispositions. | **Already designed — name it as the third leg.** The rot population is exactly what exploration samples; the fixed-mass shape gives old low items a nonzero chance of being *looked at* without promoting them. Cost: none new. |
| Blanket-very-high for briefly-useful items | — | **Rejected** (Drew's own argument): it spends the top of the scale on the items that matter least, and the top stops meaning anything. |

Net: expiry-with-surfacing + staleness sweep + exploration costs one datetime column and sweep discipline, and buys the property neither blunt instrument has — **the item surfaces itself while action is still useful, and expiring is a recorded decision instead of a discovery made too late.**

## 6. Scale granularity — does ~10x finer buy anything once axes are separated?

Answered with the model, not by taste:

- The failure Drew named — "we lose the ability to parse what matters and what doesn't" — is an **axis-conflation** failure, not a resolution failure. A 900-point single scale still encodes two judgements (how soon, how much it matters) in one number; finer grain makes the conflation *more* precise-looking, not more parseable. Counterexample from tonight: the 276-item pile at `urgent` (30) is not a granularity problem — those items are indistinguishable because one number is carrying both axes, and splitting the number into 2,760 possible values would leave them exactly as indistinguishable.
- What granularity *does* buy in the mill design is mechanical, and it is already provided elsewhere: room to order peers without collisions is provided by the exact-integer **peer rank** (PRI-03 design), and smooth sampler weights are a *calibration* property of the profile (the `d=200` lesson), not a reason to renumber humans' bands.
- Human judgement is categorical at the point of triage — the mill's own triage vocabulary is much-more / slightly-more / equal / slightly-less / much-less. A coarse named scale matches the instrument actually used to assign values; the fine distinctions belong in rank and derived pressure, where machines consume them.

**Verdict: do not expand the live scale.** If a future machine consumer needs finer resolution, PRI-01's versioned `PriorityProfile` already allows a second profile to exist beside the named one without touching a single live value.

## 7. The queue as a planning dataset — which fields earn their storage

Drew's test: queue items needn't back up all of GitHub, but searchable / queryable / groupable queue data pays for planning, reflection, reports, reprioritization, and cleanup. A field earns storage when a named query reads it and GitHub does not already answer that query cheaper.

**Earn storage (columns):** intrinsic urgency and nullable importance (every band histogram, every audit); `target_at` / `due_at` / `expires_at` (due-this-week planning, expiring view, hit-rate reflection: target vs actual); the #14170 set — resolution taxonomy, labels cache, `task_deps`, `queue_links` (unlock graph and coverage floor as SQL, not client-side assembly).

**Derived, never stored:** effective pressure (a function of pinned inputs — storing it would store a stale opinion), staleness age, unlock/fan-out counts, time-in-band. Each is one query over history + deps when a report needs it.

**Redundant with GitHub — do not store:** comment bodies, reactions (#14170 considered and rejected), full label history (the webhook mirror owns it; the queue caches only the current set). The queue mirrors what planning queries need, not the conversation.

**Metadata (JSON), not columns:** programme, tracking issue, held marks, coverage exceptions (#14170's registry) — context that travels with the item but is never a sort/filter key. The moment a metadata key becomes a planning filter, it graduates to a column under #14170's rule; the three datetimes start as columns because their queries are already named.

## 8. Decision page for Drew

**The question:** the single priority number conflates urgency and importance, and briefly-useful work rots unnoticed. Three clean options; all preserve every live value (no renumbering under any option):

**Option A — Adopt the multi-axis model as designed, plus the temporal amendment.**
Build the #9199 slices in their designed order (PRI-01 profile → PRI-02 dimension types → PRI-04 dependency pressure → PRI-05 aging/hysteresis), add the three datetime columns via #14170, importance collected null-first (recorded when a triage judgement exists, never fabricated), expiry + staleness sweep switched on.
*Daily behaviour:* triage records importance and dates when known; the number stops lying — urgent trivia is visibly trivia that unblocks, and important slow work is visibly important; stale and expiring work surfaces itself; selection stays urgent-first throughout.

**Option B — Single number + temporal fields only.**
Keep today's integer and named bands exactly as they are; add only `target_at` / `due_at` / `expires_at`, the last-chance surfacing, and the staleness sweep.
*Daily behaviour:* ordering is untouched; the rot problem is handled (expiry is declared, surfaced, and decided instead of discovered late); the urgency/importance conflation remains — the number still cannot say what matters, only when it goes stale.

**Option C — Status quo + staleness surfacing only.**
No new fields anywhere. Stewardship surfaces stale items from #14170's history (last-transition age) on a cadence.
*Daily behaviour:* cheapest possible change; the "not noticed" symptom is reduced but not solved — with no expiry dates, the sweep cannot distinguish *stale* (value gone) from merely *quiet* (patiently waiting), so its list needs more human judgement per item than B's.

**Recommendation: A, staged B-first.** Do B's temporal slice first — it is small, immediately useful, and its `due_at` is a prerequisite input to A's pressure derivation anyway — then the PRI chain in its designed order, with importance collection starting as a pilot on newly triaged items (the PRI-08 sufficiency path). The why: the mill already paid the design cost, and its research showed the binding constraint is *data existence*, not model choice. The temporal fields plus null-first importance collection **are** the data-collection step; adopting the model's formulas before collecting its inputs would repeat the conflation under new names. C is the honest fallback if even B's three columns are unwanted — strictly better than today, strictly weaker than B.

**Explicitly not asked, under any option:** a scale expansion (ruled out in §6), a live renumbering, or a queue migration. This record changes no behaviour until Drew picks an option and the corresponding slices are packetised.
