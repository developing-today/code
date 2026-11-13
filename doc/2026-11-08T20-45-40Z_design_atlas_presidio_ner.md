# Design: Atlas Presidio NER pass — recognizer set, tier scoping, cost, review queue

- **Issue:** #6048 (parent #6021, classification revalidation scan)
- **Status:** design record. No Presidio integration code is written or
  enabled by this document; implementation is a follow-on under #6021.
- **Inputs consumed, not re-defined:** the #6021 scanner design and its
  handling rules (findings handled at the secret tier's sensitivity;
  findings land as quarantine candidates — tier-lock upward, never
  silent delete); the #6048 finding summary (Presidio Analyzer =
  regex + checksum validators + spaCy NER + context words; 40+ entity
  types; checksum-backed recognizers are the reliable tier; names and
  some country recognizers false-positive without context).
- **Source note (required by task 13549's packet):** the prior-art
  source named in the #6048 body
  (`~/workspace/scratch/atlas-ingestion-brief-2026-11-04/prior-art-research.md`)
  is absent on the fleet laptop. This record is written from the
  finding summary carried in the #6048 body itself, plus the corpus
  figures stated in #6021. Where a number is an estimate rather than a
  cited figure, it is labelled an estimate with its assumptions.

## 1. Problem

Atlas is tiered: internal (28 shards / 107,001 chunks) vs secret
(exactly 1 shard / 638 chunks; secret material exists only in the 2
secret DBs — #6021). The gitleaks-class regex pass over this corpus is
cheap. The Presidio NER pass is the expensive layer, and it is also
the layer that can catch what regex cannot: personal data in prose
(names attached to addresses, account references without checksum
structure). #6021's revalidation exists to catch a mis-tiered secret
sitting in an internal shard; this record decides how the Presidio
pass is scoped so that (a) the internal tier — the broadly-queried
tier, where a leak does damage — gets the full pass, (b) the secret
tier gets a calibration sample rather than a redundant census, and
(c) every finding lands in a review queue shaped so a false positive
costs a review, never a deletion.

## 2. Recognizer set — two tiers of evidence

Presidio recognizers are not one class of evidence. The design splits
them by what backs the match, because the backing decides what a
finding is allowed to do.

### 2.1 Tier A — checksum / structure-backed (deterministic)

A match carries its own proof (a valid checksum or a rigid structure),
so Tier A findings are high-precision and may be recorded as
confirmed-class findings without NER corroboration.

| Recognizer                       | Backing                           | Scope decision                                                                                 |
| -------------------------------- | --------------------------------- | ---------------------------------------------------------------------------------------------- |
| Credit card                      | Luhn checksum                     | ON, all tiers                                                                                  |
| IBAN                             | mod-97 checksum                   | ON, all tiers, all countries (checksum makes locale FP risk negligible)                        |
| US SSN / ITIN                    | structure + issuance rules        | ON                                                                                             |
| US bank account / routing        | structure                         | ON                                                                                             |
| US passport, US driver licence   | structure                         | ON                                                                                             |
| UK NHS number                    | checksum (mod-11)                 | ON                                                                                             |
| Email, IP address, URL           | rigid structure                   | ON (recorded; PII signal only in combination — an email alone is not a quarantine trigger, §5) |
| Crypto wallet (BTC/ETH patterns) | structure (+ checksum for bech32) | ON                                                                                             |

Country recognizers beyond US/UK are **OFF by default**: ES NIF, IT
fiscal code / VAT / driver licence, PL PESEL, SG NRIC/FIN, AU
ABN/ACN/Medicare/TFN, IN Aadhaar/PAN, FI HETU, KR RRN and similar.
Rationale, straight from the #6048 finding: several country
recognizers carry high false-positive risk absent context words, and
the corpus is a US-centric personal/fleet corpus. A country
recognizer is enabled per-source-class only when that source's
provenance indicates the locale, and then it runs in Tier B mode
(context required, §2.2) for its first calibration batch regardless of
any checksum it may carry. IBAN is the exception: its checksum is
locale-independent, so it stays Tier A globally.

### 2.2 Tier B — NER + context (probabilistic)

spaCy NER entities and pattern recognizers whose backing is a name or
a shape that ordinary prose also produces:

- Entities: PERSON, ORGANIZATION, LOCATION (spaCy NER via Presidio).
- Pattern-only recognizers with no checksum: phone number, and any
  per-locale recognizer enabled under §2.1's provenance rule.

Tier B findings are **candidates, never confirmations**. A Tier B
candidate is emitted only when at least one of these holds:

1. the Presidio context enhancer finds a supporting context word
   within the recognizer's context window (e.g. "passport", "SSN",
   "account", "patient") raising the score above the recognizer's
   base threshold; or
2. the raw NER score is ≥ 0.85 (estimate threshold, to be calibrated
   in the pilot, §4.3) **and** a second, independent signal co-occurs
   in the same chunk (a Tier A hit, or a second Tier B entity of a
   different type — a PERSON next to an address-shaped LOCATION is
   evidence; a lone PERSON in prose is usually a name in a document,
   which the internal tier legitimately contains).

A lone sub-threshold Tier B hit is counted in batch metrics (so the
pass can be tuned) but does not enter the review queue. This is the
mechanism that keeps the queue reviewable at corpus scale (§4.2).

### 2.3 Explicitly not PII signals

DATE_TIME, and ORGANIZATION/LOCATION alone, never trigger quarantine.
They are recorded only as co-occurrence signals for §2.2 rule 2.

## 3. Tier scoping — full internal, sampled secret

| Tier                      | Tier A pass    | Tier B (NER) pass                      | Why                                                                                                                                                                                                                                                                                       |
| ------------------------- | -------------- | -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| internal (107,001 chunks) | 100%           | 100%                                   | This is the tier a leak damages; #6021's question is presence, and presence needs a census, not a sample.                                                                                                                                                                                 |
| secret (638 chunks)       | 100%           | **25% stratified sample** (160 chunks) | PII in the secret tier is expected and already handled at the highest sensitivity; a finding there changes no tiering decision. The sample's job is calibration: measuring Tier B precision/recall behaviour on known-sensitive text to tune the thresholds the internal pass depends on. |
| staging / new ingestion   | 100% at ingest | 100% at ingest                         | Cheapest point to catch a mis-tier: before the chunk joins a tier (relates to #6021's staging-path check; mechanics owned by the implementation follow-on).                                                                                                                               |

Secret-tier sampling design: stratified by source class (the #6021 /
manifest source classes), proportional allocation, fixed seed recorded
in the run manifest so the sample is reproducible and the next
revalidation can rotate to a disjoint sample. 25% of 638 = 160 chunks
is large enough that a recognizer behaviour seen in the sample at
≥2% prevalence is unlikely (≤ ~4% probability, binomial bound,
estimate) to be absent from the sample if present in the tier — and
small enough that the calibration review load is bounded (§5).
Escalation rule: if the sample's Tier B candidate rate or any
confirmed Tier A surprise (a checksum-valid hit class not previously
known in the tier) exceeds the pilot's expectations, the sample
expands to 100% for that source class — the decision is recorded in
the run report, not taken silently.

## 4. Cost at corpus scale

### 4.1 Model

Cost is dominated by Tier B (spaCy NER). Tier A is regex + checksum
arithmetic and is at least an order of magnitude cheaper per chunk;
it is not the budget driver and is not modelled separately beyond a
flat allowance.

| Symbol | Meaning                        | Planning value (estimate)                                                                                                                                                                |
| ------ | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `N`    | internal chunks                | 107,001 (cited, #6021)                                                                                                                                                                   |
| `S`    | secret chunks sampled          | 160 (§3)                                                                                                                                                                                 |
| `r`    | NER throughput per core        | 2–6 chunks/s for typical Atlas chunk sizes with a mid-size spaCy English model (estimate; the pilot in §4.3 replaces this with a measured fleet number before the full run is scheduled) |
| `c`    | cores used on the fleet laptop | 8 (estimate; leaves the machine usable, job runs niced and off-peak)                                                                                                                     |

Wall-clock estimate for the full internal NER pass:
`N / (r × c)` = 107,001 / (2–6 × 8) ≈ **0.6–1.9 hours** of dedicated
wall-clock; with Tier A, I/O, and checkpoint overhead (×1.5
allowance), **plan a 1–3 hour off-peak window** for the internal
census, ~5 minutes for the secret sample, and seconds-to-minutes for
Tier A alone. Storage of findings is negligible next to corpus
storage (one row per candidate; §4.2 bounds the count).

These are planning estimates, not measurements. The design's claim is
the shape — a single-digit-hour, one-machine, resumable batch job —
not the second digit.

### 4.2 Review-load bound

The real cost of an NER pass is reviewer attention, not CPU. If Tier B
emitted every raw hit, names in prose would flood the queue. §2.2's
emission rules are the bound: only context-supported or corroborated
candidates queue. The pilot (§4.3) must measure the candidate rate;
the full run proceeds only if the pilot's rate projects to a queue
the review cadence can drain (planning ceiling: 500 candidates for
the initial census — estimate; above that, thresholds are raised and
the pilot re-run before any census starts).

### 4.3 Pilot before census (mandatory)

1. Run both tiers over a fixed 1,000-chunk stratified pilot drawn
   from the internal tier plus the 160-chunk secret sample.
2. Measure: per-core throughput (replaces `r`), Tier B candidate rate,
   Tier A hit rate, reviewer agreement on a 50-candidate audit slice.
3. Calibrate §2.2's score threshold and §4.2's ceiling from the
   measurements; record them in the run manifest.
4. Only then schedule the full internal census.

### 4.4 Batching and checkpointing

- Work unit: one shard at a time; batch = 500 chunks within a shard.
- Each batch commits a checkpoint row to the run manifest:
  `(run_id, shard, chunk_range, recognizer_config_hash,
spacy_model_version, presidio_version, batch_started_at,
batch_finished_at, counts_by_recognizer_and_tier)`.
- The recognizer configuration (enabled set, thresholds, context
  word lists) is content-hashed into every checkpoint and every
  finding. A config change starts a new run_id; runs are never
  silently mixed.
- Resume is idempotent: findings are keyed
  `(chunk_id, recognizer, span_start, span_end, config_hash)` and
  written with insert-or-ignore semantics, so a crashed batch can be
  re-run without duplicating queue entries.
- A run is complete when every shard has a finished checkpoint for
  the current config_hash; the run report is generated from the
  manifest, not from logs.

## 5. False-positive review queue

Every emitted candidate lands in one queue with exactly one initial
state: **quarantine candidate**. The queue record carries:

- chunk ID, shard, current tier;
- recognizer name and entity type, Tier A or Tier B backing;
- Presidio score and, for Tier B, the context words that fired and
  the co-occurring signal ( §2.2) — recognizer + context attached, as
  #6048 requires;
- the matched span's character offsets and a redacted span preview
  (findings are handled at secret-tier sensitivity per #6021: the
  full chunk text is read only inside the secret handling path,
  never reproduced in the queue UI, this repo's issues, or logs);
- config_hash and run_id (§4.4).

Dispositions (each is a recorded act by a named reviewer — human or
agent acting as reviewer — with timestamp and rationale):

| Disposition            | Effect                                                                                                                                                                                                                                      |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| confirm                | The chunk is **tier-locked upward**: reclassified to secret handling (move + re-embed at the secret tier and purge from the internal tier incl. replicas, per #6021's remediation path, filed as a follow-on if not done in the same pass). |
| dismiss-false-positive | Chunk stays at its tier. The dismissal, with recognizer + context, feeds threshold calibration (§4.3) — dismissals are tuning data, not waste.                                                                                              |
| escalate               | Reviewer cannot decide from the record; chunk is provisionally tier-locked upward until a decider rules. Uncertainty resolves toward the safer tier.                                                                                        |

Hard rules of the queue:

- **Never silent delete.** No disposition deletes chunk content.
  Confirm moves and purges the _wrong-tier copy_ only as part of the
  recorded remediation; dismiss and escalate change no content.
- **Tier-lock only moves upward.** The queue can raise a chunk's
  handling to secret; lowering a tier is out of scope for this pass
  and for the queue entirely.
- **No auto-confirm for Tier B.** Tier A checksum hits may be
  recorded as confirmed-class in metrics, but the tier-lock act
  itself is always a disposition by a reviewer for the initial
  census; automation of Tier A confirms is a separate decision after
  the first run's precision is measured.
- Queue drain is part of the run: a census whose queue is not drained
  to zero-or-escalated within the revalidation cadence is reported as
  incomplete in the #6021 run report.

## 6. Alternatives considered and rejected

1. **NER on every tier at 100%, including secret.** Rejected: it
   spends the scarcest resource (reviewer attention on the most
   sensitive content) to answer a question the secret tier does not
   pose — presence there is expected and changes nothing. The 25%
   calibration sample buys the information the internal pass actually
   needs.
2. **No NER at all (regex/checksum only).** Rejected: Tier A cannot
   see personal data in prose — the exact class #6021's
   vector-neighborhood and sample checks exist to hunt. A
   checksum-only pass would certify the internal tier against a
   threat model narrower than the one #6021 states.
3. **Full Presidio pass over the secret tier as the calibration
   source instead of a sample.** Rejected for the reason in (1);
   §3's escalation rule recovers the full pass for any source class
   where the sample surprises.
4. **All country recognizers on.** Rejected (§2.1): high FP risk
   absent context, in a corpus whose provenance is US-centric;
   provenance-gated enablement keeps the capability without the
   flood.
5. **Auto-quarantine on any Tier B hit.** Rejected: with names in
   prose, that tier-locks legitimate internal content at scale and
   trains reviewers to bulk-dismiss — the failure mode the
   two-signal emission rule (§2.2) exists to prevent.
6. **External hosted PII API instead of local Presidio.** Rejected:
   the corpus's sensitive tiers cannot be sent to a third-party
   processor; the pass runs on fleet hardware, on local models,
   which is also what makes the §4 cost model a laptop-hours
   question rather than a spend question.

## 7. Invariants implementation must hold

1. The pass is read-only against corpus content. Its only writes are
   findings, checkpoints, and queue records. Tier changes happen
   only through reviewer dispositions (§5).
2. No finding, log, report, or queue view reproduces secret-class
   content; spans are referenced by chunk ID + offsets with redacted
   previews, and handled at secret-tier sensitivity (#6021).
3. Every finding is reproducible: it names the recognizer, the
   config_hash, and the model versions that produced it (§4.4).
   A finding that cannot be re-derived is a defect, not a result.
4. Tier-lock moves chunks upward only; nothing in this design lowers
   a classification or deletes content.
5. The secret tier's boundary is unchanged: secret material exists
   only in the 2 secret DBs before and after the pass; a confirm
   disposition moves a chunk _into_ that boundary, it never widens
   it.
6. Sampling is seeded and recorded; two runs of the secret-tier pass
   with the same seed and config examine the same chunks.
7. The pass must be safe to interrupt at any batch boundary and
   resume without duplicate findings or lost checkpoints (§4.4).

## 8. Relation to #6021 — explicit fork check

Acceptance requires any answer that would change a #6021
scanner-design decision to be stated explicitly. **This record forks
nothing from #6021.** It adopts #6021's handling rules verbatim
(quarantine candidates, tier-lock upward, never silent delete,
secret-tier sensitivity for findings) and its corpus figures, and it
supplies the pass-level decisions #6048 asked for (recognizer set,
scoping, cost, queue shape) inside that frame. Two clarifications
that are _consequences_ of #6021, not changes to it:

- The sampled secret-tier pass does not weaken #6021's converse
  boundary check (secret contents are actually secret-class): that
  check is a separate #6021 work item; this pass's sample only
  calibrates recognizers, and §3's escalation rule can widen it.
- Staging-time scanning (§3, third row) is stated as a scoping
  decision of this pass; the staging path's own mechanics remain
  owned by #5669 as #6021 already records.

Siblings untouched, per scope: the classification revalidation
cadence (#6051) and the personal-inventory pattern pack (#6049) are
separate designs; this record's calibration outputs (measured
precision/recall per recognizer) are available to both but decide
nothing for them.

## 9. Follow-on shape (not part of this record)

Implementation, when chartered under #6021, lands as: (1) the pilot
harness + manifest schema (§4.3, §4.4), (2) the queue store + review
surface (§5), (3) the pilot run and threshold calibration, (4) the
internal census + secret sample, (5) the run report linked from
#6021 with queue drain status. Each step is separately reviewable;
this design enables none of them by itself.
