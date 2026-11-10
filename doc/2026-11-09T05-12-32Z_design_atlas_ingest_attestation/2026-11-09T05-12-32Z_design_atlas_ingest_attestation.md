# Atlas — ingest attestation record: in-toto-shaped per-batch statement binding chunks to sources

Design doc — 2026-11-09. This note is the deliverable for #6045
(ingest attestation record), filed under parent #6019
(semi-automated ingestion operations). Provenance substrate: #3125
(provenance registry — chunk occurrences, content-addressed
extracts). Source-mark input: #6044 (source-mark grammar). Gate
sequence: #6057 (staging-shard ingest gate). Staging substrate:
#5669 (staging tier — versioned schema, registry, aggregator).
Research source: the Atlas ingestion prior-art brief, Cluster 3
(in-toto / SLSA attestation shape, 2026-11-04). Design only, no
implementation is authorized by this note. The go/no-go is §7.

State verified at `origin/main`
`76775bee5a5fb566a64eb63bc51e118714bffce0` (2026-11-09), fresh for
this design pass (queue task 14015, re-verifying triage task 12459's
confirmed verdict):

- Issue #6045 is OPEN, `kind/design`, `area/infra`, no held/user/owned
  labels. Parent #6019 and relations #3125, #5669, #6044, #6057 are
  all OPEN. Sibling #6053 (the shape model for this note) has since
  CLOSED with its design landed.
- No per-batch attestation exists: `pkgs/atlas-search/schema/
  atlas_stage_v1.sql` carries `items`, `chunks_raw`, `notes`, `kv`,
  `_schema_version` only; `stage_registry_v1.sql` carries
  `staging_units` only. Neither has a batch or attestation table.
- The scaler revision design
  (`doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision`, §4) names
  a merge manifest per promotion batch (unit id, rows in/promoted,
  content hash) but no attestation record and no ingest emission
  point.
- `grep -ri 'attestation|in-toto|predicate|materials'` over
  `pkgs/atlas-search` source and the Atlas design docs finds nothing
  relevant (binary-target hits only).

---

## 1. Problem

"Where did this embed come from" is archaeology today. A promotion
batch writes chunk rows into core, and the merge manifest (§4 of the
scaler revision) records that *a* batch moved N rows with content
hash H — but nothing records, in one place, *which* chunk rows the
batch wrote, *which* source bytes they came from, *which* ingester
build did it, and *what* the classification scan said at the time.
Reconstructing any of that later means re-crawling sources that may
have changed (a repo relicensed last week was ingested under last
month's license), re-running scans whose versions have moved on, and
guessing which staging shard a row passed through.

The fix is one attestation per batch: an in-toto-shaped statement
whose subject is the batch's output rows and whose materials are the
batch's input sources, emitted by the ingest pipeline itself — for
published batches *and* for refused ones (a rejection is also a
provenance fact: the batch existed, it was examined, it did not
land, and why).

## 2. The record, field by field, in in-toto terms

One attestation per ingest batch. The shape is the [in-toto
Statement v1](https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md):
envelope + `subject[]` + `predicate`, with the predicate typed for
Atlas. Field table first, then the type/system choices that make
each field mechanical to emit and to verify.

| in-toto slot | Field | Type | Meaning |
|---|---|---|---|
| statement | `_type` | const string | `https://in-toto.io/Statement/v1`. The statement-type URI, recorded verbatim on every attestation. |
| statement | `predicateType` | const string | `https://atlas.local/ingestBatch/v1`. The Atlas predicate type for an ingest batch (versioned; a future predicate change mints `v2`, never edits `v1` rows). |
| statement | `subject[]` | array | **The chunk/span rows the batch wrote.** One entry per chunk row: `digest = { sha256: <content_hash> }` (the `chunks_raw.content_hash` / #3125 CAS hash of the row), plus `uri` = the row's stable locator (`<shard>:<chunk_id>` once promoted; `<staging_unit_id>:<item_id>:<idx>` pre-promotion) and `annotations.spans` = the span locators from #3125's occurrence model (line numbers / dates / spans for extracts). A batch that wrote zero rows (pure rejection, §4) carries an empty `subject[]` — the attestation still exists. |
| statement | `materials[]` | array | **The source bytes the batch read.** One entry per source file/snapshot: `digest = { sha256: <source content hash> }` (hash of the source bytes *as ingested*, before chunking) plus `uri` = the source-mark string **verbatim in #6044's grammar** (`repo@commit:path`, `system@host@date`, `chat@session@timestamp`, or the fallback form). Marks are copied, never reinterpreted: parsing/validation belongs to #6044 and its audit consumer #6029, and this record redefines neither. The content hash rides beside the mark so the material stays resolvable even if the source is later lost (the #6044 content-hash-in-mark rule, mirrored at batch scope). |
| predicate | `builder.id` | string | **Ingester name.** The pipeline identity that ran the batch (e.g. the sweep/ingest binary or script id under #6019's operations design — `atlas-ingest`, a named sweep class, or the session-close hook from #5669). A fixed controlled vocabulary owned by #6019, recorded verbatim here. |
| predicate | `builder.version` | object | **Ingester version + repo git hash.** `{ ingester_version: <binary/tool version>, code_rev: <git rev of developing-today/code the batch ran from> }`, per the fleet's versioning ruling (name + version + git hash). `code_rev` is `git rev-parse HEAD` of the checkout that performed the ingest. |
| predicate | `predicate.batch` | object | Batch identity: `{ batch_id, staging_unit_id, gate_run_id, started_at, finished_at, outcome }`. `outcome` is `published` \| `rejected` \| `quarantined`. `staging_unit_id` joins to #5669's `staging_units` registry; `gate_run_id` joins to the #6057 gate run. |
| predicate | `predicate.classification` | object | **Classification tier + scan results.** `{ tier: public \| internal \| secret, scanner_name, scanner_version, scan_verdict, scan_evidence }`. Tier is the three-level core tier from #5669 as assigned by the #6057 classify stage (§4); `scanner_*` names the classification machinery (family under #6021) that produced the verdict; `scan_evidence` is a content hash or pointer to the scan output, not the output itself. For a rejected batch this is where the refusing verdict lives (§4). |
| predicate | `predicate.prior_attestation` | string, nullable | **Chain link.** The envelope digest (§5) of the prior batch's attestation for the same source, or `NULL` for a first ingest. The only cross-batch reference; lookup rule is §5. |
| envelope | `envelope_digest` | string | `sha256` over the canonical JSON bytes of the statement (JCS / OLPC canonical form, fixed at `v1`). Named explicitly: **envelope digest = `sha256(canonical_statement_bytes)`, hex-encoded, and it is the attestation's primary key** (`attestation_digest`). The DSSE envelope shape is adopted (`payloadType: application/vnd.in-toto+json`, `payload` = the statement, `signatures: []` — empty today, see §6) so a future signer fills the reserved array without a schema change. |

Type/system choices, stated so emission is mechanical:

- **Digest algorithm is `sha256` everywhere** — subject digests,
  material digests, evidence pointers, envelope digest. One
  algorithm, no negotiation, no `sha1` legacy path. (Subject digests
  coincide with the existing `content_hash` values where those are
  already sha256; where a legacy hash differs, both are recorded and
  `sha256` is the join key.)
- **Statement type and predicate type are constants**, recorded
  verbatim per row, so a consumer never infers the schema version
  from context. `predicateType` versioning (`…/ingestBatch/v1`) is
  the schema-evolution mechanism: new fields mint `v2`; `v1`
  attestations are never rewritten.
- **Rows are append-only.** A correction is a new attestation for a
  new batch (chained via `prior_attestation`, §5), never an edit.
  The attestation a past decision cited is byte-identical forever —
  the same rule as the sibling license-provenance record's I3.

## 3. Where records live

Beside the staging/publish gate, as a sibling under #6019 — not
inside the staging DBs and not inside the searchable corpus shards:

- **Table `ingest_attestations`** in the internal-core-tier registry
  database that already holds #5669's `staging_units`
  (`stage_registry_v1.sql`'s store, owned by the provisioning tool).
  One row per batch: `(attestation_digest PK, batch_id UNIQUE,
  staging_unit_id, gate_run_id, outcome, statement_json,
  emitted_at)`. `statement_json` is the canonical statement bytes;
  `attestation_digest` is their sha256 (§2). The registry is the
  query path: "where did this embed come from" is a lookup from
  chunk hash → attestation(s) → source marks, joining
  `staging_units` on `staging_unit_id` and the #3125 occurrence
  registry on subject digests.
- **Artifact: the canonical statement JSON**, content-addressed by
  `attestation_digest`, archived with the staging DB branch that
  #5669's aggregator keeps on unit close. The table row is the
  index; the artifact is the durable copy that survives registry
  surgery. Promotion batches additionally cite the attestation
  digest in the merge manifest's per-batch entry (manifest gains one
  field, `attestation_digest`; the manifest's existing fields are
  untouched).

Rationale for the placement: staging DBs are per-unit working
stores (read at promotion, then branched as archive — they must
stay cheap to write and cheap to branch); corpus shards are the
searchable set (a failed batch must never touch them, per #6057).
The attestation is operations metadata about *batches*, in the same
store as the operations metadata about *units* — one registry to
consult, and the rejection path (§4) has somewhere to write even
when no staging row and no corpus row exists.

## 4. Emission points — one per #6057 gate stage, plus rejection

The #6057 gate sequence is write → classify → dedup →
provenance-check → publish. The attestation is opened at write and
sealed exactly once, at publish *or* at rejection — every batch that
enters the gate leaves exactly one attestation, whatever its fate.
(#6057 owns the stage contracts and pass/fail semantics; this note
only names what the attestation layer does at each stage and
redefines nothing.)

| #6057 stage | Emission-point behavior |
|---|---|
| 1. **Write** (to a staging shard, never the corpus) | **Open** the attestation: allocate `batch_id` + `gate_run_id`, record `builder.*`, `staging_unit_id`, `started_at`, and the `materials[]` list (source hashes + verbatim #6044 marks). Resolve `prior_attestation` now (§5 lookup) so the chain link is fixed before any verdict. No subject rows yet; no outcome. |
| 2. **Classify** (tier scan assigns/verifies tier) | **Fill** `predicate.classification` with the scan's verdict (tier, scanner name/version, verdict, evidence pointer) — pass *or* fail. A failing scan does not stop recording; it starts the rejection path below. |
| 3. **Dedup** (exact + near-duplicate edges) | **Annotate** the working attestation with the dedup outcome: which subject candidates were dropped as dupes (their hashes move to a `dedup_dropped[]` annotation inside `predicate.batch`, not into `subject[]`) and which survived. Surviving rows become the provisional `subject[]`. |
| 4. **Provenance-check** (parseable source marks + attestation linkage) | **Validate** that every material's mark parses under #6044's grammar (parseable? fields in range? — the #6029 rule list, executed here, defined there) and that the subject↔material binding is complete (every subject row traces to ≥1 material). A mark failure routes to the rejection path; the failure itself is recorded in `predicate.classification.scan_evidence` / `predicate.batch` notes. |
| 5. **Publish** (promote to the live shard set) | **Seal with `outcome = published`**: finalize `subject[]` with post-promotion locators, set `finished_at`, compute the envelope digest, write the `ingest_attestations` row + archive the artifact, and cite the digest in the merge-manifest entry. Publish is the only stage that writes corpus rows; the attestation write is part of the same promotion batch (same bounded-txn discipline as #5669's ≤500 rows/txn). |
| **Rejection path** (any stage fails; failed batch never touches the corpus) | **Seal with `outcome = rejected` (or `quarantined` when the batch is held for human review rather than dropped)**. The attestation carries the materials as-read, whatever subject rows existed at failure time (possibly empty), the failing stage's verdict in `predicate.classification` / `predicate.batch`, and `finished_at`. It is written to `ingest_attestations` and archived exactly like a published one. Retry of the same source is a *new* batch chained via `prior_attestation` (§5) — the rejected attestation stays as the record that the first attempt happened and why it did not land. |

Exactly-once invariant (property-shaped, per the fleet testing
doctrine — a PBT oracle over the registry): every `gate_run_id`
that passed stage 1 has exactly one sealed attestation
(`finished_at` non-null, `outcome` in its enum); no batch seals
twice and no batch vanishes. The check is `COUNT(attestations)
GROUP BY gate_run_id = 1` plus `outcome`-enum validity.

## 5. Chained re-ingest

A re-ingest of the same source references the prior batch's
attestation — the corpus can walk a source's history without
re-crawling it:

- **Reference field:** `predicate.prior_attestation` (§2) holds the
  prior attestation's `attestation_digest`, or `NULL` when no prior
  attestation exists for the source (first ingest).
- **Lookup rule:** at stage 1 (write), before opening the new
  attestation, query `ingest_attestations` for the latest sealed
  attestation whose `materials[].uri` set intersects the new batch's
  source-mark set (same source identity under #6044's grammar —
  same `repo@commit:path` repo, same `system@host` + locator, etc.),
  ordered by `emitted_at` descending, and take the first row. That
  digest is the new attestation's `prior_attestation`. The rule
  matches on source *identity* (the mark's locator part), not on
  commit/capture-time (which legitimately advances between
  ingests) — so a re-ingest at a new commit still chains to the
  prior generation.
- **Chain properties:** the link is a single backward pointer, not a
  list — history is walked by iteration, and a missing link (prior
  attestation archived but registry row lost) terminates the walk
  with a recorded gap, never with a fabricated pointer. A rejected
  batch (§4) counts as a link: a retry chains to the rejection, so
  the history shows attempt → refusal reason → retry, not just the
  retry.

## 6. Signing — position

**Signing is optional for this personal system; the shape is the
value.** No signature is required to write, archive, or consume an
attestation, and no implementation in this design is gated on key
management. Rationale: the threat signing addresses (a third party
forging ingest provenance) does not exist here — the corpus has one
operator, one ingest pipeline, and audit happens by the operator
reading their own registry. What the shape buys without any signing
is the lookup the issue asks for: chunk → batch → sources +
builder + scan verdict, in one record, for published and rejected
batches alike.

The design still reserves the signing path so the position is
reversible without a schema migration: the DSSE envelope's
`signatures` array ships empty today, and a future decision to sign
fills it (key choice, rotation, and verification policy would be a
separate follow-on note — explicitly out of scope here, §8). An
unsigned attestation is complete and queryable; a signed one adds
non-repudiation, nothing else.

## 7. Decision / go-no-go

**Go** on the §2 record with the §3 placement, the §4 emission
points, the §5 chain rule, and the §6 unsigned posture, as a
registry-resident table plus content-addressed artifacts under #6019.
Specifically:

- The design's deliverable is the field set (§2), the table +
  artifact placement (§3), the per-stage emission behavior including
  the rejection path (§4), and the re-ingest chain rule (§5).
  Implementation (registry migration adding `ingest_attestations`,
  gate wiring at the five #6057 stages + rejection path, merge
  manifest's one new field, canonical-JSON + digest tooling) is a
  separate follow-on issue, filed with this record.
- The in-toto Statement v1 shape, the `atlas.ingestBatch/v1`
  predicate type, sha256-everywhere, and the DSSE envelope with
  empty `signatures` are fixed by this note; evolving them mints a
  new predicate version, never a rewrite of sealed rows.
- Open dependencies (all cited, none redefined): #6044's grammar
  must exist before provenance-check validation (§4, stage 4) is
  enforceable — until it lands, marks are recorded verbatim and
  validation is a recorded skip; #6057's gate must exist before
  emission wiring has stages to hook; #3125's occurrence/CAS model
  supplies the subject rows; #5669's registry database is the
  table's home.

## 8. Non-goals and dependency boundaries

- **Redefining the source-mark grammar (#6044)** — marks are copied
  verbatim into `materials[]`; parsing, formatting, and validation
  rules belong to #6044 and its audit consumer #6029.
- **Redefining the ingest gate stages (#6057)** — stage contracts,
  pass/fail semantics, quarantine handling, and publish atomicity
  belong to #6057; this note only specifies what the attestation
  layer records at each stage.
- **Changing #3125's chunk_occurrences or content-addressed extract
  design** — the subject rows reference that model; the model
  itself is untouched.
- **Changing #5669's staging tier** — the attestation table lives
  in the registry database #5669 defines; the staging schema,
  promotion batching, and merge manifest semantics are otherwise
  untouched (one additive manifest field, §3).
- **Signing infrastructure, key management, or signature
  verification** — §6 records the unsigned posture and reserves the
  envelope slot; the rest is a future note if ever wanted.
- **Any implementation** — no table, emitter, or migration is
  authorized by this note; this is a design doc only.
