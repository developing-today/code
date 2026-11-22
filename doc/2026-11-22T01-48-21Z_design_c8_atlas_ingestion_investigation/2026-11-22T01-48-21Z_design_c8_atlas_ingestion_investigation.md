# C8 design: Atlas ingestion of reports and error-driven investigation

**Status:** Design for [#9576](https://github.com/developing-today/code/issues/9576), 2026-11-22. This record settles the C8 slice; it does not implement ingestion, graph storage, or the planner. **Owner boundary:** C8 supplies requirements to Atlas v1 (#9091), whose ingestion/classification/query threads remain the platform implementation owners. C8 must not fork those threads.

## Request and outcome

C8 needs one contract for report/page ingestion, links between pages and referenced work, bounded recency/dataset boosts, and a repeatable investigation path from an error string to its likely change and follow-up. The remaining acceptance demonstrations are (1) a report page is searchable after its source's next successful ingestion cycle, with recent/high-value datasets surfaced appropriately, and (2) one real error string can be traced end-to-end with evidence and explicit gaps.

This design consumes, rather than redefines, the report/page ownership in C6, the event/catch-up machinery in C2, Atlas v1's ingestion/classification/query platform in #9091, the shared Atlas metadata evolution doctrine, the ID graph/triple model in #9106, and provenance/session fields from #9073–#9075 where those fields exist.

## Decisions

### 1. Source and ingestion contract

1. **The source remains authoritative.** C6's owning report/page task writes and updates a page. Atlas stores a searchable representation and source metadata; it is not a second page store and does not become the write authority.
2. **Ingest every report/page source many times daily.** Use C2's event/catch-up path where available. Until a source has events, a bounded catch-up scan is the fallback. A source cycle is one fetch attempt over the source's cursor/window followed by an atomic batch commit. “Within one ingestion cycle” means searchable after the first successful cycle that observes the new source revision; failed fetches do not advance the cursor.
3. **Idempotency key:** `(source_kind, source_id, source_revision)`. Replaying a key updates neither the logical identity nor its link set; a new revision replaces the current searchable revision while preserving the source's history according to the platform's retention policy. The source adapter must supply a stable ID and revision (or content digest when it has no revision); C8 does not invent a revision from fetch time.
4. **Checkpoint after commit.** Persist the source cursor only after all records in its batch have been committed or individually recorded as explicit rejects. Retry transient fetch/parse failures with bounded backoff; retain the failed item and reason for a later retry. A malformed item must not make the cursor skip unprocessed records.
5. **Initial dataset priority:** report pages and incident/error reports first, then linked issue/PR/commit records needed to navigate those reports, then other C6 pages. The adapter reports source kind and priority; it does not silently drop lower-priority pages. Fairness guarantees every eligible dataset a cycle even while a high-priority source is busy.

### 2. Corpus metadata envelope

Every C8 document uses Atlas's existing versioned metadata envelope, not a C8-only index or a new authoritative schema. The following is the **C8 profile** of fields; fields not available from a source stay absent, not guessed:

```json
{
  "schema_version": 1,
  "source": {
    "kind": "report_page",
    "id": "stable-source-id",
    "revision": "source-revision-or-content-digest",
    "url": "https://…",
    "observed_at": "UTC timestamp",
    "modified_at": "UTC timestamp or null"
  },
  "page": {
    "kind": "incident_report",
    "media_type": "text/markdown",
    "url_parts": {"scheme": "https", "host": "…", "path": "…"},
    "generated_by": [],
    "graph_libraries": []
  },
  "links": [],
  "provenance": {},
  "extras": {}
}
```

- `source.kind`, `source.id`, and `source.revision` identify a source record; `observed_at` is Atlas observation time, not a claim about when the page was authored. `modified_at` is source-supplied and optional.
- `page.kind` and `media_type` are typed, documented values. `url_parts` is parsed from the canonical source URL; credentials and secret query parameters are discarded. Retain the canonical URL only under existing source visibility rules.
- `generated_by` and `graph_libraries` are optional lists. They capture generating scripts and graph/rendering libraries only when the page source provides them. `links` holds typed source references described below.
- Use the #9095/#9091 doctrine: version always; additive-only changes; old versions remain readable; unknown fields go in `extras`; invalid safety/classification enums take the most restrictive value. A type mismatch relocates the value to `extras` rather than rejecting the whole item.
- Consume #9073 create-path provenance and #9075 session identity/worktree context only when present and permitted. Do not infer a session, worktree, commit, author, or issue link from temporal proximity. Preserve provenance as provenance, not as page-authored facts.
- Classification and access filtering run before indexing and before each linked-item expansion. Never widen the source's audience by making a page, error string, or linked title discoverable through Atlas.

### 3. Link model and bounded cross-linking

Represent source links as typed metadata relations that can be projected onto #9106's triple/link model; do not create a parallel graph store. A link has `{kind, target_id, source_locator?, evidence, confidence}`. `kind` is from a closed vocabulary: `mentions_issue`, `mentions_pull_request`, `mentions_commit`, `links_page`, `generated_by`, or `related_error`. The target ID is canonical in its owning system. `source_locator` identifies the page section/anchor when stable; `evidence` distinguishes an explicit source link from an inferred association; inferred links always carry confidence and provenance.

Resolution is two-phase: extract explicit links from the source first; then resolve known IDs and embed-level references against authorized Atlas records. An unresolved link remains a typed unresolved reference and is retried on a later cycle. Re-resolution is idempotent. A changed target does not rewrite the source page.

Cross-link traversal is bounded to one hop by default and at most two hops when a query explicitly asks for related work. Keep a visited set, deduplicate by canonical target ID, cap returned records using the planner's result budget, and retain the path/provenance for every result. A link or embedding match is a retrieval hint, never an authorization grant. Cycles, missing targets, and unauthorized targets terminate traversal without exposing their existence or counts.

### 4. Recency and dataset-aware planner boosts

The platform query planner (#9091 E/G) remains the one planner. C8 contributes bounded ranking features, not a second query engine:

- **Hard filters and authorization first.** Classification, tenant/scope, explicit query filters, and source visibility are applied before boosts. A boost can never recover a filtered candidate.
- **Two explainable features:** source-dataset priority and recency. Recency uses source `modified_at` when trustworthy, otherwise Atlas `observed_at`; each result exposes which timestamp and boost reason were used. Dataset priority follows §1.5 and is configured in a versioned policy, not encoded as per-query magic constants.
- **Bounded additive score:** `final = base + min(B_dataset, dataset_weight × dataset_priority) + min(B_recent, recency_weight × decay(age))`. The two caps are independently configured and together may not outweigh a materially stronger exact/content match. Defaults are conservative; tune against an evaluation corpus before enabling. Missing timestamps receive no recency boost. Return the unboosted score and reason codes for auditability.
- **Error-investigation mode is opt-in.** It can boost the first observed occurrence of an exact normalized error string and records matching evidence. Normalization is conservative (Unicode normalization, whitespace normalization only); preserve the original string and do not merge merely similar errors. False-positive grouping must not change ordinary search.
- **Feature gate:** keep planner boosts disabled until the implementation has deterministic ranking tests and a before/after evaluation. The existing `planner_enabled` flag is not proof that planner stages are implemented; currently the Atlas query path reports planner/spell stages unavailable.

### 5. Error-driven investigation recipe

An investigation is a reproducible evidence bundle, not an automated claim that a commit caused an error:

1. Accept an exact error string and its source record (trace/log/report), observed time range, and authorized scope. Record a stable fingerprint alongside the original bytes/string; a fingerprint groups exact normalized occurrences only.
2. Find the first **observed** occurrence within retained, authorized telemetry. State the search window and retention floor. If the actual first occurrence predates retention, label the result “earliest retained observation,” not “first ever.”
3. Search issues, PRs, commits, and report pages for the exact string, linked file/path, and bounded semantic matches. Keep exact matches distinct from suggestions.
4. Build only evidenced relations: commit → PR → issue, plus worktree/session/queue-attempt context from #9073–#9075 and queue history when present. Each edge records its source and confidence; missing edges remain missing.
5. Check worktree/session state for an unpushed final commit only when the authoritative state source reports it. Report the check time and source. Never infer an unpushed commit from an absent PR.
6. Produce a compact investigation record: query/fingerprint, time range, evidence IDs and links, exact-vs-inferred edge labels, candidate changes, rejected candidates/reasons, missing-data warnings, and next action. Preserve session-transcript expectations as explicit source references only where a transcript exists and is authorized.
7. A human/triage workflow may classify recurring “rhyming” fixes into a proposed class fix and residuals into triage-script/subtask candidates. C8 emits those candidates; it does not autonomously edit policy, file duplicate tasks, or deploy a recurring job. Any recurring job later created must also update the fleet schedule registry.

### 6. Failure and privacy behavior

Fetch failure, invalid metadata, unavailable joins, or missing telemetry are visible as per-source/per-edge statuses; none are converted into negative facts. Partial investigation is a valid result if it names its gaps. Error strings and transcripts may contain sensitive material: apply source classification before indexing, redact secrets under the existing rules, and do not put raw sensitive values in logs, metrics, or task payloads. A page excluded by policy is not represented by a discoverable placeholder or count.

## Delivery slices and ownership

These are named implementation slices for follow-on packets; the design issue does not implement them:

| Slice | Deliverable | Owner / constraint |
|---|---|---|
| C8-I — report/page adapter and cycle checkpoint | Idempotent source adapter, cursor commit semantics, retry/reject evidence, fixture-backed “visible after next successful cycle” check | C8 implementation; integrate through #9091 ingestion interfaces |
| C8-M — metadata and typed links | C8 profile in the shared versioned envelope, URL sanitization, explicit link extraction/resolution, compatibility test for #9106 relation projection | C8 implementation; consume #9073/#9074/#9075 fields where available |
| C8-R — bounded planner features | Dataset/recency policy, explainable capped boosts, exact error mode behind a flag, ranking evaluation corpus | #9091 planner owner; C8 supplies feature requirements and evaluation cases |
| C8-E — investigation evidence bundle | Exact fingerprint/time-range workflow, provenance-labelled commit→PR→issue joins, gap reporting, unpushed-final-commit check | C8 implementation; depend on live #9073/#9075 data and queue/session interfaces |
| C8-T — recurring-fix triage output | Candidate class-fix and residual-subtask records, dedupe before enqueue, schedule registry update if a recurring job is later approved | C8 output only; task creation follows one-queue filing rules |

The platform integration is a hard dependency for shipping production behavior, not for settling this C8 design. Do not fork or duplicate #9091 work. Where the exact #9073–#9075 field is not yet available, the adapter remains optional and records “unavailable”; file a narrowly scoped follow-on against the field-owning issue rather than inventing a parallel field.

## Verification contract

Each implementation slice names an oracle:

- **Replay property:** applying the same `(source_kind, source_id, source_revision)` twice produces the same indexed record, link set, and cursor; a failed batch never advances its cursor.
- **Visibility property:** no cross-link traversal returns a target outside the caller's authorized scope, including through two-hop traversal or a cycle.
- **Ranking oracle:** a fixed corpus with hand-labelled exact matches, recent records, dataset classes, missing timestamps, and filtered candidates verifies both score caps and reason codes; an exact strong match remains above a weak candidate receiving both boosts.
- **Investigation model test:** a fixture event graph includes first/earliest-retained distinctions, explicit and inferred edges, missing commit/PR/issue links, an unauthorized target, and an unpushed-final-commit state. The result must preserve evidence provenance and report all gaps without claiming causation.
- **Acceptance demonstrations:** a fixture report page becomes searchable after one successful source cycle; an exact real error string is traced through the same recipe with an evidence bundle and any unavailable #9073/#9075 fields called out. The real-string demonstration must use authorized data and redact sensitive values.

## Alternatives considered

- **Let C8 own an independent ingest and planner:** rejected; it duplicates #9091's platform threads and would fork Atlas's source, classification, and ranking contracts.
- **Copy page content into PM-stack-owned Atlas records:** rejected; report pages remain source-authoritative and Atlas stores a searchable representation.
- **Unbounded graph expansion or embeddings as facts:** rejected; both can leak unrelated records and make ranking opaque. Use bounded, authorized traversal and evidence-labelled links.
- **Hard-code exact recency windows/weights now:** rejected; no evaluation corpus or measured source cadence supports numeric values. Keep caps and policy versioned; choose values with the planner evaluation.
- **Require all #9073–#9075 fields before C8 design:** rejected; their absence is represented explicitly. Integration work that needs a missing field is blocked on the owning issue, not the design.

## Open items with defaults

1. Exact source revision token varies by page store. **Default:** source revision when provided; otherwise digest of canonical content bytes. Never use fetch time as a revision.
2. Polling interval and per-kind priority weights need measurement. **Default:** many-times-daily cycles via C2, prioritize report/incident pages, preserve fairness; do not promise a fixed latency SLA until measured.
3. Numeric ranking caps and decay curve need an evaluation corpus. **Default:** conservative bounded additive boosts; leave disabled until evaluation.
4. Exact provenance fields and queue/session joins depend on live #9073–#9075 and queue interfaces. **Default:** consume existing fields, omit absent ones, route missing-field changes to their owning issues.

## Sources and change log

- [Reporting architecture proto, C8 and collision map](../2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/note.md).
- [Atlas v1 proto, metadata schema and evolution doctrine](../2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md).
- [ID/Atlas primitives, triple core and issue map](../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md).
- [FDR-0001 resume-by-session](../decisions/FDR-0001-resume-by-session.md) and [FDR-0005 fleet run identity](../decisions/FDR-0005-fleet-run-identity-transaction-log.md).
- Atlas planner seam: `pkgs/atlas-search/crates/atlas-proto/src/lib.rs` (`planner_enabled`) and `pkgs/atlas-search/crates/atlas-core/src/shingles.rs` (existing shingle-overlap ranking primitive).

## Change log

- 2026-11-22: Initial C8 design for #9576; settles source/cycle contract, metadata profile, typed links, bounded boosts, investigation recipe, and implementation ownership without duplicating #9091.
