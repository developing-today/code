# Add an ingest source

Source: `pkgs/atlas-search/crates/atlas-core/src/stage.rs`, `pkgs/atlas-search/crates/atlas-core/src/gate.rs`, `pkgs/atlas-search/crates/atlas-core/src/boundary.rs`, `pkgs/atlas-search/crates/atlas-core/src/aggregate.rs` and `pkgs/atlas-search/schema/atlas_stage_v1.sql` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/stage.rs, and the designs `doc/2026-11-08T13-45-00Z_design_atlas_staging_shard_ingest_gate/`, `doc/2026-11-09T05-12-32Z_design_atlas_ingest_attestation/` and `doc/2026-11-08T09-15-51Z_design_atlas_license_capture_ingest/` — provenance only; this chapter is complete without following it.

A new source — a new client export, a new corpus, a new feed — never
writes to the corpus directly. It takes one route: **stage, classify,
pass the gate, promote**. That route is the subject of
[Ingest today: staging, classification, promotion](../concepts/ingest.md);
this chapter is the contributor's version: the steps a new source's code
takes, in order, and the gates it cannot skip. Classification happens
**before** promotion, never after — content that reaches the corpus
unclassified is a defect, not a backlog item.

## Step 1 — register the source

Give the source an identity in atlas-core, following the two landed
examples (the GitHub-comments feed and the query-trace corpus): a module
carrying a source class string and a source id as constants (the shape of
`GITHUB_COMMENTS_SOURCE_CLASS` / `GITHUB_COMMENTS_SOURCE_ID` and
`TRACE_SOURCE_CLASS` / `TRACE_SOURCE_ID`), a registration record naming
the source, and the record/document shapes its staged units will carry,
each with a versioned schema string (the shape of
`atlas-github-comment/v1`). If the source will be swept periodically,
its class also joins the sweep's source-class vocabulary in the `sweep`
module — the classes the sweep runner accepts today are `boot-records`,
`session-logs`, `chat-logs`, `statistics`, `reports` and `code-repo`.

## Step 2 — the staging unit

Provision one staging database per unit of work (a queue task, a
session, a batch) through the registry:

```sh
atlas-stage provision --db <path> --unit <id> --owner <owner> \
    [--sensitivity internal] --registry <path>
```

The staging schema is `schema/atlas_stage_v1.sql`: an `items` table (one
row per staged unit of content), `chunks_raw` (raw, un-embedded chunks)
and working tables. Two properties define the tier:

- **Writes are free and unmediated.** Producers write directly, through
  `stage_artifact` in code or `atlas-stage ingest` from the shell.
  Sensitivity at this tier is *recorded, not enforced*.
- **A staging database never stores vectors**, so it is cheap to write
  and cheap to branch. Embedding happens exactly once, at promotion
  (step 5).

Provisioning today targets a local libSQL/SQLite file. Hosted staging is
a later deployment surface, not part of this foundation — do not build a
new source against a hosted staging tier that does not exist yet.

## Step 3 — the parser and chunker

The source's own code parses its native format into items and chunks.
The chunking rules are not the source's to improvise:

- **Chunks are cut at declared source boundaries only** — a message or
  turn, an authored block or paragraph, a code symbol, a log record, a
  source-native unit. The splitter and the classifier share source
  offsets: `boundary::split_by_boundaries` composes the producer's
  declared classification segments into boundary-safe chunks that
  partition the source text **exactly** — no overlap across a boundary,
  no gap.
- **Uncertainty fails closed.** Undeclared, contradictory or uncertain
  text becomes one `secret-bearing` chunk marked uncertain. It is never
  split on a guess and never given a lower sensitivity to make it fit.
- Size discipline comes from the staging module: an artifact at or below
  32 KiB stages as one whole-file unit; no `chunks_raw` chunk exceeds
  64 KiB, split further at line boundaries, never mid-line; the
  promotion-time splitter subdivides oversize chunks at line boundaries
  into pieces that partition the chunk text exactly and never cross the
  classification boundary the chunk was cut on.

## Step 4 — classification, before anything moves

When the producer can declare classification, stage classified:
`stage_classified_artifact` (CLI: `atlas-stage ingest --segments <segments.json>`)
takes the producer's source-native segments — a JSON array of
`{start, end, sensitivity, certain?}` over byte offsets — and persists
one classification record per resulting chunk, with its exact source
offsets and effective sensitivity, in the item's metadata.

- Segment files are parsed **strictly**: a missing or mistyped field, or
  an unknown sensitivity name, is a usage error — never a silent default.
- Segment *declaration* stays with the producer (the classifier or
  boundary detector that knows the source); staging composes and
  persists. Do not move declaration into the staging layer.
- Read-back fails closed per record: a malformed or unrecognised
  sensitivity in a stored record resolves to `secret-bearing`.

**Sensitivity defaults.** A staged item's sensitivity defaults to the
tier word `internal` (the staging tier vocabulary is
`public` / `internal` / `secret`); per-chunk classification uses the
four-rung ladder (canonically `public`, `internal`, `private`,
`sensitive`; spelled in code `shareable`, `personal`, `local-only`,
`secret-bearing`). Defaults exist so an undeclared unit can be staged —
they confer no right to be promoted or served. Where several
classification records cover one chunk, the effective classification is
composed, not negotiated: the sensitivity is the **maximum** over the
records, and an `excluded_personal_life` disposition is sticky — once
excluded, always excluded. That composition rule is the
narrow-never-widen rule in its concrete form, and the classification
manifest enforces the same direction at load time: the manifest schema
has no sensitivity keys at all, its column shapes bind only to
code-known label columns, and any unknown shape, semantics, column or
key fails validation and falls back to the built-in manifest, loudly.
No manifest, segment file or source declaration can widen access beyond
what the code and the records already grant.

## Step 5 — the gate, then the M6 promotion gate

Two distinct gates stand between a staged unit and the corpus. Do not
conflate them.

**The batch ingest gate** (#6057 design, #7709 record) judges one batch —
a staging unit, or one increment of a long-lived unit — through five
ordered stages: **write → classify → dedup → provenance-check →
publish**. Its record (`gate.rs`, DDL in `schema/gate_records_v1.sql`)
is the durable audit trail: counts, enumerated reason codes, row ids,
content hashes and the publish manifest id — never content, so a
secret-tier batch's record is readable without reproducing what
quarantined it. The rules that matter to a new source:

- Fail-closed: a stage with no recorded verdict has not passed, and
  verdicts are recorded strictly in order — no stage can be judged
  before its predecessor passed.
- The state machine is `open → closing → gated → published | quarantined`,
  with `quarantined → gated` on retry, and no other transitions. A retry
  resumes at the failing stage; changed bytes never inherit a verdict
  obtained for different content.
- Bounded quarantine: the same reason code three times
  (`STRIKE_LIMIT = 3`) marks the batch `needs_review` and refuses
  further automatic retries. Quarantine is a state, not a deletion —
  the gate never touches the batch's staging database.

**Attestation** binds the batch to its sources: the per-batch
attestation record (#6045, in-toto-shaped, on the #3125 provenance
substrate) is the statement that these chunks came from these sources.
In the target pipeline the gate's publish step produces it; a source
whose provenance cannot be stated in that shape fails the
provenance-check stage, not the paperwork afterwards.

**The M6 model gate** is the promotion transition itself
(`Registry::promote`, #7648, from the #6060 one-model decision). A unit
promotes only when its database's recorded `atlas_manifest.embedding_model`
equals the mandated model, `EMBEDDING_MODEL_ID`
(`snowflake-arctic-embed-m-v2.0`). Every refusal is fatal and leaves the
unit unpromoted:

- a different model named — "vectors from different embedding models
  are not comparable";
- no model recorded — provenance unknown;
- an empty unit.

The promoting verb is the aggregator's embed-at-promotion step:

```sh
atlas-stage promote --registry <path> --unit <id> --shard-dir <dir> \
    [--topic codeconfig]
```

It embeds the unit's staged chunks with the mandated model
(document-side, prompt-free), writes a new core scan shard, stamps the
unit database's embedding-model provenance, then promotes through the
M6 gate. On success the registry writes the merge manifest — rows in,
rows promoted, content hash, embedding model, timestamp — branches the
staging database as the unit's archive, and takes the unit out of every
active state (`merged`). The manifest and the archive are the durable
record. Changing the mandated model is not a source change; it is
[Change the embedding model](change-the-model.md).

## Step 6 — licensing and zone gates for external sources

Everything above is the route for the estate's own sources. External
content — anything not from Drew's own orgs and accounts — has two
further gates, both policy gates, and both currently closed:

- **Licensing (#6024, #6052, #6055).** External extraction is gated on
  licensing research (#6024). The candidate set exists —
  `docs/external-set-enumeration.md` enumerates 31 candidate external
  repositories — and the file states explicitly that zero external
  comment text has been extracted or ingested. Five legal questions sit
  in a lawyer review packet under #6055, unresolved; the packet is not
  legal advice and not a ratification of the #6052 zone policy. When a
  source is cleared, licence capture happens **at ingest** (#6053):
  the per-repo licence and its flag are recorded in the content's
  provenance, not looked up later. Deletion propagation (#6054) is part
  of the same bargain: a source deleted or made private purges its
  content on the next sync.
- **Zones (#6052).** Under the memory-integration decision's composition
  with the zone policy: before ratification, scope is fleet/own corpora
  only — no bulk external content, full stop. After ratification,
  external corpora join through this same connector and stage/registry
  path, carrying their `zone` from their **first** staged chunk. The
  gate is a data/policy gate, not an architecture gate: nothing about a
  new external source justifies a new pipeline, a side door, or a
  staging tier of its own.

## The checklist, on one page

1. Source registered: class, id, versioned record schema.
2. Staging unit provisioned; producer writes items and raw chunks; no
   vectors in staging.
3. Parser/chunker cuts only at declared boundaries; uncertain text is
   one `secret-bearing` chunk.
4. Classification staged with the content — strictly parsed, composed
   by maximum, exclusion sticky; defaults recorded, never relied on.
5. Batch gate passed in order; attestation stated; M6 gate passed;
   merge manifest and archive written.
6. External sources only: licensing cleared, licence captured at
   ingest, zone carried from the first chunk.
7. The book updated in the same change: this chapter if the route
   changed, [atlas-stage](../reference/atlas-stage.md) if a verb did,
   and the source's own reference chapter if it gained one.
