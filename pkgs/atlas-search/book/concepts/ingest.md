# Ingest today: staging, classification, promotion

Source: written from `pkgs/atlas-search/crates/atlas-core/src/stage.rs`, `pkgs/atlas-search/crates/atlas-core/src/gate.rs`, `pkgs/atlas-search/crates/atlas-core/src/aggregate.rs`, `pkgs/atlas-search/crates/atlas-core/src/sweep.rs`, `pkgs/atlas-search/crates/atlas-core/src/github_comments.rs`, `pkgs/atlas-search/schema/atlas_stage_v1.sql`, `pkgs/atlas-search/schema/stage_registry_v1.sql`, `pkgs/atlas-search/schema/gate_records_v1.sql`, `pkgs/atlas-search/tools/note_promotion_sweep.py`, and the turso-atlas README record under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ — provenance only; this chapter is complete without following it.

Ingest is how text becomes part of the searchable corpus. This chapter separates two things that are easy to conflate: how data actually gets in today, which is manual and script-driven, and the in-repo machinery — staging databases, a registry, classification, and gated promotion — meant to replace the manual path piece by piece. Where a piece is designed but not automatic yet, this chapter says so plainly.

## The reality first: a manual, script-driven pipeline

The production corpus was built, and is still extended, by scripts that live outside this repository, in the private hatch-personal tree — principally `atlas-ingest.py` and its companions in `workspace/turso-atlas/scripts/`. The pipeline those scripts drive has five steps:

1. **Collect.** The ingest script runs on a machine and reads each AI client's on-disk conversation store, through per-client adapters and a per-machine locations file. Nothing is uploaded by this step.
2. **Normalise.** Every conversation becomes the same shape in one local SQLite file: sessions, then messages, then chunks of roughly 3,500 characters, deduplicated by content hash, so re-runs add only what is new. That file is the durable asset; everything downstream is derived.
3. **Index.** Built from the chunks, locally: the FTS5 keyword index, and per-model vector sets — roughly 768 numbers per chunk for the current model, as [Embeddings: Snowflake Arctic and the parity contract](embeddings.md) describes. Vectors are derived and disposable, stored per model so a better model means building a second set beside the first and keeping the winner.
4. **Publish.** A copy of the hot index — keyword index plus the current-best vectors — is pushed to the hosted Turso database as distribution. The full store stays on the laptop; the hosted copy never holds the only copy of anything.
5. **Access.** Agents search through the surfaces in [The service and the Worker](service-and-worker.md), not by writing SQL against the corpus.

The operational facts: ingestion is run by hand, or by an agent lane invoking the scripts. There is no ingest schedule; the only Atlas schedule on the laptop is the daily backup timer. Corpus counts are therefore a snapshot — last verified at the 2026-10-31 publish, as [The corpus: provenance and content addressing](corpus-and-provenance.md) records — with no verified recount of later growth. The migration moving these scripts into this repository is issue #5670, still open: the Python search reference and evaluation tooling have landed in-repo; the ingest-side scripts have not. Anything describing Atlas ingest as a running service is describing the design in [Design: automatic ingest of all sources](../design/automatic-ingest.md), not today's system.

## Staging: one small database per unit of work

The in-repo replacement for direct-to-core ingest starts with staging. A staging unit is one unit of work — a queue task, a session, an epic, a batch of notes — and each unit gets its own small database, provisioned by `atlas-stage provision` (or the atlas-core stage functions) with the versioned `atlas_stage` v1 schema. Many small databases, with free writes, is the design phrase: a producer can write to its own staging database without contending with the corpus's single writer, and a staging database is cheap to create, cheap to branch, and cheap to discard.

The v1 schema is exactly five tables, applied once to an empty database; re-provisioning over an existing set is an error, and a schema check asserts the version row and the exact table set:

| Table | What it holds |
|---|---|
| `_schema_version` | One row: the schema version, 1. |
| `items` | One row per staged item — a message, note, or document fragment — with its unit id, kind, source, text, recorded sensitivity, and an open JSON metadata bag. |
| `chunks_raw` | The item's raw chunks: item id, position, text, and content hash. Raw and un-embedded, by design. |
| `notes` | Free working notes against the unit, for the life of the unit. |
| `kv` | Key/value scratch: cursors, checkpoints, producer bookkeeping. Last write wins. |

Two properties carry the design. First, a staging database never stores vectors. Embedding happens once, at promotion, so staging stays write-cheap and no producer needs the model or the runtime described in [Embeddings: Snowflake Arctic and the parity contract](embeddings.md). Second, writes are unmediated: sensitivity is recorded on each item, not enforced at write time. Enforcement belongs to classification and the promotion gate, below — staging's job is to accept work faithfully, not to judge it.

The staging vocabulary needs one warning. A unit's recorded sensitivity, and an item's default of `internal`, use the physical tier vocabulary — `public`, `internal`, `secret` — not the four-rung search ladder (`shareable`, `personal`, `local-only`, `secret-bearing`) of [Sensitivity tiers and client ceilings](sensitivity.md). The same column name carries disjoint value sets in staging and in the publish. The aggregator translates tier to ladder when it writes a promoted chunk; nothing else declares that mapping in code.

Staging helpers bound producer shapes: an artifact at or below 32 KiB stages as one whole-file unit; a single raw chunk has a hard ceiling of 64 KiB, split at line boundaries, never mid-line. `atlas-stage ingest` stages one artifact file with repository, path, and commit SHA as its source, plus kind (`prose`, `code`, or `config`) and language.

## The registry

Staging databases are found through the stage registry, its own store holding two tables. `staging_units` maps a unit id to its database location, owner, tier, schema version, and lifecycle state — `open`, `closing`, or `merged`. `merge_manifests` is the durable audit record of each successful promotion: rows in, rows promoted, content hash, embedding model, and timestamp, kept after the unit leaves the active states.

The lifecycle is short. A unit is provisioned `open`; producers write; when work is done it moves to `closing`, the signal that an aggregator may pick it up — `atlas-stage units --state open` and its siblings are the read-backs a poll would use. Promotion, below, is the terminal transition to `merged`. A promoted unit's database is stamped and branched as its archive, and that stamp deliberately closes it to further staged writes: promotion is its last write.

## Classification at staging time

Classification — deciding how sensitive each chunk is, under the model in [Sensitivity tiers and client ceilings](sensitivity.md) — can happen while staging, and for classified artifacts it must happen at source boundaries. A producer may declare the artifact's source-native classification segments: a JSON array of byte ranges, each with a sensitivity from the four-rung ladder and an optional certainty flag. The segment file is parsed strictly; a missing or mistyped field, or an unknown sensitivity name, is an error, never a silent default.

Staging composes those declarations through the boundary splitter. Classification may change only at a declared content boundary — a message or turn boundary, an authored block, a code symbol, a log record, a source-native unit — and every declared boundary splits the chunking there, so chunks partition the source exactly and never straddle a classification change. Undeclared, contradictory, or uncertain text becomes one `secret-bearing` chunk marked uncertain: doubt resolves upward, to the most restrictive rung. One classification record per resulting chunk — exact source offsets plus the effective sensitivity — is persisted in the item's metadata. Segment declaration stays with the producer; staging composes and persists, it does not originate the judgement. Read-back fails closed in the same direction: a malformed or unrecognised recorded sensitivity resolves to `secret-bearing`.

Separately, a remainder sweep exists for what classification missed: the `atlas-classify` procedure enumerates unclassified or unresolved chunks in a corpus database and, only when explicitly committed, writes one classification record per candidate. Its default is a dry run that prints its manifest and writes nothing.

## Promotion: embed once, behind the M6 gate

Promotion is the merge-on-close step that turns a staging unit into searchable core. It has two halves.

The aggregator half does the embedding. It reads the unit's items and raw chunks, embeds every chunk document-side — prompt-free, as the corpus contract requires — and writes one new core scan shard in the shard layout: chunks, their vectors as plain blobs in `vec_snowflake_m_v2_768_f32`, and a shard manifest recording the shard's tier (the unit's recorded tier), topic, part, expected chunk count, cosine metric, embedding model, engine, and source unit. Oversize chunks are split at line boundaries into pieces that fit the model's context and partition the chunk text exactly, never crossing the classification boundary the chunk was cut on; identical piece texts are embedded once per run. Each promoted chunk's recorded sensitivity is its persisted effective sensitivity when the item was classified, otherwise its item's tier translated into ladder vocabulary.

The gate half decides whether any of that may land. `Registry::promote` enforces the embedding-model contract — gate M6 of the one-model decision: a unit promotes only when its database's recorded embedding model equals the mandated model identity, `snowflake-arctic-embed-m-v2.0` in the shard spelling. The refusals are all fatal and leave the unit unpromoted: a different model named (vectors from different embedding models are not comparable), no model recorded (provenance unknown), or an empty unit. On success the registry writes the merge manifest, archives the staging database, and moves the unit to `merged`. The `atlas-stage promote` verb runs both halves together: embed into a new shard in a given directory, stamp the unit's model provenance, then promote through the gate. It needs the model and the ONNX Runtime, resolved exactly as the search binary resolves them.

## The batch gate is a different gate

Do not conflate the promotion gate with the batch ingest gate in `gate.rs`. They guard different things. The M6 gate above is a single contract check at the moment a staging unit merges. The batch gate judges a whole batch — a staging unit, or one increment of a long-lived unit — through five ordered stages: **write, classify, dedup, provenance-check, publish**.

The gate module is the durable audit record and the rules that make later stages checkable; the stage checks themselves are separate follow-on work that records its verdicts through it. Its properties:

- The batch state machine runs `open`, `closing`, `gated`, then `published` or `quarantined`, with a retry path from quarantined back to gated. No other transitions exist.
- It fails closed. A stage with no recorded verdict has not passed, and verdicts are recorded strictly in order, so no stage can be judged before its predecessor passed. A skip is allowed only in two named cases — an empty batch at write, and a publish with zero survivors and no additions pending — always carries an enumerated reason, and never produces a publish manifest.
- Quarantine is bounded. The same failure reason three times (the strike limit) marks the batch as needing review and stops automatic retries. A retry resumes at the failing stage; a failure in the write stage re-enters at write, because the producer must rewrite; and if an earlier stage's input changed, every verdict from the earliest changed stage onward is discarded — changed bytes never inherit a verdict obtained for different content.
- The record never carries content. A gate record holds counts, enumerated reason codes, row ids, content hashes, and the publish manifest id, so the record of a secret-tier batch can be read without reproducing the material that quarantined it. Quarantine is a state, not a deletion: the gate never touches the batch's staging database.

## Sweeps and dead letters

The sweep machinery in `sweep.rs` is the scheduling discipline for periodic source ingestion: candidate filtering, guardrails, headroom checks, run state, locking, and run reporting, driven by the `atlas-sweep` runner. Its constants state the posture: a candidate over 256 MiB is rejected as oversize; a run lock lives for 2 hours; a report carries at most 100 rows. Guardrail evaluation can throttle or pause a source class on measurements such as spend anomalies, and classes can be paused and resumed per host.

Failures accumulate as dead letters: a candidate that fails repeatedly — three strikes — is dead-lettered rather than retried forever, and the operator surface is list, retry one, or drop one. Cycle reports summarise runs.

What the runner does not do is run itself. It creates and enables no schedule; cadence is an operator decision, and the recurring sweep as a standing service — issue #7969 — is not built. It is a runner an operator invokes, not a daemon.

## The two landed writers worth naming

Against that background, two continuous or repeatable writers have actually landed.

`atlas-github-comments` is the one landed continuous writer. It extracts GitHub comments — issue comments, review comments, and reviews — as corpus documents in the `atlas-github-comment/v1` record shape, through the `gh` CLI's API with pagination, sequentially, as rate-limit discipline. Its estate, with no repository restriction given, is the owner's own: the `developing-today` and `developing-today-forks` organisations plus the `dezren39` account. External repositories are never extracted by it; that ingestion is gated on separate licensing work, #6024. Extraction is incremental per repository through a stored watermark, and watermarks are persisted after every repository, so an interrupted run resumes where it stopped. A permanently access-blocked repository is logged, counted, and skipped while the run continues; any other failure stops the run with state saved. Its output is an append-only records file — a feed into staging, not a promotion — which is exactly the boundary this chapter has drawn throughout.

`note_promotion_sweep.py`, in the package's `tools/` directory, is a narrower repeatable writer. It implements the agent-note placement rule — move on next touch, never big-bang: it writes the note's bytes to the new home unchanged, records the old-to-new mapping in a submission database, and only then, after hash verification, replaces the old location with a pointer stub naming the new home. An existing new home with different content is a refusal, never an overwrite; re-running a completed promotion is a no-op receipt; a pointer stub is never itself promoted. It is a notes-placement tool, not a corpus-ingest tool, and is named here so it is not mistaken for one.

## What is not automatic yet

Stated once, plainly:

- The aggregator exists as a promotion step an operator invokes per unit. The aggregator as a standing service polling closing units unattended is not built; it was deliberately out of scope for the staging foundation.
- The session-close extraction hook — the trigger that would stage a session's content automatically when it ends — is not built.
- The recurring sweep is not scheduled, and continuous ingestion of all sources (#9097) has unmet acceptance.
- The production ingest scripts still live in hatch-personal; #5670, their migration, is open.
- The only automatic Atlas job on the laptop remains the daily backup.

The designed pipeline — source hooks and reconciliation crawls feeding detection, staging, classification, the five-stage gate, and per-batch attestation, with classification always before promotion and never after — is the programme recorded in [Design: automatic ingest of all sources](../design/automatic-ingest.md). What ships today is the manual pipeline at the top of this chapter, plus the staging, classification, and promotion machinery in the middle of it, waiting on the automation that connects them.
