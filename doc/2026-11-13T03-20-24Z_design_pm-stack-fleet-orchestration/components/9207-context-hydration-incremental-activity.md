# #9207 component design: context hydration and incremental activity

**Status:** Proposed component contract for owner review; not an adopted FDR or implementation claim. **Research date:** 2026-11-13 UTC (2026-11-12 America/Chicago). **Source pin for repo evidence:** `a038f084dbeb525494d7bb928ad91f597db2ac51` per `../census/track-b.md`. **Route context:** research was explicitly assigned `opencode-route/workhorse`; no route changes. **Safety:** design/issue specs only. No schema, production, dispatch, DB, scheduler, or binary change.

**Shared placement/interface:** proposed private modules `pkgs/fleet/crates/fleet-queue/src/work/context.rs` and `activity.rs`; shared identity/command/result contracts live in `work/contracts.rs`; storage extends existing `turso_queue.rs`/broker only after schema gate. Existing `pkgs/fleet/crates/fleet/src/gh.rs` is the adapter seam; no `fleet-gh` crate. Wire follows `../architecture.md`: snake_case; `work: {authority_id, work_id}` string pair; authenticated principal, not client actor, controls authorization; IDs/versions/generations are canonical decimal strings where needed. Proposed paths are not current functionality. One integration slice alone owns `fleet-queue/src/lib.rs`, Fleet `main.rs`, and registry module registration; component slices create private module files only.

## V1 — current baseline

Pinned census `../census/track-b.md` finds a queue-focused `fleet-queue` CLI and no registered lifecycle hydration/activity namespaces; queue listing tests exist but do not establish hydration, cursors or deployment. Existing `fleet gh` is the adapter boundary. See the pinned evidence overlay below for source-line qualification.

## 1. Purpose, boundaries, and ownership

Hydration creates a durable, navigable, provenance-bearing snapshot of the Fleet work item and its linked context, then supports refresh/change queries that do not skip edits or present mixed/stale evidence as complete. Track B owner is #9207. It consumes canonical WorkId/resource links and lifecycle ownership from #9206; note/journal and checkpoint content authority from #9208; completion-time refresh and effect receipt from #9209. Checkpoint *command and state-transition* contract belongs to #9206/#9208; this component owns only recording checkpoint references in bundle/session context and cursor snapshots. Work with #9208 on private/public separation and hostile-content treatment; do not duplicate its field-authority rules.

Reuse existing Fleet queue/client, `fleet gh` boundary, and storage/artifact conventions after census confirms their exact API. Census at pinned commit finds no registered `fleet work hydrate/refresh/changes`, `fleet activity`, or `fleet sync` surfaces; current CLI registry is queue-focused. This is source/test evidence, not runtime/deployment evidence. No new queue, database, scheduler, binary, general GitHub client, or hidden polling service.

## V2 contract: identifiers, manifest, and immutable bundle generations

### 2.1 Identity and safe paths

Input is canonical `WorkId`, never a GitHub issue number alone. Each linked source ref includes provider, owner/account, repository, resource kind (`issue`, `pull_request`, `review`, `comment`, etc.), and stable remote ID; display `owner/repo#number` is presentation only. Thus `alpha/repo#10` and `beta/repo#10` cannot collide. A Fleet work item may link multiple issues/PRs and multiple repos.

Artifact directory proposal (implementation must use existing artifact root/path policy): `.fleet/work/<work-id>/context/generations/<generation-id>/manifest.json`; stable `current.json` points at the last fully published generation. Filenames use a safe slug plus collision-resistant source identity, not raw title/path/owner input: e.g. `pull-<repo-id>-<number>-<remote-id>.json`, `diff-<repo-id>-<pr-number>-<head-sha>.patch`. Reject path separators, `..`, control chars and symlinks escaping root. Path shortening must not remove a digest suffix. A PR filename is scoped by repository; same PR number in another repository is distinct. Content is stored by artifact references outside the queue row, bounded under the existing storage owner.

### 2.2 Versioned manifest (normative shape candidate)

The manifest digest follows the exact `fleet.context-manifest/v1` framing and self-field
exclusion in [`architecture.md`](../architecture.md#digest-and-serialization-contract).
Omit only `integrity.manifest_digest` before RFC 8785 canonicalization, hash the registered
ASCII purpose plus one NUL byte plus canonical UTF-8, and emit `sha256:` plus lowercase hex.
Referenced artifact hashes cover exact artifact bytes. HYD-01 includes independent framing
and changed-field fixtures; hashing a manifest including its own digest is not the contract.

```json
{
  "schema_version": "fleet.context-manifest/v1",
  "work": {"authority_id":"local", "work_id":"842"},
  "generation_id": "g-<stable unique id>",
  "created_at": "<RFC3339 UTC>",
  "status": "complete | partial | failed",
  "source_pin": {"queue_version":"19", "lifecycle_version":"6"},
  "sources": [{
    "source_ref": "github://owner/repo/pulls/231",
    "endpoint": "pull-request-detail",
    "api_version": "<negotiated version>",
    "query_fingerprint": "<stable digest>",
    "status": "complete | partial | unavailable | forbidden | not_found | truncated",
    "fetched_at": "<RFC3339 UTC>",
    "source_updated_at": "<RFC3339 UTC or null>",
    "remote_version": {"head_sha": "...", "etag": "..."},
    "pages_fetched": 1,
    "records_seen": 1,
    "records_persisted": 1,
    "truncation_reason": null,
    "artifact_refs": [{"path":"...", "sha256":"...", "bytes":123}],
    "error": null
  }],
  "summary_ref": {"path":"summary.md", "sha256":"..."},
  "previous_generation": "...",
  "integrity": {"algorithm":"sha256", "manifest_digest":"..."}
}
```

The contract includes every linked queue field/custom column, status/reason, owner and lease metadata subject to access policy; issue and PR title/body/state/labels/assignees/milestone/project fields; comments and their create/update state; issue timeline; linked PRs; PR head/base SHA, title/body/state, checks, reviews, review threads/discussion, commits, author/committer identities/times, messages/parents, changed paths/rename source, diff hunks and old/new ranges; typed dependency refs and resolution state; previous Fleet notes/checkpoints/activity cursor snapshot with privacy boundaries. Large payloads/diffs live as refs, not queue-row blobs.

Completeness is per source/endpoint, then aggregated. `complete` means every required resource was fetched to endpoint's documented bound and durably stored, with no omitted pages, truncation, forbidden/missing dependency or error; optional resources need explicit `not_applicable`. `partial` includes missing/forbidden/unavailable/truncated. `failed` means no generation could be published. `summary.md` is a concise navigation layer, never evidence of completeness by itself. Include findings, blocker/review/check warnings, source refs, freshness, partial-status summary, and exact file paths. Do not summarize missing data as empty.

### 2.3 No partially published “complete” snapshot

Build generation in a staging location; stream each page/item to the artifact store with bounded buffers; validate paths/hashes; write manifest last; then atomically publish by changing `current.json` (or use the storage equivalent if atomic rename is unavailable). Readers see either old current generation or new one, never partially written files. If storage cannot atomically swap, write immutable manifest, then a versioned pointer with CAS/conditional update; document weaker semantics and test recovery. Failed refresh leaves prior current bundle intact and new generation marked failed/partial outside current pointer. Never mutate prior immutable generation.

Bind all PR-derived artifacts in a generation to one captured PR `head_sha` and record it on each PR detail/review/check/commit/diff fetch. If head changes during retrieval, do not combine old diff with new head metadata: retry source snapshot within bounded caller deadline or mark that source `partial/stale_head`; keep the prior complete generation available. Snapshot consistency across GitHub endpoints is not guaranteed, so manifest states source versions and retrieval interval rather than claiming a global atomic snapshot.

## 3. Retrieval and performance contract

Use `fleet gh` / existing adapters as verified by census, follow each endpoint's pagination continuation, honor API rate-limit/retry metadata, and set per-command deadline/maximum response bytes. GitHub pagination docs state `Link` header is the continuation authority; pages default to 30 and usually max 100. API reference reports PR commit listing max 250 and changed-file listing max 3000. Reaching an endpoint cap without an alternate complete method is `truncated`, not `complete`. Stream page-by-page. Memory bound is configured max item/page buffer plus small summary state, independent of total hydrated bytes; no unbounded all-history aggregation in RAM.

Resource budget is explicit: maximum total persisted bytes, per-source/page bytes, diff bytes, elapsed time, and request count are config/capability limits. Hitting a limit produces a typed partial reason and resumable cursor, never silent dropping. Keep `summary` compact via source counts and ranked findings; allow `--include`/`--full` and direct artifact access for detail. Complexity target is O(P + B) remote work/storage processing where P=pages visited and B=bytes persisted; an incremental refresh should cost O(new/overlap pages + changed payload bytes), not O(all historical activity) except a deliberate audit/full rebuild. Cache only immutable source-versioned artifacts; bound cache by bytes and retention policy, never arbitrary “last N” item loss. Queue DB row remains lightweight.

### Command/service contract

Proposed compatibility names, not implementation claims:

```text
fleet work hydrate <work-id> [--full|--incremental] [--output DIR] [--json]
fleet work refresh <work-id> [--include SOURCE,...] [--since-checkpoint|--since ISO]
fleet work changes <work-id> [--since-start|--since-checkpoint|--since ISO] [--external-only]
fleet activity reconcile <work-id> [--source REF] [--dry-run] [--json]
fleet activity watch <work-id> [--timeout DURATION]   # bounded foreground watch, not scheduler
```

All are callers of the shared `contracts.rs` command/result envelope in `../architecture.md`, not a component-specific envelope. `watch` must be bounded and interruptible, with a durable cursor; it does not spawn background scheduler. Detailed context outcomes map into shared outcomes (`accepted`, `running`, `pending_sync`, `partial`, `conflict`, `uncertain`, `verified_complete`) and per-effect requested/observed/verified values. Return concise machine JSON and human summary. Unknown work/ref, ambiguous ref, permissions, rate limits, truncation, stale PR head, and cursor gaps are distinct errors/outcomes.

## 4. Incremental activity and cursor correctness

Cursor identity is `(source_ref, endpoint, query_fingerprint, API_version, auth-scope-class, cursor_generation)`. Store opaque continuation URL/token where applicable; normalized high-water mark, overlap interval, last durable source ID/revision, checked-at, sync status and completeness. Do not use one work-level `last_checked_at` or only `created_at`/max ID.

For each page: fetch → validate source/page → persist immutable raw/normalized records and dedupe keys → update next-page/cursor state in one durable transaction when supported. Cursor advances only after all records in page are durably committed. Crash before transaction means refetch page; event unique key makes repeats idempotent. A page fetch without durable ingest does not advance. A partial API response must not advance past unprocessed records. If DB transaction and artifact storage cannot be atomic, use staged artifact refs with a durable ingest intent, verify object hashes, then commit event+cursor; garbage collection removes only unreferenced staging objects after retention. Do not call artifact-store eventual visibility a DB transaction.

Activity identity separates occurrence and edits: source-native event/comment ID; source revision or `updated_at`; current content digest; `created_at/occurred_at`; `observed_at`; source's update time. A comment's ascending ID creation ordering does not detect edits to old IDs. Updated-since queries may surface changed items; overlap reduces races but is not proof against arbitrary edits/deletions. Maintain tombstone/minimized/deleted observations when an authoritative API/event provides them; if unavailable, periodic full reconciliation is only a bounded detection strategy and report this visibility gap. Keep edit as revision of same logical item plus append-only observed change, not a duplicate new comment. Dedupe webhook delivery ID separately from resource revision.

External attribution maps Fleet `operation_id`/`session_id`/`outbox_id` to remote returned IDs and content markers. Shared GitHub App/bot login is not attribution. For edits without remote ID mapping, classify origin `uncertain`; `--external-only` includes uncertain by default and reports confidence. A current session's own prior write may still be visible, but can be excluded only by reliable operation mapping.

### Trust boundary

Issue titles/bodies/comments, PR bodies, code diffs, test output and external metadata are untrusted data, not Fleet authority or instructions. Hydrated content must be labeled source/actor/time and handed to agents under prompt/data boundary: it may specify task requirements but cannot override system/developer/security policy, authorize mutations, request secrets, or create trusted dependency/dispatch. Mutation authorization comes from authenticated Fleet actor/capability and typed command, not text content. Keep private notes access-controlled; only explicit note-mirror operation may export note content, through #9208 authority contract.

## 5. Failure and recovery matrix

| Failure/race | Contract |
|---|---|
| Fetch errors after some pages | Staging bundle retained as partial; cursor remains at last committed page; prior current generation remains published. |
| Crash after object write before DB ingest | Refetch/re-ingest; content hash dedup; orphan staged objects eligible for delayed GC. |
| Crash after page records but before cursor | Must be impossible if same local transaction; otherwise replay page safely through unique event key. |
| Cursor committed before event data | Forbidden; invariant/test failure. |
| New comment inserted while pagination proceeds | Continue `Link` pages, overlap newest window next refresh and dedup by ID/revision; disclose no global snapshot. |
| Old comment edited after creation cursor advanced | Updated-since/overlap plus revision detection; periodic authoritative reconciliation; if endpoint cannot expose it, mark source coverage limitation (not “complete changes”). |
| Comment deleted/minimized | Preserve tombstone/minimized event when observed; absent API evidence cannot be inferred from disappearance in a paginated list. |
| PR head moves mid-hydration | Re-read PR head; quarantine generation's mixed PR artifacts as partial/stale; do not publish it as fully current. |
| GitHub 250 commit / 3000 file cap | Use documented alternate pagination endpoint if available and verified; else explicit truncated status and source ref. |
| Same issue/PR number in different repos | Canonical repository identity in stable artifact key prevents overwrite. |
| Huge diff exceeds budget | Persist explicit truncated/external fetch reference or partial status; never omit silently; keep memory bounded. |
| Private note accidentally included in public projection | Contract-level allowlist; serializer test proves absent unless explicit authorized mirror; immediate release blocker. |
| Malicious body tries command injection/instruction override | Data rendered as quoted/untrusted source; no authority follows content; mutation API still enforces actor/capability. |
| Cursor query/API version changes | Start distinct cursor generation/query fingerprint; don't reuse opaque token across incompatible query. |
| Remote API unavailable at completion | Completion receives partial/unknown freshness and cannot claim clean reconciliation; #9209 controls whether operation remains pending. |

## 6. Testing and oracles

Required future test layers (none claimed to exist):

* **MBT oracle:** a reference cursor/bundle model consumes generated page arrivals, edits, duplicate deliveries, failures and crashes. Assert page data and cursor advance together, no skipped IDs, replay idempotent, generations immutable, and published-current always complete per its manifest.
* **PBT invariants:** generate arbitrary repo names, PR numbers, titles, paths, Unicode/control/`..` input; artifact paths stay under root and distinct source refs never collide. For any sequence of pages/retries, every durably committed source revision appears at most once in logical current view and edits remain distinguishable from creations. Resource memory stays within configured bound for generated page stream (instrumented byte oracle).
* **Golden contract oracle:** versioned manifest/JSON result examples for complete, partial, forbidden, truncated, stale-head and unavailable. Validate schema, summary refs/hashes, no accidental private field serialization.
* **Failure injection:** kill between fetch/store/ingest/cursor/publish; verify prior generation and resume behavior. Fake GitHub changes head/comment between pages; fake `Link` omitted/next, pagination cap, reordered/duplicate pages, 403/429/5xx, comment edit old ID, deleted/minimized records, same number across repositories.
* **Acceptance commands:** future issue should name exact repo command after owner confirms harness, e.g. targeted `cargo test -p fleet-work-hydrator` plus golden CLI tests and fixture runner; this crate name is a proposal, not asserted to exist. Integration contract test against GitHub fixture/mock validates endpoint adapter. A live authenticated read-only test is separate evidence and cannot replace deterministic oracle.

### V1 census evidence detail (not deployment evidence)

At pin `a038f084dbeb525494d7bb928ad91f597db2ac51`, `fleet-queue` is the queue domain package, and the CLI census found no registered `fleet work hydrate/refresh/changes`, `fleet activity`, or `fleet sync` namespace (`../census/track-b.md:13–30`). Queue listing/chunked-list source tests exist but do not establish hydration. Existing `fleet gh` is the GitHub boundary; no new `fleet-gh` crate. Task/event/result/blocked-dependency fields and bounded creation outbox exist as source anchors, not proof of context bundle/cursors or deployed behavior. This is a gap statement scoped to the pinned census, not a live runtime claim.

### V1 reuse/collision row — existing PM projector webhook and hot ledger

| Pinned surface | File/test evidence | V1 capability and strict limit | Track B compatibility |
|---|---|---|---|
| GitHub webhook authentication | `pm-stack/projector/src/webhook.ts:7–28,31–57`: bounded raw-body read, HMAC-SHA256, length-checked byte comparison. `src/server.ts:150–190`: delivery ID and signature gate before event application; `README.md:23–30` lists supported issue/comment/PR/review/review-comment/push events. | Existing authenticated projector ingress. Projector remains read/projection glue and disposable; `README.md:3–8` says it never writes back to GitHub/Kaneo/Pad/Grist and its rows rebuild from GitHub + link writes. Not a second Fleet command policy or durable authoritative work store. | #9203 reuses this HMAC/intake seam and fans authorized events into the shared Fleet command interface; do not implement a second webhook-signature service or put mutation policy in pure `webhook.ts`. Keep Project as its existing sole-writer projection boundary. |
| Transport delivery dedupe | `test/webhook-delivery-id.test.ts:90–155`: invalid signature causes no data/freshness publish; missing/empty GUID rejected; same GUID applies once and redelivery returns duplicate with one projection row. Lines 157–180 test retries after freshness-publish failure. | `x-github-delivery` deduplication proves projector application semantics only. It does not prove a Fleet command was durably accepted, a session launched, or a remote mutation verified. Keep transport `delivery_id`, projector `event_id`, Fleet `operation_id` and outbox effect ID distinct. | Reuse delivery identity as `source_event_ref`; derive a separate Fleet operation ID/idempotency contract. Persist command acceptance/operation intent in canonical Fleet authority, not `pmp_deliveries`. Projection delivery acknowledgment != operation acceptance. |
| Projector hot-event sweep | `pm-stack/projector/src/store.ts:602–739`: normalized `pmp_hot_events`, stable event IDs, bounded page, server-issued high-water, consumer checkpoint; `README.md:145–179` describes dedup and sweep. `test/hot.test.ts:65–117` covers duplicate event IDs, distinct deliveries, exclusive pagination/high-water, monotone ack and forged-future-high-water rejection. | This is a durable **projector-owned disposable ledger** and consumer cursor, not Fleet's authoritative journal, general transactional outbox or complete per-resource activity store. Protocol expects consumer ACK after its sweep; server validates monotonicity/high-water but cannot prove consumer-side durability. It is not Fleet's page-ingest/cursor transaction. | #9207 may consume as one observation adapter/input with source IDs and its own durable Fleet cursor; do not replace per-source/resource/endpoint/API cursors, raw GitHub hydration or incomplete/deleted/edited-record policy with the projector global hot cursor. |
| Projector reconciliation cursors | `pm-stack/projector/src/reconcile.ts:211–243` loads/saves a cursor per `(repo, stream)`; `test/reconcile.test.ts:62–137,237–267,286–359` exercises pagination, independent issue/review-comment streams, per-PR failure handling and cursor advancement. README `:27–30` describes repair reconciliation. | Useful GitHub-source ingestion and recovery prior art, but limited to configured repos/GitHub streams and disposable `pmp_` projections. Reconciliation reads current API resources; schema-rebuild note `README.md:46–64` warns deleted comments/overwritten history no longer returned by GitHub cannot be reconstructed. Cursor is not proof all edits/deletes/events were observed or attribution is complete. | Reuse adapter/page/cursor lessons where API semantics match. #9207 still owns Fleet's versioned bundle and per `(authority,work,source,endpoint,query,API-version)` durable cursor/attribution; map projector rows as one source with confidence, not all-provider activity or canonical journal. Preserve deleted/old-edit blind spots explicitly. |
| Queue/PM read view | `README.md:113–125`: `queue-view` consumes an exported broker task JSON snapshot plus projector hot list; read-only, no broker fetch and no new persistence. | Projected snapshot for view, not live claim-order authority or broker integration. | Hydration/work-next must use existing queue authority and identify snapshot age; cannot claim projector view is atomic/current queue state. |

The queue view exposes machine and human faces of the same snapshot (`README.md:113–133`); `test/queue-view.test.ts:50–182,220+` and `test/queue-view-html.test.ts:87–162` prove pure ordering/rendering over supplied task-export/hot-list data. These tests do not demonstrate live broker fetch or persisted context hydration.

This is source/test proof at `a038f084dbeb525494d7bb928ad91f597db2ac51`, not proof of deployment. It is a reuse path, never scope reduction: do not duplicate HMAC/delivery dedupe/hot projection/reconcile adapter, and do not promote disposable projector tables into Fleet operation truth. Compatibility seam: #9203 reuses authenticated webhook intake and forwards authorized intent through the shared command interface; projection delivery outcome is distinct from Fleet command acceptance. #9207 may read hot/reconcile records as one source adapter but must preserve Fleet cursor durability, source-confidence and edit/delete visibility limits. Projector reconcile can reconstruct current GitHub resources, but README's rebuild note states deleted comments and overwritten historical records no longer returned by GitHub cannot be recovered. Projector delivery/high-water ACK is not a Fleet event ACK or proof all upstream pages/events were durably processed by Fleet.

## V3 implementation, migration, and rollback

No shared queue schema change is required for initial hydration artifact prototype if an existing artifact directory/store can be used. Any persistent cursor/index additions must be additive behind existing Fleet storage owner and coordinated with Queue Table Improvements; migration includes backfill as “unknown cursor” (trigger full initial fetch), dual-read/compare if needed, explicit rollback retaining prior tables/bundles and ignoring new records. Do not delete legacy state until rollback window and current authority approve. Recovery from partial migration leaves old reader authoritative; write path must not create two independent truths.

V3 slices: first shared command/result identity contract and manifest golden; then safe path+staging publication; then full hydration with limits/provenance; then per-source cursors/page-atomic ingest; then activity attribution and `changes`; then reconciliation/watch as bounded same-service operation; then completion freshness integration with #9209. One shared CLI integration slice owns `main.rs`/registry; this component contributes service contracts/tests and must not race another writer there.

## V4 — hosted parity

Run the same fixtures/contracts through hosted PM/Fleet surfaces #9079/#9081 with pagination, rate-limit and performance budgets. Storage internals may vary; identity, manifest, cursor, completeness and attribution semantics may not. Report bytes, pages, peak memory, elapsed time, completeness/partial rate, and incremental bytes vs full baseline.

## 8. Proposed granular issue specifications (append to #9207; no issue filing performed)

Issue sizing: each spec is one bounded PR-sized unit; IDs below are candidate stable IDs, not GitHub IDs. Common safety gate: design only here; implementation issues must not alter production until separately authorized and queue-table/schema dependencies settled. Link #9206 canonical identity/CLI, #9208 note authority, #9209 completion, and #9198 census. Hard blockers are called out; otherwise prerequisite relations are soft/rework-if-early.

### HYD-01 — Freeze versioned hydration manifest and result envelope

**Tier:** crucial. **Goal/deliverable:** schema/reference examples for versioned manifest, per-source completeness, generation lifecycle, result/errors. **Owns:** `pkgs/fleet/crates/fleet-queue/src/work/context.rs` and shared identity/command/result references in `contracts.rs`; tests in `context.rs`. **Hard blockers:** #9198 pinned code ownership/storage convention; granular #9206 identity/result-envelope slice, not all of #9206. **First failing oracle:** add `pmo_hyd_manifest_missing_required_source_is_partial` and `pmo_hyd_manifest_rejects_unknown_schema_version`; before implementation both must be listed and fail. **Done command:** confirm both appear in `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_manifest_`. **Acceptance:** complete/partial/truncated/unavailable; hashes/provenance/PR-head binding; private data excluded. Wire names snake_case; authority/work IDs are string pair; unsafe integer IDs/versions/generations decimal strings. **Negative:** missing source called complete, invalid digest/path, unknown version silently coerced. **Migration/rollback:** contract version additive; old artifacts read-only; disable serializer without deleting files. Unit, deployed rollout, live GitHub read separately recorded.

### HYD-02 — Safe cross-repository source keys and artifact paths

**Tier:** crucial. **Goal:** prevent collisions/path traversal for same PR/issue number in different repos; preserve stable remote IDs. **Owns:** `pkgs/fleet/crates/fleet-queue/src/work/context.rs` path-key helpers/tests. **Hard blockers:** HYD-01 and granular #9206 resource-ref normalization slice. **First failing oracle:** `pmo_hyd_artifact_keys_are_repo_scoped_and_traversal_safe` generates same issue/PR number in two repositories plus `../`, control and long Unicode titles. **Done command:** confirm exact test is listed by `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_artifact_keys_are_repo_scoped_and_traversal_safe`. **Acceptance:** stable deterministic unique digest-suffixed keys; rejects traversal/symlink escape/control chars; normalization does not collapse distinct refs. **Negative:** case-fold collisions, long title truncates digest. **Migration/rollback:** old artifact aliases remain readable; disable writer without deleting old artifacts. Future test is not present evidence.

### HYD-03 — Immutable hydration generation with atomic publication

**Tier:** crucial. **Goal:** stage artifacts/manifest and atomically update current pointer only after validation. **Owns:** `src/work/context.rs` publisher and crash-cut tests; `src/turso_queue.rs` only if existing storage owner persists pointer. **Hard blockers:** HYD-01/02 and #9198 artifact-store/transaction proof. **First failing oracle:** `pmo_hyd_publish_crash_never_points_to_partial_generation` injects failure before/after manifest and pointer swap plus competing CAS. **Done command:** verify test is listed with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_publish_crash_never_points_to_partial_generation`. **Acceptance:** old generation stays current on failure; no partial current pointer; immutable generations; orphan behavior deterministic. **Negative:** pointer before manifest durability, concurrent publishers. **Migration/rollback:** legacy/none pointer default; rollback pointer only, retain objects until approved GC. Implementation/deployment/live-store evidence separate.

### HYD-04 — Full bounded hydration adapter and navigable summary

**Tier:** crucial. **Goal:** fetch all required queue, issues, PRs, comments, reviews, checks, timeline, commits/diffs, dependency/checkpoint refs and render concise summary. **Owns:** `src/work/context.rs` orchestration and `src/work/authority.rs` read-policy call; adapter extension stays in existing Fleet `gh.rs`, no new client. **Hard blockers:** HYD-01/03; granular #9206 resource-identity slice; #9208 `AUTH-01`; #9198 adapter contract. **First failing oracle:** `pmo_hyd_summary_reports_partial_and_diff_provenance` fixture includes unavailable review endpoint and renamed diff with author/committer timestamps/hunk ranges. **Done command:** confirm exact test listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_summary_reports_partial_and_diff_provenance`. **Negative:** 403, missing vs empty, same PR number cross-repo, hostile instructions, private note leakage. **Limits:** request/time/total bytes/diff budgets and streaming memory. **Migration/rollback:** artifact-only read path, disable command preserving queue; mocked, deployed and live-read evidence separately recorded.

### HYD-05 — Pagination, API caps, and resumable durable page ingestion

**Tier:** crucial. **Goal:** follow `Link` continuation and persist page data/cursor atomically only after durable processing. **Owns:** `src/work/activity.rs` page iterator/cursor model and existing `src/turso_queue.rs` transaction integration after storage approval. **Hard blockers:** HYD-01; #9198 actual DB/artifact transaction contract; #9209 operation/outbox coordination if shared storage. **First failing oracle:** `pmo_hyd_page_ingest_and_cursor_commit_atomically` crashes before/after page-2 ingest; model asserts no skipped item. **Done command:** confirm exact filter listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_page_ingest_and_cursor_commit_atomically`. **Acceptance:** `Link` continuation; cap explicit truncation; idempotent repeated page; query/API version fingerprint. **Negative:** absent/malformed Link, duplicate/reordered entries, documented 250 commits/3000 files caps, object-store success + DB failure, cursor without events. **Migration/rollback:** unknown cursor forces full snapshot; legacy `last_checked_at` not trusted. No new scheduler.

### HYD-06 — Per-source incremental activity revisions and tombstones

**Tier:** crucial. **Goal:** detect new comments and edits to older comments, retain revision/tombstone/minimized observations, distinguish occurrence/observation/update times. **Owns:** `src/work/activity.rs` normalized records/revisions/change projection; append seam in `src/work/journal.rs`. **Hard blockers:** HYD-05; #9208 journal contract; granular #9206 session/checkpoint-ref slice. **First failing oracles:** `pmo_hyd_refresh_observes_edit_behind_creation_cursor` and `pmo_hyd_replay_deduplicates_revision_not_creation`; fake API edits old comment after ID watermark. **Done command:** confirm both listed with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`; then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_observes_edit_behind_creation_cursor` and the same command filtered by `pmo_hyd_replay_deduplicates_revision_not_creation`. **Acceptance:** per source/resource/endpoint/query/API cursor; cursor post-durable; overlap not proof; deletion gap reported. **Negative:** equal `updated_at`, shifting pages, absent deletion, minimized records. **Migration:** unknown full snapshot; rollback disables incremental and retains generations. **Performance:** O(new+overlap pages/bytes), bounded RAM.

### HYD-07 — Operation/session attribution and external-only semantics

**Tier:** recommended (hard before exposing `--external-only`); canonical attribution classifier. **Goal:** classify Fleet-caused source activity via op/session/outbox ID and remote IDs; include uncertain events as external by default. **Owns:** `src/work/activity.rs` mapping/confidence and `pmo_hyd_external_only_includes_uncertain_shared_actor`. #9208 AUTH-05 defines the shared principal/provenance-record contract and references this acceptance; it does not add a second activity classifier. **Hard blockers:** granular #9209 operation identity/receipt; granular #9206 session identity; HYD-06 revision store. **First failing oracle:** this test uses same bot login from two sessions with only one op/remote-ID map. **Done command:** confirm exact test listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_external_only_includes_uncertain_shared_actor`. **Negative:** coincidental same-body human comment, stale mapping. **Migration:** legacy uncertain; username never assigns session. Dispatch/#6533 remain #9203 only.

### HYD-08 — Bounded refresh/watch/reconcile CLI and capability contract

**Tier:** recommended. **Goal:** implement bounded `hydrate`, `refresh`, `changes`, `activity reconcile`, `watch` service behavior mapped to shared contracts. **Owns:** `pkgs/fleet/crates/fleet-queue/src/work/context.rs` and `activity.rs` unit tests; does not own `fleet/src/main.rs`, registry or shared CLI integration files (one #9206 integration slice owns those). **Hard blockers:** granular #9206 Command/Result contract and shared CLI-registration slice; HYD-04–06. #9203 live event routing excluded. **First failing oracle:** `pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt` tests deadline/cancel without worker spawn. **Done command:** verify test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt`. Shared CLI acceptance belongs its owner and has a separate named golden help/JSON test. **Negative:** ambiguous resource, unsupported include advertised, timeout returns success or fictional cancellation, watch spawns hidden poller. **Migration:** additive only; no queue rename. Deployment/runtime separate.

### HYD-09 — Migration, performance, rollback, and hosted conformance

**Tier:** recommended release gate. **Goal:** validate cursor/artifact migration/rollback and hosted contract parity. **Owns:** `src/work/context.rs` model/unit tests and component docs; actual storage migration remains existing owner path defined by #9198 (no new DB). **Hard blockers:** #9198 storage authority, Queue Table Improvements, HYD-01–08. **First failing oracle:** `pmo_hyd_migration_rollback_preserves_generation_and_unknown_cursor` models upgrade→write→disable/rollback→read prior generation. **Done command:** confirm exact test listed, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_migration_rollback_preserves_generation_and_unknown_cursor`. A separate DB migration command must be cited by the actual migration owner before rollout; do not guess it. **Negative:** partial migration, pointer/object mismatch, API drift, deleted referenced artifact. **Acceptance split:** unit/model, DB migration, staging, deployed observation, authenticated live read and hosted parity separately recorded. No recurring schedule without registry update.

## 9. Sources and requirement links

Primary source refs: `../sources/01_handoff_pm_stack_fleet_orchestration.md` lines 51–59; `../sources/02_charter_v1_1.md` §§10–11, 15–16; `../sources/03_full_orchestration_proposal.md` §§8–11, 16; `../sources/04_recovered_original_combined_design.md` line 13 (flattened full original; validate each slice against it). Research references: `../research/track-b-state-of-art.md` sections on hydration, cursor and failure semantics; source inventory `../research/track-b-sources.json` IDs S2–S5.

Future parent crosswalk overlays should add stable `H-R-*`, `C-R-*`, `P-R-*`, `O-R-*` statement IDs beside each requirement in the design and issue packet; IDs are intentionally not invented here before parent requirement inventory allocates them. Candidate FDRs are proposals: `FDR-PMO-9207-001` manifest/completeness; `-002` cursor semantics; `-003` hostile hydrated content boundary. Reconcile naming against #9180 before ratifying; no decision is adopted by this file.
