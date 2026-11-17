# Design: Atlas classification revalidation cadence — rule-triggered rescans, sampled human audits, allowlist expiry

- **Issue:** #6051 (parent #6021, classification revalidation scan)
- **Status:** design record. No scanner, scheduler, or allowlist-store
  code is written or enabled by this document; implementation is a
  follow-on under #6021 (scan execution) and #6019 (ingestion
  operations, the standing gate's owner).
- **Inputs consumed, not re-defined:** #6021's corpus figures
  (internal: 28 shards / 107,001 chunks; secret: exactly 1 shard /
  638 chunks; secret material exists only in the 2 secret DBs), its
  handling rules (findings handled at secret-tier sensitivity;
  confirmed mis-tiered chunks get a remediation path — move + re-embed
  at the correct tier, purge from the wrong tier incl. replicas — filed
  as a follow-on before close), and its scan layers (deterministic
  secret scan full corpus both tiers; PII NER scoped per #6048;
  personal-inventory pattern pack per #6049; vector-neighborhood +
  sample classifier checks); #6048's pass-level decisions (Tier A
  checksum-backed vs Tier B NER+context, two-signal emission rule,
  internal 100% / secret 25% stratified sample of 160 chunks seeded,
  500-chunk batches with config-hashed checkpoints, quarantine-candidate
  review queue, 1,000-chunk pilot before any census); #6019's standing
  requirements (trigger model per source class, usefulness test,
  disk/IO guardrails, plan-spend ceiling).
- **Source note (required by task 13868's packet):** the prior-art
  source named in the #6051 body
  (`~/workspace/scratch/atlas-ingestion-brief-2026-11-04/prior-art-research.md`)
  is absent on the fleet laptop (same finding as the #6048 record).
  This record is written from the finding summary carried in the
  #6051 body itself, plus the corpus figures stated in #6021 and the
  landed #6048 design. Where a number is an estimate rather than a
  cited figure, it is labelled an estimate with its assumptions.
- **Dependency note:** when the 2026-11-05 triage ran, #6021 had not
  yet produced its recurrence recommendation (one-off vs standing
  gate), which this design takes as input. As of this writing #6021
  is still OPEN with no scan executed and no recurrence recommendation
  recorded. This record **assumes the standing-gate branch** — the
  direction the parent program (#6018) and #6019 already commit to —
  and §8 states exactly what collapses if #6021 recommends one-off.
  Nothing here blocks the #6021 wave-1 scan from running.

## 1. Problem

One-time scanning certifies a moment, not a corpus — the #6051 body
names this the dead end the cadence exists to avoid. Three forces
re-open a certified corpus:

1. **Rules improve.** Scanner rules, the inventory pack (#6049), and
   the recognizer set (#6048) all change. Every change retroactively
   redefines what the last scan could see: new rules find old leaks.
2. **Corpora re-tier and grow.** New ingestion lands continuously
   (#6019's operational source set); sources turn out dirtier than
   believed; the manifest's per-source tier rules (#5410) get
   corrected. A chunk correctly tiered at ingest can become
   mis-tiered by a later rule change without moving.
3. **Allowlists rot.** Every suppressed finding is a bet that the
   suppression is still right. A stale allowlist — reason forgotten,
   scope drifted, expiry missing — is how leaks get grandfathered:
   the scanner still runs, but it no longer sees.

The cadence is therefore a **recurring gate with three moving parts**,
per the body: rule-update-triggered rescans, a sampled human audit
per round that turns each detector's "we don't catch everything"
disclaimer into a measured miss rate, and allowlist entries with
reasons and expiry.

## 2. Trigger table

A revalidation round is started by exactly one trigger. The trigger
decides the scope (§3); the round always ends with an audit sample
(§4) and an allowlist review for entries expiring inside its window
(§5). Triggers are checked in priority order; a higher-priority
trigger subsumes lower ones opening in the same window.

| # | Trigger | Fires when | Priority | Window / SLA |
|---|---|---|---|---|
| T1 | Recognizer / rule-pack change | #6048 recognizer set, #6049 pattern pack, deterministic rule set, or stopword/threshold tuning merges (any `config_hash` change, cf. #6048 §4.4) | 1 (highest) | Rescan starts within 7 days of the config merge; the config merge itself is blocked on a rescan plan (scope + compute window) being recorded |
| T2 | New source class or tier-rule change | A new source class first feeds a tier, or a #5410 manifest per-source tier rule changes | 1 | Targeted rescan of the affected source class within 7 days; full-cadence scope only if the class straddles tiers without a rule (#6021's open question) |
| T3 | Corpus-growth threshold | Internal tier grows by ≥5% chunks (estimate threshold; ≈5,350 chunks at current 107,001) since the last full round, or any single ingestion run adds ≥1,000 chunks (estimate) | 2 | Targeted rescan of the new content at ingest + inclusion in the next calendar round; threshold re-tuned after the first two rounds from measured growth |
| T4 | Calendar round | Quarterly (estimate cadence; first two rounds may run monthly to calibrate, then relax) | 3 | Full deterministic layer + audit sample every round; full NER census at most annually unless T1 fires for Tier B — see §3 |
| T5 | Allowlist expiry sweep | Any allowlist entry reaches its `expires_at` (§5) | 2 | The entry's scope is rescanned un-suppressed before the entry may be renewed; entries expiring inside a T1–T4 round's window ride that round |
| T6 | Incident / ad-hoc signal | Staging-path check (#5669) flags a tier-assignment mistake; a review-queue confirm (§5 of #6048) indicates a systematic miss; an external incident suggests a new leak class | 1 | Scoped rescan defined in the incident record; may promote to a full round if the miss looks systematic |

What does **not** trigger a round: a new Atlas shard being added
with no new source class (covered by staging-time scanning at
ingest, #6021 §"staging path" / #5669 mechanics — the chunk is
scanned before it joins a tier); a pure infrastructure change
(host, replica layout, embedding model version for search — the
latter triggers re-embed, not reclassification, and is out of scope).

## 3. Scope per trigger per layer

Cost discipline comes from #6048 and #6021: the deterministic layer
is cheap (GB-scale histories are its native workload) and always
runs in full; the NER Tier B layer is the expensive layer and runs
in full only when its inputs changed or on the slow calendar. The
table below is normative.

Corpus symbols: `N` = internal chunks (107,001 cited); `S` = secret
chunks (638 cited); `Δ` = chunks added since last round.

| Layer | T1 (rule change) | T2 (source/tier-rule) | T3 (growth) | T4 (calendar) | T5 (expiry) | T6 (incident) |
|---|---|---|---|---|---|---|
| L1 deterministic secret scan (gitleaks-class, both tiers) | **Full**: N+S | Affected source class + both tiers if class straddles | Δ + affected shards | **Full**: N+S every round | Entry scope, un-suppressed | Incident scope, minimum affected class |
| L2 Tier A (checksum/structure, #6048 §2.1) | **Full**: N+S | Affected class, both tiers | Δ | **Full**: N+S every round | Entry scope, un-suppressed | Incident scope |
| L3 Tier B NER (#6048 §2.2) | **Full internal** N + 160-chunk secret calibration sample **iff** the change touches Tier B (recognizers, context words, thresholds); if the change is Tier-A-only or deterministic-only, Tier B runs **sampled** (1,000-chunk stratified pilot shape, #6048 §4.3) as a regression check | Affected class at #6048 scoping (internal portion in full; secret portion sampled) | Δ at #6048 scoping | **Sampled** (1,000-chunk stratified + 160-chunk secret sample) per round; **full internal census at most annually** (estimate; re-tune after first census cost is measured) | Entry scope at #6048 scoping | Incident scope at #6048 scoping; escalate to full internal if the miss is Tier B systematic |
| L4 pattern pack (#6049) | **Full**: N+S (pack changes are cheap to re-run; pack is regex-class) | Affected class | Δ | **Full**: N+S every round | Entry scope, un-suppressed | Incident scope |
| L5 vector-neighborhood + sample classifier (#6021 scope) | Re-run iff Tier B or embedding-adjacent logic changed; else sampled | Affected class (neighborhood of the class centroid) | Δ neighborhood check (new chunks vs secret profile) | Sampled every round | Not run (expiry is a suppression question, not a neighborhood question) | Incident scope |
| Staging-path check (#5669) | No round scope (staging scans at ingest with the *new* config from merge) | New class onboarding includes staging-rule review | Δ scanned at ingest | Audited via §4 sample (are staged chunks landing in the right tier?) | No scope | Scoped to the flagged path |

Run mechanics are inherited, not re-invented: every round mints a
`run_id`; the recognizer/pack/rule configuration is content-hashed
into every checkpoint and finding (#6048 §4.4); batches are 500
chunks per shard with idempotent resume; a run is complete when every
in-scope shard has a finished checkpoint for the current
`config_hash`; the run report is generated from the manifest. A T1
round never mixes configs: chunks scanned under the old hash keep
their checkpoints; the new hash starts a new `run_id`.

## 4. Sampled human audit per round — the miss-rate estimate

Every round, regardless of trigger, ends with a hand-checked sample
drawn from **each tier**. The sample's job is the one the body
states: to *estimate the scanners' miss rate instead of assuming
it*. "No detector catches everything" is every tool's disclaimer;
the audit turns it into a number with a confidence interval, tracked
round over round.

### 4.1 What is audited

The sample is drawn from chunks the round **did not flag** (the
pass's negatives), stratified by source class proportional to tier
composition. Reviewers — human, or an agent acting as reviewer with
name + timestamp recorded, per #6048 §5 — read the full chunk under
secret-tier handling and rule each sampled chunk `clean` or
`miss` (miss = should have been flagged under current rules), with
the rule that would have caught it. Misses are filed as review-queue
candidates and feed threshold/pack tuning exactly like dismissals
do in #6048 §5; they are tuning data, not blame.

A second, smaller slice re-reviews the round's **dismissed**
false positives (a 10% resample, estimate) to catch reviewer drift:
if re-review overturns dismissals above the §4.3 escalation rate,
the round's queue is re-opened.

### 4.2 Sample sizes (planning values, estimates)

Rare-event math governs: leaks are (we hope) rare, so the sample
must be large enough that *zero misses observed* still means
something. For a tier with true miss rate `p`, `n` clean samples
bounds `p` above at roughly `3/n` (95% rule-of-three). The table
gives planning sizes; the first two rounds calibrate them.

| Tier | Sample `n` (negatives) | Zero-miss reading (95%) | Why this size |
|---|---|---|---|
| Internal (N≈107k) | **300** (estimate) | true miss rate < ~1% | 300 hand-reads is a bounded review load (≈1–2 reviewer-days at 150–300 chunks/day, estimate); <1% on the broadly-queried tier is the headline guarantee |
| Secret (S=638) | **160** (estimate; reuses the #6048 25% calibration sample where the round already drew it) | true *over-inclusion* rate < ~2% (secret tier's audit question is the converse: is mundane content inflating the most-protected tier, #6021) | Same seeded stratified sample as #6048 §3, so the two passes share review load; the converse question needs no separate draw |
| Dismissal re-review | 10% of the round's dismissals, minimum 20 (estimates) | reviewer-agreement signal | Catches drift, not leaks |

Worked reading: if the internal 300-sample finds 2 misses, the
point estimate is 2/300 ≈ 0.7% with a 95% Wilson interval of roughly
0.2%–2.4% (estimate; the run report computes it exactly) —
projected to at most ~2,600 suspect chunks corpus-wide, which sizes
the follow-on targeted pass. If it finds 0, the report claims
<1% miss rate at 95% confidence. Either outcome is a result; "no
misses found in n=300" without the bound is not.

### 4.3 Escalation

- **Any confirmed miss of the catastrophic class** (live credential,
  private key/token — L1's class) escalates immediately: the round is
  reported incomplete, the affected source class gets a full Tier
  A+B rescan, and the allowlist is frozen for that class until the
  miss's suppression path is explained.
- **Miss rate above 2%** (estimate threshold) in any tier's sample
  escalates the round to a full internal Tier B census regardless of
  the §3 calendar, and raises the next round's sample to 500
  (estimate) until two consecutive rounds clear below threshold.
- **Dismissal-overturn rate above 10%** (estimate) re-opens the
  round's queue and rotates reviewers.

All three thresholds are re-tuned after the first two rounds from
measured rates; the tuned values are recorded in the run manifest,
not in this document.

### 4.4 Cost honesty

The audit is the recurring human cost of the cadence and must stay
bounded or it will be skipped — a skipped audit is how the cadence
silently dies. 300 + 160 + dismissal-slice reads per round is the
ceiling this design defends: if a round's queue plus audit load
projects above what the review cadence can drain before the next
round opens (planning ceiling 500 queue candidates + 500 audit
reads per round, estimates carried from #6048 §4.2), thresholds are
raised and the round re-scoped *explicitly in the run report* —
never by quietly shrinking the sample.

## 5. Allowlist lifecycle — reason, expiry, no grandfathering

Every suppression of a finding lives as an allowlist entry. An
entry without a reason and an expiry is not an allowlist entry; it
is an unrecorded hole. Schema (fields normative; store shape is the
implementation follow-on's choice):

- `entry_id`, `created_at`, `created_by` (named reviewer);
- `scope`: the exact suppression predicate — rule/recognizer name +
  chunk-ID list or source-class + shard range. Class-wide
  suppressions (`all PERSON hits in shard X`) require a second
  reviewer's sign-off; bare recognizer-wide suppressions
  (`ignore SSN everywhere`) are forbidden.
- `reason`: why this finding is not a leak (e.g. "test fixture
  corpus, intentionally credential-shaped, source class
  `fixtures/`"), with a pointer to the evidence (run report,
  review-queue disposition ID).
- `expires_at`: mandatory, maximum 180 days (estimate; re-tune from
  observed churn). No permanent entries.
- `owner`: the party responsible for renewal review (defaults to
  the entry creator).
- `last_verified_at` / `last_verified_run_id`: updated by every T5
  sweep that re-examines the entry.

Lifecycle rules:

1. **Expiry forces re-examination, not auto-renewal.** At
   `expires_at` (T5), the entry's scope is rescanned
   *un-suppressed* under the current config. Renewal requires a
   fresh disposition citing the new run; the renewal records the
   new `run_id`. An entry that fails re-examination becomes a
   review-queue candidate; the tier-lock-upward rule (#6048 §5)
   applies while it is disputed.
2. **Config changes suspend affected entries.** On a T1 merge, every
   entry whose scope intersects the changed rules is marked
   `suspended-pending-revalidation` until the T1 round re-examines
   it. A suspended entry suppresses nothing.
3. **Grandfathering is the named failure.** The run report lists, per
   round: entries renewed, entries expired-into-candidates, entries
   suspended by T1, and the oldest live entry age. A round whose
   report shows the allowlist growing while the corpus grows is
   flagged for owner review — suppression expanding with the
   corpus is the shape of a leak being normalized.
4. **Findings are secret-handled.** Allowlist reasons reference
   chunk IDs + offsets with redacted previews only; no secret-class
   content in entries, reports, issues, or logs (#6021 handling
   rules, #6048 §5).

## 6. Where the cadence plugs into #6019 (standing gate)

#6019 owns ingestion operations; this cadence is one of its
standing gates, alongside the usefulness filter and the disk/IO +
plan-spend guardrails. The plug-in points (for the #6019 design to
adopt, not decided here):

1. **Staging merge gate.** A source class's staging batch (#5669)
   merges into a tier only with a clean staging scan under the
   current `config_hash` (L1+L2 full on the batch, L3 at §3
   scoping) *and* no live non-expired allowlist entry covering the
   batch's findings. The merge record cites the scan `run_id`.
2. **New-source-class onboarding.** #6019's "which steps stay
   human-approved" list gains: first-ingest of a new source class
   requires a T2 round (this design) before the class's tier rule
   is recorded in the manifest (#5410).
3. **Periodic gate.** The T4 calendar round is scheduled by the
   same scheduler that owns #6019's sweeps; its run report is
   attached to the ingestion-operations log, and a round reported
   incomplete (queue not drained, §4.3 escalation open) blocks bulk
   backfills until cleared — backfills during an incomplete round
   can bury a leak under new content.
4. **Plan-spend awareness.** The deterministic layers are compute-
   trivial; the Tier B census is the cost center (#6048 §4: 1–3 h
   off-peak on the fleet laptop, estimate). The T4 annual full
   census and any §4.3-escalated census are scheduled off-peak,
   niced, checkpointed — the same scheduling discipline #6019
   requires for ingestion itself.

## 7. Alternatives considered and rejected

1. **One-off scan only, no cadence.** Rejected: it certifies a
   moment. Rules improve and corpora re-tier — the body's named dead
   end. (If #6021's recurrence recommendation nevertheless says
   one-off, §8 states the fallback.)
2. **Rescan everything on every trigger (no scoping).** Rejected:
   the Tier B full census is a single-digit-hour job plus a review
   queue that can exceed drain capacity (#6048 §4.2). Scoping (§3)
   spends full censuses where inputs changed and samples elsewhere;
   the audit (§4) is what keeps the sampling honest.
3. **Audit the flagged findings only (no negative sampling).**
   Rejected: re-reading what the scanner already caught measures
   precision, not recall. The miss rate lives in the negatives; only
   sampling the un-flagged corpus estimates it.
4. **Fixed-size audit regardless of tier size.** Rejected: the
   secret tier (638 chunks) and the internal tier (107k) ask
   different questions (converse over-inclusion vs leak presence)
   at different scales; the shared 160-chunk secret sample is
   deliberate load-sharing, not a fixed fraction.
5. **Permanent allowlist entries with annual review.** Rejected:
   review-without-expiry is how leaks get grandfathered — the entry
   stays live while nobody looks. Mandatory `expires_at` with
   un-suppressed re-examination (T5) makes "nobody looked" expiry,
   not silent continuance.
6. **Allowlist keyed on content hash (suppress this exact string
   everywhere).** Rejected: content-keyed suppression follows the
   leak if the string is copied to a new chunk. Suppressions are
   scoped to rule + location/class (§5), so a leaked string
   appearing somewhere new is a fresh finding.

## 8. Relation to #6021 and #6019 — explicit fork/dependency check

- **#6021 (parent scan): forks nothing.** This record adopts #6021's
  corpus figures, handling rules, remediation path, and layer
  structure verbatim, and #6048's pass-level decisions as the NER
  layer's scoping. Two consequences (not changes): the T3 growth
  threshold and T4 calendar assume the wave-1 scan establishes the
  baseline counts this cadence deltas against; if wave 1 re-tiers or
  re-counts the corpus, the thresholds re-tune from the new baseline
  without altering this design.
- **If #6021 recommends one-off:** this cadence collapses to a
  single T1-configured round (triggers T2–T5 disabled, T4
  cancelled), the audit §4 runs once as the scan's confidence
  statement, and allowlist entries keep their expiry (a one-off
  scan with permanent suppressions is strictly worse than no
  scan). The #6019 plug-in points (§6) reduce to the staging merge
  gate alone. This fallback is recorded here so the decision, if
  taken, requires no redesign — only a scope note on this record.
- **#6019 (ingestion operations): touched at the seams only.** §6
  lists plug-in points *for the #6019 design to adopt*; this record
  decides nothing about #6019's trigger model, usefulness test, or
  guardrails beyond where the cadence connects. **#6049 (pattern
  pack) and #6050 (trufflehog ruling) are untouched siblings**: L4
  re-runs whatever pack #6049 defines, and the deterministic layer
  uses whichever scanner #6050 blesses — this cadence is
  scanner-agnostic by `config_hash`.
- **#5669 (staging) and #5410 (manifest)** are referenced as
  mechanics-owners (staging path) and rule-home (tier rules),
  respectively; neither is redesigned here.

## 9. Follow-on shape (not part of this record)

Implementation, when chartered under #6021/#6019, lands as:
(1) the run-manifest extension (round triggers, per-layer scope
flags, audit sample seeds, allowlist table — schema in §5);
(2) the scheduler wiring for T3/T4/T5 into #6019's operations;
(3) the allowlist store with expiry enforcement (suspend on T1,
un-suppressed re-examination on T5); (4) the audit sampler with
seeded stratified draws + Wilson-interval reporting; (5) the first
T4 round run as the pilot that calibrates every estimate labelled
above. Each step is separately reviewable; this design enables none
of them by itself.
