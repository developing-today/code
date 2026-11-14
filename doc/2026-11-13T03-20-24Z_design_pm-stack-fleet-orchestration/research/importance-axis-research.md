# PRI-08 — Importance-axis mixing and calibration options research

<!-- pmo-9196:PRI-08 -->

**Issue:** #9486 · **Design parent:** #9199 (distributed priority) · **Slice:** PRI-08
**Status:** research only. This document selects **no** numeric weights, makes **no**
live taxonomy change, and authorizes no schema or production implementation.
Every proposed weight, mixture, and threshold below is marked **DRAFT**.
**Requirement IDs (allocated):** C-R040, C-R105, H-R010.
**Gate state at writing:** PR #9225 merged 2026-11-13T13:36:36Z, so this design
tree is on `main` and this research is unblocked. Persisted importance fields
remain gated by #9198/QTI and the #9073/#9074 owners; offline research is not.
**Oracle:** `pmo_importance_axis_prior_art_comparison_is_reproducible` — this
artifact must contain a source-traceable alternatives matrix, evidence and
limitations, a null-data path, and the explicit decision that importance is
not inferred from urgency.

## 1. Question

The live system orders work by one axis: intrinsic urgency, lower-is-sooner,
on the ratified #7030 taxonomy (discrete profile `[10, 20, 30, 40, 50, 70, 90]`
in `pm-stack/dispatch/src/validate.ts` — `critical=10`, `high=20`,
`normal=40`, `low=70` — with the ratified stored values `expedite=15` and
`next-up=25` preserved losslessly and distinct from the `do_now` directive).
#9199 defines `importance` as a second, independent dimension: expected
impact/value, nullable/provisional at first, **never derived from urgency**.

This research compares (a) candidate independent importance/impact signals,
(b) future profile-mixing options that could combine the two axes, and
(c) calibration options for any such mixture — against the existing
low-is-sooner urgency taxonomy — and specifies the evidence, data
sufficiency, and null/unavailable behavior each option needs before any
future, separately authorized decision could adopt it.

## 2. Evidence base — fact, inference, anecdote

Labels used throughout: **[fact]** = measured or source-verified in the
pinned design package (`a038f084dbeb` baseline, PR #9225 tree) or in live
readbacks cited inline; **[inference]** = a reasoned conclusion drawn from
those facts, not itself measured; **[anecdote]** = a single observed example
or historical source statement, usable as a hypothesis seed only.

- **[fact]** The current claim order in the broker is
  `(priority IS NULL), priority ASC, created_at ASC` — urgency plus age only;
  the census found no durable separate fields for importance, effective
  pressure, aging, or dependency-derived pressure (#9199 "V1 — observed
  baseline"). A queue integer plus opaque payload metadata is not evidence
  of a multi-axis contract.
- **[fact]** The dispatch adapter silently snaps off-scale integers to the
  nearest profile value (ties down), so ratified `15→10` and `25→20` change
  meaning at that boundary; #7030 readbacks record 15/25 as actual broker
  values. Any future importance value passing through a similar adapter
  would inherit this defect class unless the contract is fixed first
  (FDR-PMO-002).
- **[fact]** The charter's `450/High`, `1000` ordinary default, and the
  weighted formula's `d=200` belong to a historical 0–2000 charter profile.
  They are source examples, not live scores: on the live 10–90 range the
  `d=200` exponent nearly collapses distinctions (#9199, "Taxonomy mapping").
  This research therefore never copies 450/1000 into live priority and
  never treats an affine conversion between profiles as valid.
- **[fact]** #9204's candidate weighted-selection formula
  (`w_i = exp(-ln(r)*(u_i-u_ref)/d) * n_bucket(i)^(-alpha)`, uniform mixture
  `q_i=(1-epsilon)*Pweighted(i)+epsilon/N`, sampling without replacement
  over a frozen eligible population) is specified for the *urgency* axis
  and for exploration; its own text states: "Importance is null/unavailable
  until distinct data exists. Do not fabricate it from urgency. Any future
  axis mixture is a separate versioned policy with distinct filter and
  statistical tests."
- **[fact]** FDR-PMO-008 keeps all historical exploration settings (4%,
  3 candidates, `r=1.25`, `d=200`, `alpha=.5`, width 200, `epsilon=.05`) as
  hypotheses, not adopted constants, and records that no representative
  production distribution or performance benchmark exists yet.
- **[inference]** Because no item in the live queue carries an independently
  sourced importance value today, every mixing option is currently
  untestable against real data; the binding constraint is data existence,
  not formula choice. Calibration work must therefore start with signal
  collection and distribution evidence, not with weight tuning.
- **[anecdote]** The charter profile's own examples (450 = High,
  1000 = new ordinary default) show a past design intent to separate
  "how much it matters" from "how soon it is needed" (charter C-052/C-144,
  handoff H-135–H-137 occurrences behind C-R040/C-R105/H-R010). They are
  evidence that the *distinction* was wanted, not evidence for any
  particular numeric mapping.

## 3. prior art

Receipts are the two state-of-the-art tracks shipped in this design tree
(`research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`,
with `research/track-a-sources.json` / `research/track-b-sources.json`).
Only precedents that separate *priority* from *share/fairness* or from
*within-group order* transfer to the importance question:

| Precedent (Track A receipt) | Documented mechanism | Lesson for an importance axis |
|---|---|---|
| Kubernetes Pod Priority and Preemption | Numeric priority orders scheduling; feasibility is a separate gate. | An axis value must never make a hard-ineligible item eligible; eligibility stays a gate, not a low weight. Preemption is **not** borrowed for leased agent work. |
| Kubernetes API Priority and Fairness (APF) | Requests classified into priority levels/flows; fair dispatch controls shares of concurrency. | "Which first" (priority) and "who gets a share" (fairness) are different questions. Importance, if it ever mixes, is a third question again — expected value — and must not be conflated with either. |
| Linear priority groups / Jira ranking (LexoRank) | Categorical priority plus a human-controlled rank *within* a group; rank is a separate field. | Supports categorical importance bands + within-band peer rank as a low-risk mixing shape (option E below); also warns that rank rebalance is a first-class maintenance problem. |
| Temporal / CloudEvents / Kafka (Track A/B) | Durable command identity, replay with pinned inputs, partition-local ordering only. | Any derived mixture must pin policy version + input snapshot to be explainable/replayable; a mixture computed from unpinned inputs is not auditable. |

What prior art does **not** establish: any validated numeric weight for
mixing importance with urgency in a work queue. No receipt measures a
production benefit for a mixed score, and this research asserts none
(negative case: simulated production benefit is not claimed anywhere in
this document; all simulation outcomes would be hypotheses until run).

## 4. Candidate importance signals (independent of urgency)

Each signal is scored for *independence* (can it be observed without
looking at urgency?), *evidence quality today*, and *failure mode*.
None is adopted; all collection is future, separately scoped work.

| Signal | Definition | Independence | Evidence today | Failure mode |
|---|---|---|---|---|
| Expected value / impact note | A triage-time structured judgement (expected value, strategy fit, risk, scope — the C-R040 outcome fields) recorded with reason + confidence + as-of. | High — recorded by a human/triage process that never reads the urgency integer. | **[fact]** No durable field exists (census). | Subjective drift; needs the #9199 adjustment discipline (reason, evidence, confidence, previous/new, next-review) to stay auditable. |
| Dependency fan-out (unblock count) | Number of items whose hard gates this item clears (typed dependency graph, #9208/FDR-PMO-005). | High — computable from the graph, not from scores. | **[fact]** Graph edges exist as queue dep-blocks; no aggregated fan-out metric is computed. | Gameable by splitting work; measures *leverage*, not value — a proxy, labelled as such wherever used. |
| Risk / deadline exposure | Verified external deadline, incident, or opportunity-expiry evidence attached to the item. | Medium — deadlines already feed `effectivePressure` in #9199; counting them again as importance would double-count. | Partial: pressure inputs are design-specified, not implemented. | Double-counting with effective pressure; this research recommends risk live in pressure, **not** in importance, unless a future decision explicitly separates the two. |
| Strategic alignment tag | Membership in a ratified program/epic with an owner-recorded priority of the *program*. | Medium — program membership is a fact; program importance is an owner judgement. | **[anecdote]** Epic labels exist fleet-wide; no owner-recorded program ranking exists. | Label inflation; every epic becomes "strategic". Needs scarcity (a bounded number of top programs) to carry information. |
| Realized-value backtests | Post-completion outcomes (merged PRs, closed gates, measured effect) aggregated per class of work. | High — observed after the fact. | None collected for calibration purposes. | Survivorship and lag; calibrates *classes*, not individual items; useful for validating the other signals, not as a live input. |

Explicitly rejected signals: any function of the urgency integer, of
queue age alone, or of label *presence* without owner judgement — each
either derives importance from urgency/age (forbidden) or carries no
information.

## 5. alternatives — mixing options

All options assume the #9199 contract: eligibility is a gate first;
`importance` is a separate, nullable field; explanations name the policy
version and pinned inputs. **No option is selected by this research.**

| Option | Shape | Pros | Cons | Data needed before it can even be evaluated |
|---|---|---|---|---|
| **A. Urgency only (do nothing)** | Status quo: order by effective/intrinsic urgency, peer rank, created_at, identity (#9199 comparator). | Zero new data, zero new failure modes; fully explainable today. | Importance information, where humans hold it, is invisible to ordering except via urgency edits that corrupt the urgency axis's meaning. | None. This is the baseline every other option must beat *with evidence*. |
| **B. Lexicographic secondary axis** | Urgency primary; importance breaks ties only *within* an equal-urgency peer group, before peer rank. | Cannot displace a more urgent item; failure is bounded to within-band reordering; null importance degrades exactly to option A. | Effect is small by construction — most groups have one member per urgency value, so the axis rarely fires; may create false confidence that importance "is handled". | Independent importance values on a meaningful share of same-urgency peers (see §7 sufficiency). |
| **C. Categorical bands + within-band rank** | Importance recorded as a small set of named bands (Linear/Jira prior art); ordering stays urgency-first, bands annotate/explain and order within band via peer rank. | Human-legible; matches how triage actually judges ("much more / slightly more / equal / less" categories in #9199 triage mechanics); no arithmetic on judgements. | Band boundaries become the new argument; still needs the rank-rebalance machinery. | Band definitions with scarcity (bounded top band) and owner agreement on assignment rules. |
| **D. Weighted mixture score (DRAFT)** | A versioned policy computes a combined score from normalized urgency and normalized importance, e.g. extending the #9204 weight shape with a second axis term. All weights **DRAFT**; none proposed for adoption here. | One comparable number; tunable; the #9204 simulation oracle (frozen population, seeded draws, exact first-draw analytics) can evaluate it offline. | Normalization hides judgement inside arithmetic; on the live 10–90 scale, charter-calibrated constants (e.g. `d=200`) collapse distinctions; a mixed score is the hardest option to explain and the easiest to game; silently becomes "importance inferred from urgency" if importance is imputed for missing items. | Independently sourced importance for a representative population **plus** a live score-distribution census **plus** preregistered simulation acceptance criteria (FDR-PMO-008 pattern). |
| **E. Importance-weighted exploration only** | Normal selection stays urgency-only (option A); importance may only influence the *exploration* sampler's candidate weights (FDR-PMO-008, disabled by default). | Production ordering is untouched; the axis earns evidence in the lowest-risk surface first; kill switch already designed. | Evidence accrues slowly and only about cold work; does not help head-of-queue choices. | Same signal data as D, but tolerable at far lower coverage because normal selection is unaffected. |
| **F. Effective-pressure only** | Derive all cross-axis effects through #9199 `effectivePressure` (deadlines, dependency critical path, phase) and keep importance purely as recorded metadata for humans. | One derived quantity to calibrate instead of two; pressure inputs are evidence-based, not judgement-based. | Conflates "valuable" with "time-critical" — exactly the confusion C-R040 exists to prevent; valuable-but-not-urgent work stays invisible. | Pressure policy implementation and its own calibration; does not satisfy the importance requirement, only defers it. |

**Research recommendation (not a decision):** sequence B/C/E before D —
options that cannot displace urgent work and degrade exactly to the
baseline earn the right to collect the data option D would need. Option D
stays a named hypothesis until §7's sufficiency criteria are met and a
separate decision records adopted weights; until then all its weights
remain **DRAFT** and unused.

## 6. The null / unavailable behavior (normative for any future option)

- Importance is **null** until an independent signal in §4 is recorded
  with provenance (actor/source, reason, confidence, as-of). Null is a
  first-class value, never an error and never a hidden default.
- Null importance is **not inferred from urgency**, not imputed from
  band medians, age, labels, or any other field. An item with null
  importance orders exactly as it would under option A.
- Any mixture formula must define its null path explicitly: a null axis
  contributes *no term* (the mixture reduces to the urgency-only result
  for that item), rather than substituting a fabricated neutral value
  that would silently rank the item mid-pack.
- Explanations must render the absence: `importance: null (unavailable)`
  in the ordering explanation, so a reader can distinguish "assessed as
  average" from "never assessed".
- Coverage is reported as a measurement (share of eligible items with
  non-null importance, by source) wherever a mixture is evaluated;
  a mixture evaluated on a population whose importance coverage is
  anecdotal must say so in its results.

## 7. Calibration options and data sufficiency

Calibration = turning a DRAFT mixture into an adopted one with evidence.
Falsifiable measurement needs, in dependency order:

1. **Distribution census (fact-gathering).** Snapshot the live eligible
   population: urgency values, same-urgency group sizes, lane/filter
   split. Falsifies or supports the premise of option B (if
   same-urgency groups are almost always singletons, B is dead on
   arrival — a checkable claim, run before any build).
2. **Signal collection pilot.** Record §4's expected-value signal (and
   fan-out, computed) on newly triaged items for a bounded period,
   without using it for ordering. Sufficiency gate **(DRAFT)**: on the
   order of hundreds of items with independent assessments, spanning
   every live urgency band and more than one assessor, before any
   mixture is fitted — below that, results are anecdote, and the
   research conclusion must remain "insufficient data".
3. **Offline simulation (the #9204/FDR-PMO-008 oracle).** Freeze
   populations, declare intervals/tolerances *before* running, compare
   candidate mixtures against the urgency-only control on: urgent-work
   delay (must be zero for B/C/E shapes by construction; measured for
   D), cold-work discovery rate, and explanation completeness. A
   mixture that delays Emergency/Critical/`do_now` work in simulation
   is rejected outright, not tuned.
4. **Backtest against realized outcomes.** Where realized-value data
   (§4) exists for completed items, check whether high-importance
   items actually delivered more measured effect than a random or
   urgency-only ordering would have surfaced first. Pre-register the
   effect metric; a null result is a publishable answer and keeps the
   weights DRAFT.
5. **Shadow mode before any live use.** Any adopted-in-principle
   mixture first runs read-only/shadow (#9199 PRI-02 pattern): it
   publishes what it *would* order, humans compare against actual
   claim order, and divergence is reviewed before activation is even
   proposed. Activation remains a separate approval; nothing in this
   research activates anything.

## 8. limitations

- This research is **offline and document-only**. It measured nothing
  in production; every production-facing claim above is labelled fact
  (from the pinned design package), inference, or anecdote, and the
  package itself warns that pinned-source claims are not deployment
  proof.
- No queue distribution census, importance dataset, or simulation was
  run for this document — those are §7's future work items, so all
  mixing evaluations here are structural (what *could* be tested and
  what would falsify it), not empirical rankings of the options.
- The #7030 taxonomy facts are taken from the design package's cited
  readbacks (comments 6715673367 and 6786217364 on #7030); #7030's
  separate staffing/coverage acceptance remains open and is not
  re-litigated here.
- Queue Table Improvements (QTI) remains **unlocated/unverified**;
  #9241 is the existing provenance-recovery issue. This research claims
  no QTI equivalence for any field or option, and any persisted-field
  design must wait for the QTI crosswalk and its named owner.
- The historical charter numbers (450/1000, `d=200`, 4%/`r=1.25`/
  `alpha=.5`/`epsilon=.05`) are preserved as source examples and
  simulation hypotheses only. Treating any of them as a live value,
  an adopted weight, or evidence of production benefit would violate
  this slice's negative cases.
- Persisted importance storage timing is coordinated with #9200 and
  gated by #9073/#9074; this document designs no schema, field
  spelling, or migration.

## 9. Requirement trace

- **C-R040** — importance/impact as a separate (here: researched) axis:
  expected value, strategy, risk and scope as candidate signals (§4);
  importance remains null/unfabricated until evidence exists and is
  never urgency-derived (§6).
- **C-R105** — when separate importance scores exist, a tunable
  urgency-versus-importance mixture may be *evaluated*: option D (§5),
  gated on §7's sufficiency and calibration path; the urgency-only
  comparator remains the control.
- **H-R010** — the weighted-selection formula and candidate defaults
  are retained for evaluation as hypotheses (§§2–3, 5, 7): normalize
  over eligible work, sample without replacement, expose seeded
  simulation/explanation, never fabricate importance.

## 10. Done check

```
python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md").read_text(); assert all(x in s for x in ("alternatives", "prior art", "null", "not inferred from urgency", "limitations"))'
```
