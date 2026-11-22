# Design: Atlas per-embed classification and disclosure model

- **Issue:** [#9096](https://github.com/developing-today/code/issues/9096), workstream D of #8983.
- **Inputs:** Atlas proto §D; workstream C (#9095) schema doctrine; #6037 chunk-boundary policy; #6035 span schema; #6021 revalidation and #6048 Presidio design; #6052 ingestion zones; #6059 secret-tier local embedder; #6041 retention rules; #8479 secret-bearing monolith remediation.
- **Status:** design proposal. It authorizes no corpus migration, re-embedding, purge, classifier run, or public disclosure.
- **Scope:** settle classification values, per-embed assignment and safe defaults, boundary handling, disclosure contexts, recurring classifier operations, and the #8479 remediation choice. Schema evolution mechanics remain owned by #9095; search enforcement by #9092; query-record fields by #9099.

## 1. Decisions

1. **Classification attaches to each immutable embed/span, not a file or session.** A source file can yield embeds with different classifications. Reused span content retains the most restrictive classification ever established for that content; a reference/version may raise, but never lower, its effective classification.
2. **Use a strict sensitivity enum plus a separate exclusion disposition.** The ordered sensitivity values are `shareable < personal < local-only < secret-bearing`. `personal` means owner/project-personal material, not Drew's unrelated personal-life content. That latter material is `excluded_personal_life`, a non-searchable/non-disclosable disposition, not a fifth sensitivity level. Unknown or missing enum values are invalid for new records and map to the locked default for legacy/unclassified records; they never become shareable by coercion.
3. **Default an unclassified embed to `secret-bearing` until classified.** This is a temporary handling state, not a claim that all content is secret. It is searchable only through the secret-tier path and secret-tier embedder; normal internal/public results cannot include it. A classifier may lower it only by writing a versioned, attributable classification decision with evidence. Existing explicitly classified embeds are retained during the common sweep.
4. **Classification is monotonic for reuse and copied data.** Effective handling is the maximum restriction among the content's own classification, its source/version classification, and any local recipient raise. A new decision may lower a prior classification only as an explicit reviewed correction on the canonical content record; it does not silently rewrite copies or historical decisions.
5. **Disclosure is the intersection of identity/grant, sensitivity ceiling, exclusion disposition, and output context.** A searcher's broad internal access never authorizes posting the same result publicly. Public-surface policy is applied at the final output boundary, including snippets, citations, metadata, cached results, and generated summaries.

## 2. Value semantics and assignment

| Value / disposition | Meaning | Search / handling |
|---|---|---|
| `shareable` | Project content intentionally suitable for external disclosure, subject to the destination's own checks. | Eligible for public-surface filtering; not an automatic publish authorization. |
| `personal` | Owner/person-specific project or operational content, excluding unrelated personal-life material. | Fleet-internal or same-owner hosted-per-user only; never public by default. |
| `local-only` | Restricted content whose raw text and embedding stay in the local/private boundary. | Local authorized search only; no hosted or public disclosure. |
| `secret-bearing` | Direct credentials, tokens, private keys, or other secret-bearing content, including unreviewed content. | Secret-tier storage/embedder and secret-handling path only; no ordinary search result or output. |
| `excluded_personal_life` | Drew's personal-life content unrelated to coding, design, or projects. | Excluded from Atlas search, embedding, and disclosure. Remove from active searchable index through the documented retention/remediation process; retain only the minimum audit tombstone required by policy. |

Classification is assigned to each embed (or span under #6035). A source-level classification is an input ceiling, not a substitute for per-embed assignment. The effective level for a result is the most restrictive of the embed/span, source version, and applicable recipient-local raise. The same byte-identical span reused in multiple locations inherits the most restrictive applicable classification; a low-classification reference cannot dilute a secret-bearing occurrence. This follows #6035 I7's `max(span, version)` rule, refined to include the local raise and exclusion disposition.

Every decision stores the strict enum value, schema version, classifier/reviewer identity, timestamp, evidence references, method/model/config version, and prior decision reference. The metadata artifact and signature companion are themselves content subjects and embeds; they do not inherit permission to disclose the subject's contents. Their classification may be more restrictive than the signed assertion they describe.

## 3. Boundary and chunking contract

The splitter and classifier share source offsets and boundary metadata. For each source, classification may change only at a declared content boundary: message/turn, authored block/paragraph, code symbol/declaration, log record, or another source-native unit. The chunker must not combine content across a known classification boundary into one embed. If a boundary falls within a candidate chunk, split there even when that creates a short embed; do not duplicate neighboring text across the boundary as overlap. Preserve speaker, timestamp, heading path, source ID, and byte offsets as metadata, not as an excuse to merge across classes.

Apply #6037's structural-first policy (256–512 embedder tokens, aim 384; structural overlap 0–5% only from complete units; fixed-window fallback 10%) within one classification segment. For an indivisible oversize unit, split recursively at the next safe structural boundary, then sentence/token fallback, and mark the split. If the system cannot establish a safe boundary or the classifier's boundary decision is uncertain, keep affected material `secret-bearing` and do not join it to a less restricted segment. A chunking-policy change creates a candidate generation; it does not rewrite vectors or silently change classifications in place. Coordinate with #6035's exact-byte span identity and #6038's full-window vector-reuse rule.

Classifier output must identify the exact embed/span and source offsets it classified. Any boundary revision invalidates classification only for embeds whose content or boundary changed; unchanged embeds retain their current classification and vector. Classification provenance must not expose secret text in logs, run manifests, review queues, or issue comments.

## 4. Disclosure contexts

The effective result set is computed as: authorized identity/grant **AND** context ceiling **AND** sensitivity eligibility **AND** not excluded. Context is explicit on every search request and query record; an absent or unknown context fails closed to `public-surface` for outbound sinks and to `local-only` for interactive local reads. Context does not raise identity grants.

| Context | Maximum eligible value | Rule |
|---|---|---|
| `fleet-internal` | `local-only` | Authenticated fleet task agents may search project/personal/local-only material only within their grant. `secret-bearing` requires the dedicated secret path and purpose-bound authorization; never include it in ordinary agent context. |
| `hosted-per-user` | `personal` | Tenant/owner boundary and grant required; no local-only or secret-bearing data. No cross-user retrieval. |
| `public-surface` | `shareable` | Apply immediately before transmission/posting regardless of caller clearance. Filter snippets, quoted text, file paths/metadata, citations, cached payloads, and generated output references. If an output cannot be safely filtered, withhold it. |
| `local-owner` | `secret-bearing` | Explicit local owner action may access secret-tier content through the local secret path. This is not an override of cryptographic boundaries and cannot make a public/hosted result eligible. |

`excluded_personal_life` is ineligible in every context, including local-owner search. Search implementations in #9092 enforce the table; #9099 records the context, policy version, and effective ceiling with the query and results. A downstream surface must not trust a prior searcher's context as proof of permission to disclose.

## 5. Classifier operating procedure

### 5.1 Common case: classify the unclassified remainder

The recurring operation does not rebuild embeddings. It enumerates existing embeds whose classification is absent or still at the locked `secret-bearing` default and whose classification provenance is missing. It leaves all already-classified embeds and their vectors untouched. For each candidate, it assembles only available context: embed kind, capture date, source path, and the short associated set of issues, PRs, commits, files, and designs captured with the item, plus link edges/chains known for that item. Links are evidence, not permission to copy linked content into the candidate.

If context is absent, contradictory, or too weak to support a lower classification, retain `secret-bearing` and emit a reviewable unresolved decision; never infer `shareable` from a filename, age, or lack of a detector hit. Every run is deterministic and resumable, records candidate counts by prior state and outcome, and supports a dry-run report before writes.

For a reclassify/re-metadata pass, gather related files and same-section issues/PRs/commits in a declared lookback window. Default window is **90 days**, bounded to the same project/repository and code section where available; the run manifest records the actual interval and sources. This is context only, not a bulk reclassification command. Broaden to a quarter only when section-level evidence is sparse; do not exceed one quarter without an explicit run-specific rationale. Re-embed only if content, chunk boundaries, embedding model, or embedding profile changed.

### 5.2 Schema-change review

On each #9095 schema version change, use a low-cost local classifier/reviewer model to compare the old and new schema section by section. It considers only changed or newly required sections by default. Existing per-embed classification and chunk-boundary facts are inputs; unchanged schema sections do not trigger reclassification. Changed sections produce proposed classifications and rationale in a dry-run review batch. Apply only after deterministic validation against the strict enum and a reviewer disposition; uncertain items remain at `secret-bearing`. Record model/version, prompt/config hash, schema versions, sections examined/skipped, and outcomes. No schema change triggers a full re-embed absent an independent content/chunk/embedder change.

### 5.3 Detector and human review

Use the #6048 Presidio design as a detector input, not as a replacement for classification policy: checksum/structure-backed findings can produce high-confidence secret candidates; NER/context findings remain candidates for review. Run local-only on sensitive tiers. Findings are references to embed IDs and offsets, with redacted previews; no raw secret is copied into issue comments or ordinary logs. A finding may raise handling immediately to the secret path, but automated detections never lower classification or delete content. False positives are reviewed and recorded as tuning evidence.

## 6. #8479 remediation decision

Choose **reclassify and move confirmed secret-bearing embeds into the secret tier, then purge only the confirmed wrong-tier copies from the internal searchable tier**. Do not purge the canonical content merely because it contains a secret; do not leave confirmed secrets searchable internally; do not move uncertain candidates out of the locked secret handling path until reviewed.

Order of operations: inventory and checksum the affected 638 reported chunks; produce a no-write mapping from old chunk IDs to destination secret-tier records; verify destination content and vectors (or re-embed with #6059's local secret embedder); atomically switch references/visibility so the internal index stops serving the source; propagate tombstones/purge to replicas and caches under #6041 retention rules; verify counts, referential integrity, and absence from internal retrieval; then record the run report and audit trail. Retain stable IDs/redirect tombstones only where needed to resolve existing traces without revealing content. If a source cannot be safely copied or verified, keep it unavailable to ordinary search and report it as unresolved—never fall back to leaving it visible. This remediation is a separately reviewed migration; this design does not execute it.

## 7. Worked examples

### Mixed transcript: trip planning followed by deployment

Input has a trip-planning turn followed by a deployment discussion, separated by message boundaries. The trip embed is `excluded_personal_life` if unrelated to projects; otherwise `personal`. The deployment embed is `personal` or `shareable` according to its content/evidence; if it contains a credential, split at the smallest safe boundary and classify that embed `secret-bearing`. No overlap crosses the turn/classification boundary. `fleet-internal` receives only the deployment embed allowed by its grant; `hosted-per-user` receives eligible `personal` deployment content for that owner; `public-surface` receives only deployment embeds individually classified `shareable`; no context receives excluded trip content or the secret-bearing embed.

### Project file with an embedded secret

Chunk around the secret using source structure and exact offsets. Non-secret project embeds may be `shareable`; the secret-containing embed is `secret-bearing`, and overlap must not copy the secret into a neighboring shareable embed. If the secret cannot be isolated with a reliable boundary, raise the whole affected embed to `secret-bearing`. Fleet-internal ordinary search and hosted/public surfaces omit it; local-owner secret path may retrieve it. Presidio creates a redacted finding/reference and remediation candidate.

### Personal-life session

An unrelated personal-life session is marked `excluded_personal_life` at each embed; file-level metadata alone is insufficient. It is omitted from embedding/indexing where identified before ingestion. If discovered after ingestion, stop serving it immediately, remove active vectors/search rows under the retention procedure, and retain only a minimal audit tombstone. No disclosure context can retrieve it.

### Never-classified legacy embed with no associations

Candidate stays `secret-bearing`, receives an unresolved reason such as `no_classification_context`, and is excluded from ordinary retrieval. The run reports it for later evidence/review; it does not guess from path or capture date, and it does not re-embed.

## 8. Guarantees and acceptance oracles

1. No file/session-wide classification substitutes for per-embed decisions.
2. Unknown/missing legacy classification is never public or ordinary-internal by default.
3. A source-native boundary cannot be crossed by a chunk or overlap; an uncertain boundary raises handling.
4. Reuse, references, and recipient copies never lower effective classification.
5. Disclosure filtering is applied at the destination boundary, independently of caller clearance, across content and metadata.
6. The common sweep preserves existing embeds/vectors and classifies only the unclassified remainder; lack of evidence preserves the locked default.
7. Schema review touches changed sections by default and records its model/config/schema provenance.
8. Metadata artifacts and signature companions are classification subjects themselves.
9. #8479 remediation verifies the secret-tier destination before purging wrong-tier searchable copies; no silent deletion or internal exposure fallback.
10. All state-changing runs are dry-runnable, idempotent, auditable, and resumable; ordinary logs and public outputs contain no secret spans.

Implementation tests should use an explicit oracle for each guarantee: exact expected embed boundaries and labels; no result at a stricter context; byte-identical/vector-preservation checks for untouched embeds; and run-manifest assertions proving that absent evidence leaves the locked default. The three examples above are minimum fixtures, not the whole test suite.

## 9. Implementation workstreams (proposed issues)

File under #9096 with native parent linkage. Keep each issue independently testable and avoid changing #9095's schema doctrine or #9092/#9099 ownership.

1. **Classification decision schema and legacy default adapter.** Add the enum/disposition, provenance fields, strict parsing, and locked-default behavior; tests cover unknown/missing values and raises-only effective classification.
2. **Boundary-aware classifier/chunker integration.** Carry source offsets and boundary tags, prohibit cross-boundary overlap, isolate or conservatively raise mixed embeds; fixtures cover transcript, code, logs, and embedded secret.
3. **Disclosure-context enforcement in search/output.** Implement the four-context table at result and outbound serialization boundaries, including snippets/metadata/caches; tests show high-clearance caller cannot publish non-shareable content.
4. **Unclassified-remainder and schema-change review runner.** Deterministic dry-run/commit manifests, related-context window, section-level schema diff, no-evidence locked outcome, idempotent resume, and no re-embed for classification-only changes.
5. **#8479 secret-tier remediation migration.** Inventory, verified destination copy/re-embed, visibility switch, replica/cache purge, trace-safe tombstones, and post-run absence checks; run only after the implementation and migration review are ready.

Dependencies: (1) before (2–4); #9095 schema version contract before (1, 4); #6035 span identity and #6037 boundaries before (2); #9092 search surface before (3); #6059 secret-tier path and #6041 retention before (5). The design is complete when these decisions are implemented as separate tracked slices and the acceptance oracles pass; no decision here claims those tests already exist.

## 10. Coordination and non-decisions

- #9095 owns field names, strict-enum serialization, additive-only versioning, and artifact/signature metadata schema; this record supplies values and behavior only.
- #9092 owns query/search enforcement; #9099 owns query trace fields. They consume the context table and effective classification rule here.
- #6052 ingestion zones determine where data is admitted; a zone does not override per-embed classification.
- #6021/#6048 own revalidation and Presidio mechanics; this record consumes their quarantine, redaction, and secret-tier handling rules.
- #6041 owns retention/deletion mechanics; the #8479 decision specifies which wrong-tier searchable copies must be purged after verified migration.
- The exact detector thresholds, classifier model, and cadence are implementation/pilot measurements. Full re-embedding remains rare and requires a content, boundary, or embedding-profile change.

## Sources

- #9096 and its 2026-11-13 amendment (unclassified remainder, relatedness window, schema-change review, metadata artifacts).
- `doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md` §D.
- `doc/2026-11-11_decision_id5817_sensitivity_classification.md` (related ID model; not imported wholesale into Atlas).
- `doc/2026-11-09T01-31-29Z_design_atlas_chunk_boundary_policy_v2/2026-11-09T01-31-29Z_design_atlas_chunk_boundary_policy_v2.md`.
- `doc/2026-11-13T10-55-11Z_design_atlas_span_table_schema/2026-11-13T10-55-11Z_design_atlas_span_table_schema.md`.
- `doc/2026-11-08T20-45-40Z_design_atlas_presidio_ner.md`.
- `doc/2026-11-09T05-10-38Z_design_atlas_revalidation_cadence.md`.
