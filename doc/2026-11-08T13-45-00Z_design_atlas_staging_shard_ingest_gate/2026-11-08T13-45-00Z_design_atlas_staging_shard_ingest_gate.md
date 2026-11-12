# Atlas — staging-shard ingest gate: write → classify → dedup → provenance-check → publish

Design doc — 2026-11-08. Deliverable for #6057 (Follow-up 38, Cluster 7;
parent: Atlas ingestion program #6018). Design only; no implementation is
authorized by this note. Implementation is split into follow-ons in §9.

Companions and the decisions this note consumes rather than re-decides:

- staging tier, registry, aggregator, merge-on-close promotion — #5669
  (design #5667 §4; OPEN, not yet triaged)
- classification revalidation and tier semantics — #6021 (OPEN)
- ingest-time dedup, shingle overlap, recorded dedup edges — #5733 (OPEN,
  blocked)
- provenance: `chunk_occurrences`, CAS extracts, `repo@commit:path` —
  #3125 (OPEN); source-mark grammar/attestation — #6018 (OPEN)
- content versioning, lineage, retention — #6026 (design landed:
  `doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`)
- fan-out search consumer — #5755 (**CLOSED 2026-11-07**); its surface now
  lives in #6662 (live verification), #6664 (`--expand`), #6665 (filters +
  FTS), #6666 (serve-mode snapshot id), and the successor Worker + Durable
  Object cache #6788 (CLOSED) with eviction follow-up #6805

The #5755 closure is the one place this packet's brief is stale: the issue
text names #5755 as an open atomicity constraint. It is closed, but the
constraint did not disappear — it moved to #6666 (`ShardSet::snapshot_id()`
feeding the result-cache key) and #6788 (a cache that stores results
indefinitely). §6 addresses atomicity against those successors, not against
the closed issue.

State verified at `origin/main` `83b7a024b8` (2026-11-08): no `doc/` entry
covers the ingest gate; the Atlas design docs present are binary_service,
scaler_revision, manifest_filter_surface, license_capture_ingest,
content_versioning_retention, offset_matching, cross_tier_vector_comparability
and storage_growth_model. Issue #6057 is OPEN with no hard-stop labels.

---

## 1. Requirement in one paragraph

Every batch of Atlas content — a session close, a repo delta, a doc-tree
sweep, a chat-log import — must pass through a **gate** before any of it can
be seen by a search client: it is written to a staging shard, classified,
deduplicated, provenance-checked, and only then published to the live shard
set. A batch that fails any stage never touches the searchable corpus; a
batch that fails is quarantined with a machine-readable reason, not deleted
and not silently retried forever; and a retry of the same batch is safe
because landing is content-addressed. A newly executed fan-out must never
observe a half-published batch. A cache may still return an older, internally
coherent result until the separate cache-freshness work lands (§6.5); this
design does not mislabel that stale hit as current.

This is lakeFS's **write-audit-publish** pattern applied to Atlas's existing
staging tier. The gate is not a new storage system: #5669 already defines
the staging DB, the internal registry (unit → staging DB, owner, sensitivity
level, state, schema version) and the aggregator that promotes on unit
close. The gate is the **contract** that promotion runs under.

## 2. Vocabulary and the unit of work

- **Batch** — the unit the gate judges. In #5669 terms a batch is a
  *staging unit*: one queue task, session, epic or notes batch, identified by
  its unit id. A long-lived epic unit promotes in **increments** (§5.4), and
  each increment is judged as its own batch.
- **Staging shard** — the unit's staging DB (§5669 `atlas_stage` v1:
  `items`, `chunks_raw`, `notes`, `kv`, `_schema_version`). Nothing in it is
  searchable; it is the producer's only write target. The gate writes
  publish candidates separately and commits visibility through the catalog.
- **Live shard set** — the committed `atlas-core-<tier>-<topic>-<nn>` scan
  manifests plus the per-tier content stores (#5755 layout, authoritative
  map on #5669). Candidate writes are separate; publish changes visibility
  only by committing the catalog generation.
- **Gate record** — the durable, per-batch judgement: batch id, stage
  reached, verdict per stage, reason codes, counts, content hashes, the
  publish manifest id if published. One row per batch in the internal
  registry DB; it is the audit trail the gate exists to produce.
- **Verdict** — per stage, exactly one of `pass`, `fail`, `skip`
  (`skip` only where §4 says a stage is not applicable, and the reason must
  be recorded — an unexplained skip is a gate failure, not a pass).

The gate is **fail-closed**: a batch with no verdict for a stage has not
passed it. Absence of a record is never interpreted as success.

## 3. The five stages, as contracts

Each stage is a pure function of the batch plus the stage's inputs, and
returns `(verdict, reason_codes, counts, artifacts)`. Stages run in order;
a `fail` at stage *n* sends the batch to quarantine (§7) and stages
*n+1…5* do not run.

### 3.1 Stage 1 — write

**Contract.** The producer writes rows only into the unit's staging DB,
never into a live shard or the production libSQL monolith. Every row
carries: the unit id, the schema version, the source locator(s) it came
from, and the content hash of its bytes.

**Passes when** all of:

- the staging DB exists, was provisioned by the provisioning tool, and its
  `_schema_version` is one the aggregator supports;
- the unit is registered and in state `open` (or `closing` for a close
  increment) — an unregistered staging DB is a fail, not an implicit unit;
- every written row has a non-empty source locator and a content hash;
- the row count is non-zero (an empty batch is `skip`-with-reason, not
  `pass` — it must not bump a snapshot id).

**Fails when** the staging DB is unregistered, schema version unsupported,
any row lacks a locator or hash, or a write targeted a live shard (the
producer bypassed the gate — this is a **hard fail** and quarantines the
batch even if the rows look valid).

**Does not decide:** what the source set is, how deltas are computed, or
how a unit is created. Those are #5669 and the #6018 ingestion-operations
design.

### 3.2 Stage 2 — classify

**Contract.** Every row in the batch is assigned exactly one sensitivity
level — `public`, `internal`, or `secret` — before anything can be
published. The level is a property of the row, not of the batch: a batch
that contains both internal and secret rows is split before publish
(§5.2), never published wholesale at the batch's maximum level.

**Passes when** every row has a level, the level is one of the three, and
the classifier's own confidence is at or above the acceptance threshold for
the level it assigned. A row the classifier cannot place at acceptable
confidence is **not** defaulted to `internal`: it fails the stage.

**Fails when** any row is unclassified, carries an unknown level, or falls
below the confidence threshold. Reason codes name the row ids and the
failing class, never the row content (see §8).

**Does not decide:** the classifier's implementation, its thresholds, or
the tier content profile. Those belong to #6021 and the classification
manifest work (#5410). This design fixes only the gate's obligation: no
unclassified row reaches publish, and low confidence is a fail rather than
a guess.

**Standing rule consumed:** secret material exists only in the secret DBs
(#6021, #6018). The gate enforces it mechanically at publish (§5.2); the
revalidation scan (#6021) is what proves the existing corpus still obeys it.

### 3.3 Stage 3 — dedup

**Contract.** Every row is checked for exact and near duplicates. Exact
duplicates are byte-identical content (same content hash). Near duplicates
are shingle-set overlap at or above the #5733 threshold (≈0.9 Jaccard).

**Passes when** every row has a dedup disposition, one of:

- `new` — no exact or near duplicate found;
- `exact-duplicate-of:<chunk_id>` — same content hash as an existing chunk;
- `near-duplicate-of:<chunk_id>@<score>` — an edge recorded, not a deletion.

**Fails when** a row has no disposition, or when the dedup stage would
have to *delete* something to proceed — deletion on a similarity score is
forbidden (#5733: "dedup findings are data, not deletions").

**Semantics of each disposition at publish:** `new` publishes normally.
`exact-duplicate-of` does **not** create a second chunk; it writes an
occurrence addition into the batch's hidden candidate overlay. The catalog
commit makes that occurrence visible with the rest of the tier publication
(#3125 rule 3: every chunk records every place it was found). This is the
gate's answer to the historical defect where dedup silently dropped ~127K
occurrences.
`near-duplicate-of` publishes the chunk **and** the edge; promotion policy —
not the gate — decides whether a near-duplicate is suppressed, so a
suppressed near-duplicate must still leave its occurrence recorded.

**Does not decide:** shingle size, hashing, storage cost, or the exact
threshold. Those are #5733's design note.

### 3.4 Stage 4 — provenance-check

**Contract.** Every row that will become a chunk carries (a) a parseable
source mark and (b) an attestation, and its content is resolvable back to
the bytes it came from.

**Passes when** for every row:

- the source mark parses against the grammar from the #6018 / #3125 family
  (`repo@hash:path` for git sources; source-system marks such as
  `opencode-db @ amd @ <date>` for non-git sources);
- the content is interned content-addressed (a CAS hash that resolves) or
  pinned as `repo@commit:path` with spans recorded (#3125 rules 4–5);
- an attestation is present and names the source, the observation time, and
  the ingest run;
- the occurrence list is non-empty and every occurrence has a span and an
  `observed_at`.

**Fails when** any mark is unparseable, any content hash does not resolve,
any attestation is missing or malformed, or any occurrence lacks a span or
timestamp. A batch may not publish a chunk it cannot point back to.

**Does not decide:** the grammar itself or the attestation format. This
design fixes the gate's obligation: **no chunk publishes without a
resolvable mark and an attestation**, and the check is mechanical (parser +
CAS lookup), not a human review.

### 3.5 Stage 5 — publish

**Contract.** Build immutable, batch-scoped candidate artifacts in bounded
transactions, verify them, then make them visible with one atomic
per-tier catalog commit (§6). Publish is the only stage that changes the
search-visible catalog; it never writes into an already-active shard.

**Passes when** all of:

- every surviving row was promoted exactly once — `rows_in == rows_promoted
  + rows_deduped_into_occurrences + rows_quarantined_at_publish`, and the
  identity holds by count **and** by content hash;
- all candidate chunk rows and exact-duplicate occurrence additions were
  written to immutable, not-yet-visible batch artifacts; no active shard or
  visible occurrence set was mutated;
- candidate writes stayed inside the ≤500-rows-per-transaction bound
  (#5669); the active shard set is never held under a long writer;
- candidate manifests (unit id, tier, rows in/promoted, content hash,
  schema/model identity) and a gate record were durable before one atomic
  catalog commit made their complete manifest set visible (§6);
- the new per-tier snapshot id was computed from that committed manifest
  set and emitted in the merge manifest for #6666 / #6805;
- the tier split placed every row in the shard set its classification
  allows, and no secret row touched an internal or public shard.

**Fails when** the count/hash identity does not hold, a transaction exceeds
the bound, a candidate is visible before commit, the catalog commit is
missing, the snapshot id does not match the committed manifest set, or a
tier boundary was crossed. A publish-stage failure is the most serious
class: it is quarantined with `publish_partial` (§6.4); candidate artifacts
remain invisible while reconciliation runs (§6.4).

## 4. Per-stage pass/fail semantics, in one table

| stage | pass | skip allowed when | failure class |
|---|---|---|---|
| write | registered unit, supported schema, every row has locator + hash, non-empty | batch empty | `write_invalid` / `write_bypass` |
| classify | every row has one of three levels at acceptable confidence | never | `classify_unresolved` |
| dedup | every row has a disposition; no deletion required | never | `dedup_undispositioned` |
| provenance-check | every mark parses, content resolves, attestation present, occurrences spanned | never | `provenance_unresolvable` |
| publish | count + hash identity, bounded candidate writes, hidden prepare, atomic catalog commit, committed snapshot id, tier split | tier unit had zero survivors after dedup suppression | `publish_partial` |

`skip` is never a pass and never advances a snapshot id. Only the write
stage (empty batch) and publish (zero survivors) can be skipped, and each
requires a recorded reason.

## 5. Failure and quarantine handling

### 5.1 Quarantine is a state, not a deletion

A failed batch moves to state `quarantined` in the registry. Its staging DB
is retained (branched as an archive, per #5669's archive behaviour) and its
gate record carries the failing stage, the reason codes, and the row ids.
Nothing in the batch is published, and nothing in it is deleted.

### 5.2 Tier split happens before publish, not at classify

Classification assigns per-row levels; the gate splits the original staging
unit into **tier-scoped publication units** before Stage 5. Each tier unit
gets separate candidate artifacts, a catalog commit and a snapshot id. A
mixed-tier source batch is therefore not one cross-tier transaction: each
tier is atomic independently, and a query reports the snapshot id for each
tier it was authorized to search. This preserves the security boundary — a
secret-capable client can never make an internal shard write reachable —
without claiming a cross-database atomic transaction the storage system
does not provide.

The search-visible catalog is the authority for visibility, not physical
database enumeration alone. A candidate may exist in the Turso organization
and carry an `atlas-core-*` name, but fan-out must exclude it until the
catalog commit references its manifest. This is a required extension to
#5669/#6666, not behavior claimed to exist today.

### 5.3 Retry is safe because landing is content-addressed

Re-running a quarantined batch re-enters at the **failing stage**, not at
write, unless the failure was `write_invalid` or `write_bypass` (in which
case the producer must fix and re-write). Because content is
content-addressed (#3125), a retry that re-lands identical bytes is a no-op
for those bytes; a retry that lands changed bytes is a new version under
#6026's versioning model, not an overwrite. A retry never duplicates an
occurrence: occurrences are keyed by (chunk, source locator, span,
observed_at).

### 5.4 Long-lived units promote in increments

An epic unit promotes incrementally. Each increment is a batch with its own
gate record; a failed increment quarantines only that increment, and the
unit stays `open`. This is why the gate judges batches, not units: one bad
increment must not stall an epic, and must not be able to publish the rest
of the epic unreviewed.

### 5.5 Quarantine is bounded

A batch quarantined on the same reason code three times escalates to
`needs-review` and stops auto-retrying. Infinite retry of a deterministic
failure is a spend leak (#6018 S3 ceiling), not resilience.

## 6. Publish atomicity against fan-out search

### 6.1 The reader-visible invariant

**A newly executed fan-out observes either the pre-commit or post-commit
manifest set for each tier-scoped publication unit, never a mixture.** The
gate does not claim a cross-tier transaction: the per-tier snapshot ids in a
response make that boundary explicit. A cached result may be stale; that is
a separate freshness contract (§6.5), not permission for a live fan-out to
read a partially published batch.

### 6.2 What #5755's closure changes

#5755 is closed; its atomicity surface is now three separate mechanisms:

1. **`ShardSet::snapshot_id()`** — currently specified as sha256 over sorted
   shard name/tier/chunk count (slice 1, #6666). That physical-inventory
   digest is not an atomic commit point: counts can change while bounded
   writes are still in progress. The gate requires it to be derived from
   the committed catalog generation (sorted published manifest ids, tier,
   counts and content hashes); physical inventory is only candidate
   discovery.
2. **Result-cache keying** — the snapshot id must feed the #5668 result-cache
   key so a re-shard invalidates correctly (#6666 item 2).
3. **The Worker + Durable Object cache** (#6788, closed) — stores results
   **indefinitely** with no TTL in v1, keyed on
   `sha256(normalized_query || config_fingerprint)`. Its eviction and
   invalidation policy is the open follow-up #6805.

The third is where freshness is at risk, and this design states it plainly:
**a cache that never expires and is not keyed or invalidated by snapshot id
may serve pre-batch results indefinitely.** The gate cannot fix that from
the publish side. It can guarantee that a fresh fan-out reads one committed
snapshot, and emit the id for cache work to consume; it cannot claim every
cache hit is current. §6.5 records that limit.

### 6.3 Prepare/commit visibility protocol

- **Prepare:** write each tier's immutable candidate shards and occurrence
  additions in ≤500-row transactions; keep them outside the active manifest
  set. Write and verify all candidate manifests and the gate record. Retries
  are idempotent by batch id + content hash.
- **Commit:** in one transaction on the tier's catalog/registry, append a
  commit record that references the complete candidate-manifest set and
  advances that tier's active generation. This catalog commit is the single
  visibility point. An incomplete prepare has no commit record and remains
  invisible; no multi-DB transaction is assumed.
- **Read:** the fan-out resolves the active catalog generation once at query
  start, pins its committed manifest list and snapshot id, then scans only
  those manifests. It must not treat every physical `atlas-core-*` database
  as searchable merely because it exists. A query spanning multiple tiers
  carries one pinned snapshot id per authorized tier.
- **Snapshot:** hash the committed manifest ids, tier, counts and content
  hashes (or an equivalent generation id), not live mutable row counts.
  Publish returns/emits this id in its merge manifest; #6666's
  `ShardSet::snapshot_id()` and cache integration must consume the committed
  generation rather than independently observing in-progress writes.

This is a **new gate requirement**: the current #5669 aggregator description
promotes into core in bounded batches and bumps a snapshot id per batch, but
does not specify a hidden candidate set or one catalog commit point. The
implementation must add that visibility boundary before it can claim
atomicity. If the chosen registry/catalog cannot make the active-generation
update atomic, use an equivalent single-record pointer flip; do not weaken
the invariant to "bump the hash after the writes".

### 6.4 Reconciliation after a partial prepare/commit

If preparation fails mid-batch (`publish_partial`), some candidate rows may
exist, but without the catalog commit they remain invisible. Reconciliation
re-reads candidate manifests and content hashes for the batch id, then
completes missing candidate writes or quarantines mismatched artifacts and
re-lands from staging. Only after the count/hash identity in §3.5 holds may
the catalog commit occur. If commit acknowledgement is lost, read the
catalog first: if the commit record exists, report success; otherwise retry
the same idempotent commit. Never infer visibility from physical row counts.

### 6.5 Cache freshness is separate from fan-out atomicity

The Worker + Durable Object cache (#6788) stores results indefinitely and
its key `sha256(normalized_query || config_fingerprint)` does not currently
include the shard snapshot id. It may therefore return a coherent but old
result indefinitely. This does **not** mean a fresh fan-out can see half of
a batch; it means the system does not yet promise that a cache hit sees the
latest batch. #6805 owns invalidation/eviction, and #6666 owns wiring the
committed snapshot id through the serve path. Until those land, the response
should expose the snapshot id that produced it where the current envelope
allows, and documentation must not label the DO cache fresh or atomic.

## 7. What this design does not decide

- The staging schema, provisioning tool, registry columns, aggregator
  implementation, batch size tuning, archive mechanics — #5669. The
  prepared/published catalog boundary and per-tier generation are additional
  requirements this gate sends back to #5669.
- Classifier implementation, thresholds, tier content profile, the
  revalidation scan — #6021 and #5410.
- Shingle construction, hashing, storage cost, the exact Jaccard threshold,
  the rescore formula — #5733.
- The source-mark grammar and attestation format — #6018 / #3125.
- Version identity, lineage, retention and cold rollout — #6026 (landed).
- Cache eviction policy and serve-mode wiring — #6666, #6805.
- Filters and FTS on the shard path — #6665.
- The ingestion source set, delta computation, disk/IO and spend guardrails
  — the #6018 ingestion-operations design.

**Sibling decisions this note does not silently re-decide.** It consumes
#6026's versioning model unchanged (a changed file is a new version, not an
overwrite; a retry lands a new version). It consumes #3125's keep-all rule
unchanged (dedup adds occurrences, never deletes). It consumes #5733's
"findings are data, not deletions" unchanged. It consumes #5669's ≤500-row
transaction bound and merge-on-close promotion unchanged. Where this note
adds obligations those designs did not state — per-tier candidate artifacts
and an atomic catalog commit (§6.3), the per-row tier split (§5.2),
snapshot ids derived from committed manifests (not live counts), the
snapshot-id-in-manifest requirement, and the three-strikes quarantine bound
(§5.5) — those are explicitly new gate requirements to reflect back into
#5669 and #6666; they are not claimed to exist today and are not silent
changes to sibling decisions.

## 8. Logging and sensitivity

The gate record is written at the sensitivity of its batch: a secret-tier
batch's gate record names counts, classes, row ids and locations, never
content. Reason codes are enumerated strings plus ids, so a quarantine
report can be read and acted on without reproducing the material that
caused it (#6021's handling rule, applied to the gate).

## 9. Implementation split (follow-ons)

The gate design is finished when the contracts above are accepted. Its
implementation is split so each part lands against the sibling that owns it:

1. **Gate record + state machine** (depends on #5669 registry) — the
   per-batch gate record, the `open → closing → gated → published |
   quarantined` transition set, and the fail-closed "no verdict = not
   passed" rule.
2. **Stage 1–2 wiring** (depends on #5669, #6021) — write contract checks
   and the classify gate, including the no-default-to-internal rule.
3. **Stage 3 wiring** (depends on #5733) — dedup dispositions and the
   occurrence-on-exact-duplicate behaviour, which is the fix for the
   dropped-occurrence defect described in #3125.
4. **Stage 4 wiring** (depends on #6018 grammar, #3125 CAS) — the
   mechanical mark/attestation/CAS check.
5. **Stage 5 wiring + atomicity** (depends on #5669 aggregator, #6666) —
   immutable hidden candidate shards/occurrence overlays, the count/hash
   identity, per-tier catalog commit, snapshot derived from committed
   manifests, the manifest carrying the snapshot id, and §6.4 reconciliation.
6. **Quarantine + retry** — the quarantine state, resume-at-failing-stage
   retry, and the three-strikes escalation to `needs-review`.

Each follow-on is filed against the sibling it depends on, so the gate is
built as those siblings land rather than as one large change. The
acceptance for the whole gate is the #5669 end-to-end demo extended with a
deliberately failing batch that never reaches a search client.

## 10. Open questions

1. **Does the classifier's confidence threshold live in the gate or in
   #6021?** Default taken here: the gate consumes a threshold #6021
   publishes; until then the gate fails any row the classifier marks
   uncertain. Recorded rather than blocked.
2. **Should a `near-duplicate-of` edge suppress publication by default?**
   Default taken here: no — the edge is recorded and the chunk publishes,
   because #5733 says suppression is a promotion-policy decision. If the
   policy later suppresses, the occurrence is still recorded.
3. **How long is a quarantined staging DB retained?** Default taken here:
   until the three-strikes escalation resolves, then archived under
   #6026's retention rules. Not a gate decision; recorded for #5669.
4. **Does the DO cache need a snapshot-id dimension in v1, or can it wait
   for #6805?** Default taken here: wait for #6805, with the staleness
   window recorded in §6.3 rather than hidden.

---

Filed as the design record for #6057. The implementation follow-ons in §9
are the tracked next actions; the gate is not "done" until at least the
gate-record state machine (follow-on 1) exists as an issue against #5669.

**Tracked next actions.** Follow-on 1 (gate record + state machine) is
filed as #7709 against #5669. Follow-ons 2–6 are filed against their
siblings as those siblings land (#6021, #5733, #6018/#3125, #5669/#6666).
