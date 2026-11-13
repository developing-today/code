# Fleet credential reconciliation, history, journal, and recovery

**Status:** proposed V3 design; no live inventory or provider mutation was performed or authorized. **Owner:** domain starter #9159. **Depends on:** [secrets.md](secrets.md) / #9158 for typed credential and provider seams. **Source baseline:** `34faffbff26615e313488f6d04705a444342e41e` (2026-11-12); the intake census recorded its then-current short revision `6a7843cae598`.

## Decision and safety invariants

Implement reconciliation as a deterministic planner plus durable per-item saga, not an “upsert everything” loop. Every plan compares **two current provider observations against an encrypted, last-verified common baseline**. Provider inventory completeness is explicit and independent per account/container/scope. Plan generation is read-only; application requires a separately authorized policy and approval token tied to an exact plan hash and scope. Each side effect has a durable intent before it starts, a receipt after it returns, and readback verification before advancing the baseline. Cross-provider writes are never described as atomic.

Hard invariants:

1. **No false deletion.** Timeout, 401/403, rate limit, pagination gap, unavailable attachment, stale cursor, unknown scope, partial listing, parser failure, moved-out-of-scope item or one failed field read is `unknown/incomplete/inaccessible`, never `deleted`. A deletion candidate requires prior verified native binding, completed authoritative scope inventory, full pagination/filter coverage, and explicit missing-item confirmation. Scope shrink is not deletion.
2. **No unverified baseline advance.** A common baseline advances only after all required provider observations and item mutations/readbacks are terminal and verified for the exact scope; an incomplete run can save resumable observations but cannot move the common watermark. Keep last good baseline and append a run outcome.
3. **Fieldwise three-way merge.** For each field, compare baseline `B` with current `L` (left) and `R` (right). Neither changed → no-op; only L changed → plan L→R if policy/field mapping permits; only R changed → plan R→L; both changed to semantically equal value → no-op/metadata confirmation; both changed differently → preserve both observations and conflict. Modification timestamps only help order/display; they do not choose truth.
4. **Per-adapter/per-operation guard level.** Declare `atomic-precondition`, `serialized-by-contract`, or `unguarded`, established by current primary documentation plus an isolated test, never inferred from a revision timestamp. `atomic-precondition` means the provider itself rejects a stale revision for every writer. `serialized-by-contract` means every writer for that scope is proven to use the same serialization mechanism; a Fleet-local lease does not qualify if humans/apps/other tools can write. `unguarded` means neither property holds. Automatic apply to conflict-sensitive fields (every secret value and any field both sides may edit) requires one of the first two levels. For `unguarded`, stop the plan step as `needs_review`, or use only a separately approved, explicitly named weaker policy with the lost-update window stated in both plan and receipt. Additive writes (new item or immutable history record) are not gated by this rule. Until primary docs plus isolated tests prove otherwise, BWS, `op`, and `bw` are all `unguarded`.
5. **Journal before side effect; verify before checkpoint.** A crash/retry cannot create duplicate history, overwrite a newer value, or claim success based on process exit code. Ambiguous provider write outcomes are reconciled by idempotency key/marker plus lookup/readback before repeating.
6. **Retain rather than destroy.** `zz.delete` is immutable recoverable history, not a physical delete/automatic purge. For a verified one-sided password rotation, first create and read-back protected history records in each policy-approved store, then update the existing stale native item ID in place, verify projected fields, then checkpoint. Any failed step stops the item and leaves a resumable partial state.
7. **Never treat equality tokens as harmless metadata.** HMAC comparison tokens avoid unsalted guessing but leak repeat equality/change/access patterns and become enumerable if key leaks. Keep the key independent, versioned and access-controlled; do not place raw values or public unsalted hashes in events/indexes.
8. **Recovery is non-authoritative.** Cached/stale data can be served only when explicit class policy allows it and max age/revocation checks pass. It never authorizes a write or overwrites a provider; root/recovery/transient default deny offline stale use.

## Inventory and baseline protocol

### Inventory completeness is a typed result

One `InventoryResult` per provider/account/container/scope should include records, opaque pagination/cursor evidence, requested and granted scope, filter semantics, expected/received page count where available, provider snapshot/revision token where available, list start/end and consistency semantics, inaccessible field/attachment set, warnings, and a status:

- `Complete`: all pages and requested permitted fields enumerated; authoritative only for the exact declared scope and time interval.
- `Incomplete`: a page/filter/record/attachment/timeout/error was skipped or response was malformed. May support safe reads of observed records but cannot prove absence or advance complete-inventory watermark.
- `Inaccessible`: auth/scope denial, expired token, network/provider outage, permission loss. No absence claims.
- `ScopeChanged`: grant or configured scope differs from prior run; requires operator review and new baseline, not deletes.
- `Unsupported`: provider cannot enumerate a required record class/field. Preserve observed records; no lossless-sync claim.

The provider's list may not be a transactional snapshot. If no stable snapshot revision exists, use bounded start/end inventory, change cursor and second confirmation scan for missing candidates; record that a concurrent mutation window remains. A full inventory's completeness only proves what the provider exposed to the granted identity. It never proves unseen personal vaults are empty. Attachments/OTP/passkey reads have per-field accessibility, not blanket “record complete”.

Pairing is by prior native binding, verified stable global ID/reference, or explicit reviewed strong match. Title/path alone never merges. For independent same-name additions with no strong match, preserve both and generate conflict references (spec permits `.op`/`.bws` disambiguation). Rename/move is resolved by stable native ID and scope transition, not tombstone. Preserve exact provider IDs, native title/case, field names/order/types, URLs/URI matching, visibility, tags, attachments and unknown native attributes.

### Baseline is a transaction boundary

Store the last verified common baseline as an encrypted, versioned snapshot referenced from durable state, not a plaintext SQL row. It includes canonical typed field values only in a separately protected encrypted payload; metadata includes logical ID, per-provider native refs/revisions, mapping/policy versions, HMAC key version/tokens if enabled, complete inventory manifest, scope digest, verified timestamp, field observation states and event IDs. Do not put secret bytes in Supabase or an unencrypted local JSONL journal.

Advance algorithm:

1. Create `SyncRun` and record exact policy version, source scopes, prior checkpoint ID, time, caller/host, `full|incremental`, allowed mutation count/class and run ID. Persist `planned` state and an immutable plan digest.
2. Fetch both inventories independently. Validate pagination, field retrieval and scope. Persist encrypted observations and manifest. If either side is not `Complete` for deletion-sensitive reconciliation, set run `incomplete`; safe non-destructive independent work may continue only under policy, but no common baseline checkpoint.
3. Build three-way plan against checkpoint and create conflict records for ambiguous pair/field changes. A plan is immutable; approval is bound to plan digest, scopes, expires-at, max sensitive changes and exact operation set. Plan drift invalidates approval.
4. Execute each item saga in durable order (below). Persist step result/receipt after each external response. An operation in `unknown_outcome` is reconciled before retry.
5. Re-read changed provider objects and verify all projected typed fields, preserved native IDs, expected revisions, sidecar/attachment hashes and history references. Readback of unsupported/hidden fields remains incomplete, never inferred.
6. Append verified events. Flush required local durable journal. If remote index/outbox unavailable, queue immutable metadata-only events locally; whether provider writes may proceed is explicit policy. Do not proceed unjournaled.
7. Only when all required sources are complete and each required mutation is readback-verified, atomically write new encrypted baseline artifact, append checkpoint referencing it, and update `last_complete_run_id`/watermark in one local transaction. The SQL/index is secondary projection: it may lag and must be replayable from journal. If artifact and checkpoint cannot commit atomically, use temp-write + fsync + atomic rename, then checkpoint transaction referencing the content hash; orphan artifact is harmless and can be GC'd only after verified reachability.
8. Mark run `completed` only after required durable records are confirmed. Else `partial`, `incomplete`, `needs_review`, or `unknown_outcome`, with exact recovery actions.

Do not use `since-success` as absence proof. It is an optimization hint; periodic full scans (initial full, then weekly/periodic and after cursor/scope error) close missed-change gaps. Last common watermark advances across the **entire configured policy scope**, not whichever provider happened to answer. Keep per-provider observation cursors separate from common-baseline cursor.

## Reconciliation state machine

Run states: `created → inventorying → planned → awaiting_approval → applying → verifying → checkpointing → completed`. Alternate terminal/hold states: `plan_only`, `incomplete`, `blocked`, `needs_review`, `partial`, `unknown_outcome`, `failed`. Terminal states are immutable; replay creates a new run linked by `replay_of` and emits compensating events rather than editing the old run.

Per-item operation states:

```text
observed
  -> matched | unpaired | inaccessible | unsupported | missing_candidate
matched -> unchanged | one_sided_change | converged | conflict | moved_or_renamed
one_sided_change -> planned -> approved -> precondition_checked
  -> history_intent -> history_created -> history_verified
  -> write_intent -> write_accepted -> readback_verified -> event_durable -> baseline_eligible
any external intent -> unknown_outcome (timeout/crash) -> reconcile_receipt -> resume | needs_review
any precondition mismatch -> conflict / needs_review; never overwrite blindly
missing_candidate -> second_complete_scan -> delete_verified -> zz_delete_intent
  -> zz_delete_created -> zz_delete_verified -> retire_in_place -> retirement_verified
```

For each operation, record `operation_id`, idempotency key, logical ID, direction, action, exact source/target native refs and revisions, per-operation write-guard level, class/policy/mapping versions, before/after HMAC refs (never root-private or recovery material), planned effect, stage, provider request receipt, readback status, retries and recovery action. Retrying must preserve operation/idempotency keys. Use provider idempotency keys if available; otherwise deterministic client marker in restricted metadata plus post-write lookup. If neither idempotency nor lookup is supported, an unknown create result is not automatically retried: quarantine for manual search/review.

### Field-level semantics

- Compare values only after mapping version, type normalization and sensitivity policies match. Do not normalize away case/order/URI match semantics unless the mapping declares a reversible semantic normalization and tests prove equivalence.
- `Absent`, `Null`, empty string, inaccessible, redacted, unknown are separate states. Inaccessible is not a deletion or an instruction to erase peer data. Unknown native field values round-trip as encrypted provider envelope; if destination cannot represent/reconstruct them, block that item with `lossless_projection_unavailable`.
- A field update is not a whole-record replacement unless provider API forces it. If forced, merge freshly read target record and preserve unrelated current fields immediately before write. If provider lacks such capability, disclose and require conflict review.
- Sensitive fields (password, API token, access key, TOTP seed, private key, recovery code, passkey) default to preserve-both conflict and prohibit timestamp/LWW. Non-secret metadata may be auto-merged only when types are compatible, policy permits it, and neither side changed same field differently.
- For attachment content: identity includes protected ref, type, size and keyed or cryptographic content proof; inaccessible bytes block claiming equality. Re-read destination attachment after upload and verify hash under approved rules.
- In-place password update retains existing OP/BW login item ID, URI/autofill associations, folder/vault/container and provider-native record relationships. Native `revision` expected to change; the identity must not.

### `zz.delete` and revocation semantics

Keep separate lifecycle facts: `retired_superseded`, `deleted_on_source`, `moved`, `scope_removed`, `revoked`, `compromised`, `inaccessible`, `unknown`, `duplicate_invalid`. A verified provider deletion does not mean the secret is safe to reuse; a rotation history value is sensitive and may remain usable. `zz.delete` records are dated immutable history items with event ID, original logical/native ID reference, old/new relation, reason, provider, time and restricted access. They are not normal login suggestions: use archive/retired marker and exclude from autofill through provider-specific tested feature (or do not claim prevention if provider cannot guarantee it). Never physically delete history automatically.

For a missing candidate, require all: prior complete baseline with this exact scope/native binding; current inventory `Complete`; no item found across all pages/allowed folders; no scope movement; second authoritative confirmation (or provider-native deletion event/revision); plan evidence and approval policy. If old value exists only in peer/last verified encrypted baseline, history may be reconstructed there; if no recoverable prior value, record tombstone as `unrecoverable` and alert without fabricating it. History copy is created only to policy-approved replicas. Before retiring any active copy, ensure at least one valid recoverable history ref and verify both stores where policy requires both. `zz.delete` record itself is never auto-deleted.

## Durable ordering, crash recovery, and cross-provider semantics

External stores have no shared transaction. Use an item saga with write-ahead operation log and outbox; do not promise two-phase commit unless provider primitives actually exist. The required order for a stale password update in two approved stores is:

1. **Plan/approval:** dry-run plan includes exact source/target, old binding, expected revisions, field mapping, history destinations, max changes, rollback/recovery description; approval digest exact-match.
2. **Read/precondition:** read full current objects and encrypted baseline; verify source revision and target revision unchanged. If target/source changed since planning, replan. If provider lacks CAS, label race window and limit scope.
3. **Journal intent:** append/fsync local `history_create_intent` for each destination, reserve deterministic `zz.delete` logical/native identity and idempotency key. Required local journal must be durable before remote write.
4. **History writes:** create protected history copy in each policy-approved store; persist receipt immediately. If response lost, query exact deterministic history identity and verify content, do not blindly duplicate. Re-read each history item and verify old value under protected comparison and metadata/field completeness.
5. **Active update:** only after all mandatory history refs verified, journal `update_intent`; update stale active item **in place** using exact original native ID and expected revision/CAS. Preserve unrelated fresh fields. Record each provider separately.
6. **Readback:** retrieve the active item by stable native ID and verify mapped fields, exact stable item ID, expected new revision, URLs/autofill/custom fields and no clobbered concurrent change. Reconcile history linking.
7. **Durability:** append immutable field-change/history/write/verification events to local journal. Queue metadata-only SQL/index projection in outbox; idempotent key `(event_id, sink)`. Remote index acknowledgement is stored as receipt; replay safe.
8. **Checkpoint:** only after all required peers meet policy and full scope inventory is complete, publish encrypted baseline and checkpoint. If one provider fails, item remains `partial`, journal identifies committed side and retry stage; no all-sides success.

Crash table (all cases must be injected in tests):

| Crash/failure point | Recovery behavior |
|---|---|
| Before journal intent | No provider effect expected; operation can be replanned. |
| After intent, before request | Query provider by idempotency/marker; if absent, retry same key; if inconclusive, `unknown_outcome`, never create a second history item blindly. |
| Provider accepted request but response lost | Read by stable ID/marker and verify exact content/revision; append recovered receipt. Do not infer failure from timeout. |
| First `zz.delete` succeeds, second fails | Keep first verified history; no active update. Mark partial and resume second destination with original intent. Never compensate by deleting history. |
| All history succeeds, active update fails | Old active remains; history is retained (acceptable duplicate history), retry guarded update after re-read. Do not restore/delete history. |
| Active update accepted but readback fails | Mark unknown outcome; stop item; re-read later by native ID before any retry. Do not say update failed or apply stale write. |
| First provider update succeeds, second fails | Keep successful side, issue event, block baseline; retry remaining side only if current revision still matches and guard policy permits it. For unguarded sensitive fields stop for review unless separately approved weaker policy applies. |
| Journal append fails before side effect | Fail closed; no external write. |
| SQL/Supabase/outbox unavailable | Keep local immutable journal and queued metadata event; according to policy either safe provider writes continue with local fsync or stop. Never report remote index committed. |
| Crash during baseline temp write | Ignore unreferenced temp/orphan artifact; prior checkpoint remains authoritative. Verify content hash then atomic rename/checkpoint. |
| Scope change or partial pagination | No delete classification, no common watermark; keep observed subset as non-authoritative. |

No physical delete is used as “compensation”; compensation for a bad active write is a new forward correction with human review and new event. Event log is append-only; corrections and outbox delivery receipts are new events. Provider native version history is useful evidence but not assumed present/available.

## Outbox and data partitioning

Local journal (0600 file, 0700 parents) is the durable source-of-replay for operation intent and evidence. It may contain restricted identifiers and HMACs, never plaintext secret values or raw credentials. Encrypted baseline/history payloads are separate encrypted artifacts keyed by a key not stored inside the same encrypted payload. Use append-only event IDs, schema version, sequence number, previous-event chain digest and integrity check. A hash chain detects edits only if its head/verification key is protected externally; it does not prove authorship or protect confidentiality. Signed audit records require separate signer custody/rotation, not an unreviewed “sign everything” addition.

Supabase/Postgres, if adopted, is a **metadata-only projection**. Rows include run/event/operation refs, scope digest, policy/mapping versions, redacted action/outcome, revisions where allowed, conflict IDs, status, outbox receipt, nonreversible metadata-safe snapshot reference. Do not store plaintext secrets, raw attachment bytes, unkeyed low-entropy digests, auth tokens, or overly revealing item IDs/paths in public metrics. HMAC tokens are restricted metadata with leakage described above. Restrict RLS/service role; report-only role read-only. Local event journal remains replay authority; SQL rows are deduplicated projection by event ID. Never advance provider baseline merely because SQL insert succeeded.

## Concurrency, locks, and conflict policy

The laptop executor must prevent overlapping runs and credential rotation operations over intersecting scopes. Use an OS lock for same-host mutual exclusion plus durable lease/run ownership if multiple executors are possible. Locks reduce accidental overlap; they do not replace native CAS and do not serialize external human edits. Lease expiry cannot authorize automatic takeover of an operation with unknown provider outcome. A stale lock may be reclaimed only after inspecting persisted stage and reconciling receipts.

Plan concurrency: bind approval to scope+plan digest+expiry; any changed source revision or scope invalidates only affected item approval or whole plan (default whole affected item; whole plan if scope manifest changed). Enforce per-item preconditions only where declared guard level provides them. A Fleet lease/readback does not serialize external writers. For `unguarded`, conflict-sensitive updates stop as `needs_review` unless an explicitly approved weaker policy states its lost-update window in plan and receipt. Synthetic schedule preflight → external edit → apply; oracle: external value stays recoverable and operation reports conflict/unsupported guard, not success. Multiple independent items may proceed with bounded concurrency; shared run checkpoint commits only after all required effects verified. Respect rate limits and Retry-After, bounded retries with jitter; do not retry non-idempotent writes absent outcome reconciliation.

Conflict resolution choices include `preserve_both`, `choose_left`, `choose_right`, `manual_merge`, `keep_unpaired`, `mark_scope_moved`, `confirm_deleted`, `reject_plan`. Resolution is a typed human/system decision with actor and reason; protected-field `choose_left/right` requires refreshed observations and explicit plan approval. Timestamps cannot auto-resolve protected fields. Conflicts remain durable until a resolution event and verification completes.

## V3 issue-ready proposals for domain starter #9159

Stable proposals only; no GitHub issues created here. Each child below is one independently reviewable deliverable and follows coordinator handoff §4.2.

### REC-01 — Implement typed inventory completeness and protected baseline records

- **Kind / urgency / priority:** implementation+test; High, urgency 450, High. **Lifecycle:** Backlog until SEC-02/03 schema contracts; **estimate/stage:** M, V3 first. **Reqs:** spec §§20.1-20.3, 21.3-21.4, 36.1, 36.4; V3-S06/S10; addendum §1 lines 69-80.
- **Current evidence:** current resolver distinguishes BWS absence vs malformed list in `fleet-secrets/src/lib.rs:554-604`; broker validates list entries before cache install in `fleet-queue/src/secrets_broker.rs:147-188`, but there is no reconciliation inventory/baseline service. Broker list authority is not cross-provider delete evidence.
- **Deliverable / boundary:** `InventoryResult` with `Complete|Incomplete|Inaccessible|ScopeChanged|Unsupported`, per-field accessibility and page/cursor/scope evidence; encrypted baseline artifact and checkpoint metadata local-first. Synthetic providers only. No production inventory or database deployment.
- **Acceptance:** complete requires all pages/field reads; checkpoint references source manifest, exact scope/policy/schema/mapping, baseline hash and last complete run; incomplete run cannot advance common watermark; baseline round trip encrypted and redacted metadata. **Negative:** timeout/403/truncation/malformed page/scope shrink/missing attachment never yields delete or `Complete`; no plaintext value in journal/index fixture.
- **Deps:** SEC-02, SEC-03. **Rollback:** no live baseline migration; keep old state unused; orphan encrypted fixture ignored. **Recovery:** prior checkpoint remains authoritative; retain incomplete snapshot for retry. **Open:** exact encrypted artifact store and transaction mechanism selected with target host's fsync/atomic rename support documented.

### REC-02 — Build pure three-way field planner and conflict records

- **Kind / urgency / priority:** implementation+model-based test; High, urgency 440, High. **Lifecycle:** Backlog after REC-01/SEC-02; **estimate/stage:** M, V3. **Reqs:** spec §§20.3-20.4, 20.7, 36.2; V3-S03/S05/S06.
- **Current evidence:** no canonical reconciliation code in V1 (`fleet secrets` supports only get/list per `pkgs/fleet/crates/fleet/src/secrets.rs:1-19,73-79`).
- **Deliverable:** deterministic pure planner `(baseline,left,right,policy,mapping)->Plan`, per-field observed states, pair provenance and stable conflicts. No provider calls and no mutation. Native IDs/revisions retained; fields exact/reversible; timestamps informational only.
- **Acceptance/oracle:** generated three-way operation sequences are compared against a small independent reference model: one-sided field edits propagate only to unchanged peer; same-field divergent protected changes conflict; disjoint-field edits merge; equal concurrent values converge; no-op idempotent; unknown/inaccessible never delete; title-only match never pair. **Negative tests:** changed password on both sides with skewed timestamps preserves both; null vs absent vs empty remains distinct; unmapped field blocks full-sync claim; mapping version mismatch fails closed.
- **Deps:** SEC-02, REC-01. **Rollback:** pure planner has no side effects; disable new planner and preserve V1. **Recovery:** re-plan from last baseline and current complete inventory; no state to roll back. **Open:** auto-merge whitelist for nonsecret metadata must be explicit and tested.

### REC-03 — Add revision guards, idempotency ledger, and operation receipts

- **Kind / urgency / priority:** implementation+test; High, urgency 430, High. **Lifecycle:** Backlog pending SEC-03/REC-02; **estimate/stage:** M, V3. **Reqs:** spec §§8.3, 20.3, 21.3, 36.3; V3-S02/S04/S05.
- **Current evidence:** no provider writes/revision guard in current `fleet-secrets` (`Cargo.toml:9-12`; source lines 139-193). Spec §35.1 lines 2191-2203 explicitly allows emulated preflight only with race exposure.
- **Deliverable:** operation WAL records exact target native ref/revision, deterministic idempotency key, guard kind `CAS|preflight_then_verify|unsupported`, provider receipt and recovery action. Fake providers can simulate conditional/nonconditional writes.
- **Acceptance:** stale expected revision causes `PreconditionFailed`; retry with same idempotency key returns same logical operation; ambiguous timeout enters `unknown_outcome`; receipt distinguishes accepted/pending/readback-verified; no false atomic pair status. **Negative:** never blindly retry create after lost response when lookup unsupported; revision changes between plan and apply stop item; provider success with contradictory response isn't verified.
- **Deps:** SEC-03 and REC-02. **Boundary:** no real credentials/provider APIs. **Rollback:** no default mutation entry point until receipt/recovery contract accepted. **Recovery:** replay reconciles provider by stable ID/idempotency metadata before applying. **Open:** supported providers may have different CAS semantics; descriptor reflects version-tested evidence.

### REC-04 — Persist append-only local journal and metadata-only outbox

- **Kind / urgency / priority:** implementation+security review+test; High, urgency 440, High. **Lifecycle:** Backlog after REC-01; **estimate/stage:** M, V3. **Reqs:** spec §§21.1-21.6, 36.3-36.4; V3-S10; R-SEC-03/R-SEC-18.
- **Deliverable:** local fsync-before-write journal, immutable event schema, integrity chain, redaction, replayable SQL metadata outbox; event delivery receipts idempotently projected by event ID. No live SQL credentials needed for core tests; use fake sink.
- **Acceptance:** append and replay preserve sequence, no update/delete of old event, duplicate outbox delivery is idempotent, interrupted append is detected/truncated only to last valid frame with explicit recovery event; integrity chain verifies; redaction scanner finds no fixture secret/token/unkeyed low-entropy digests. **Negative:** remote sink outage never marks sink committed; journal write failure prevents provider side effect; corrupted event schema refuses replay; item refs omitted from metrics labels.
- **Deps:** SEC-02, REC-01. **Boundary:** do not deploy Supabase schema or sync actual inventory. **Rollback:** disable remote projection, retain local journal; schema versions remain readable. **Recovery:** replay immutable events to rebuilt index; backup/restore evidence required before production. **Open:** whether chain is hash-only or separately signed is a security-review decision; hash chain alone is not signer attribution.

### REC-05 — Implement protected `zz.delete` history lifecycle and in-place update saga

- **Kind / urgency / priority:** implementation+test+security review; High, urgency 450, High. **Lifecycle:** Blocked for live stores; Ready for synthetic test fixture only. **Estimate/stage:** L, V3. **Reqs:** spec §§20.4-20.6, 36.2; addendum §1 lines 7-11; V3-S04/S08/S25.
- **Current evidence:** no write/reconcile surface in V1; expected algorithm is specified at spec §20.5 lines 1154-1162. Intake explicitly disallows live reconciliation in this phase.
- **Deliverable:** append-only history identity generator and state-machine saga. Create encrypted dated history records in all policy-approved replicas, verify each, then update original stale item by native ID with revision guard, verify and link event. Tombstone/reason types distinguish deleted/moved/revoked/inaccessible/unknown. No generic delete API.
- **Acceptance:** synthetic providers prove stable native ID/URL/autofill fields remain; history old value verifies before active update; history never physically deleted; run resume doesn't duplicate history; partial first/second provider history blocks active update; provider failure leaves explicit partial state; uncompromised old secret is not presented as active. **Negative:** no title match overwrite; no timestamp-winner; no provider delete call exists in normal transition; history store inaccessible prevents required active update; missing prior value records unrecoverable tombstone without fabricated value.
- **Deps:** SEC-02/03/07, REC-02/03/04. **Boundary:** live `zz.delete` writes, root materials, production credential rotation and any full inventory are explicit approval-blocked. **Rollback:** forward corrective event only; never delete history as rollback. **Recovery:** resume from persisted per-step receipt and re-read before each action. **Open:** retention/access roles for obsolete credentials require operator review; no automatic purge.

### REC-06 — Add authoritative deletion-candidate confirmation and scope-move handling

- **Kind / urgency / priority:** implementation+model-based test; High, urgency 430, High. **Lifecycle:** Backlog after REC-01/02; **estimate/stage:** M, V3. **Reqs:** spec §§20.3-20.4, 20.6, 36.2/36.4; V3-S06.
- **Deliverable:** pure candidate state machine using baseline binding, exact scope, complete inventory, pagination and second confirmation/native deletion evidence; mark `missing_candidate` until sufficient evidence. Scope access loss/move is separate status.
- **Acceptance/oracle:** MBT model enumerates observation sequences and expects no tombstone before complete confirmation; after confirmed deletion, creates tombstone/history plan but never physical delete; move between authorized scopes preserves logical identity. **Negative:** timeout, 403, partial page, rate limit, cursor reset, provider outage, revoked token, out-of-scope relocation never produce `deleted`; reappearance is resurrection/new-generation conflict, not silent un-delete.
- **Deps:** REC-01/02, SEC-03. **Rollback:** candidate logic can be disabled and status remains `unknown`. **Recovery:** full inventory in same authorized scope and human review. **Open:** provider-native change feed may replace second scan only where verified semantics are adequate.

### REC-07 — Add guarded apply, crash injection, and resumable item saga

- **Kind / urgency / priority:** implementation+test; High, urgency 440, High. **Lifecycle:** Backlog after REC-03-06; **estimate/stage:** L, V3. **Reqs:** spec §§20.2-20.5, 21.4, 48.1-48.3; V3-S04-S06/S25.
- **Deliverable:** apply immutable plan only with exact approval; durable ordering, bounded concurrency/rate limits, crash recovery at every external boundary, item-level resume, no cross-provider atomicity claim.
- **Acceptance:** injected crash at before intent/after intent/lost provider response/partial history/update/readback/journal flush/outbox/baseline commit resumes without duplicate write, overwrites, event mutation or watermark advance. Verify complete readback before checkpoint. Approval tied to digest+scope+expiry; changed revisions invalidate approval. **Negative:** no `force` bypasses policy; no non-idempotent retry on unknown outcome; a failed required store prevents `completed`; independent safe items continue only under configured policy.
- **Deps:** REC-01..06, SEC-03/07. **Boundary:** synthetic providers and fixture data. **Rollback:** feature flag leaves planner/apply disabled; saga can only recover forward; stop scheduler. **Recovery:** explicit report of committed effects and next action per operation. **Open:** transaction/journal guarantees and systemd lock implementation verified on target laptop before daily schedule issue is unblocked.

### REC-08 — Add full-scan vs incremental scheduling and verified checkpoint policy

- **Kind / urgency / priority:** design+implementation+ops test; Standard, urgency 360, Standard. **Lifecycle:** Backlog until REC-01/07 and first-run approval gate; **estimate/stage:** M, V3 late. **Reqs:** spec §§20.1-20.3, 21.2/21.4, 36.4; addendum §1 line 80.
- **Deliverable:** reusable CLI/library scheduling state; exact scope lock, persistent after sleep/reboot, full inventory cadence, lookback as hint only, bounded retries and alerts. Timer only invokes library; do not invent alternate schedule surface without registry update (AGENTS.md Fleet schedules).
- **Acceptance:** synthetic restart/resume keeps last complete baseline; full scan resets invalid cursor; overlap runs prevented; lock loss does not permit takeover of unknown operation; timer tests don't access live secrets. **Negative:** no watermark advancement after incomplete run; no destructive branch on stale incremental cursor; no daily production enable before reviewed initial full migration and explicit approval.
- **Deps:** REC-01/07 and #9165 schedule/ops alignment. **Rollback:** disable timer and retain journal/checkpoint. **Recovery:** manual `plan-only` full scan in authorized scope. **Open:** daily local time and schedule ownership deferred to implementation/ops decision.

### REC-09 — Define cache/revocation coupling for reconciled writes and replicas

- **Kind / urgency / priority:** implementation+security test; High, urgency 420, High. **Lifecycle:** Backlog; remediation of V1 #6767 may precede other V3 work; **estimate/stage:** M, V1/V3 bridge. **Reqs:** spec §§20.3, 36.4, 37.2; addendum §9 lines 99-102; V3-S07/S09.
- **Current evidence:** broker has stale-while-revalidate, retains prior values for failed per-secret fetches and exposes age (`fleet-queue/src/secrets_broker.rs:244-266,280-346,532-545`); intake note calls out rotation stale cache #6767.
- **Deliverable:** write-through invalidation/refresh by logical/native binding, revocation status propagation, explicit cache provenance and class max-age; encrypted offline snapshot policy only if independent bootstrap/recovery tested.
- **Acceptance:** successful authorized rotation cannot return old value under freshness-required policy; invalidation failure marks `stale/unknown` and refuses; allowed stale read includes source, revision, age and policy reason; provider conflict forbids fallback; cache never authorizes write. **Negative:** outage does not reset age; root/recovery/transient denied; revoked marker cannot be overridden by warm local cache.
- **Deps:** SEC-08, REC-03/07 for reconciliation path; isolated broker bug fix can run against #6767. **Rollback:** fail closed/live-only. **Recovery:** authoritative provider read + cache eviction. **Open:** cache TTL/max-age by classification chosen after synthetic benchmarks and explicit security policy.

## Verification strategy: PBT, MBT, and explicit oracles

Use property tests for pure invariants and model-based tests for lifecycle. Do not rely on a pile of hand-written happy paths.

- **Planner PBT:** random baseline/left/right typed fields and revisions. Oracle is independent reference merge function. Invariant: no output write when neither side changed; one-sided changes only write unchanged peer; divergent two-sided protected fields always conflict; unknown/inaccessible never become delete; apply twice after verified receipt is idempotent.
- **Inventory PBT:** generator varies page count, cursor uniqueness, malformed row, truncated response, auth/scope errors, attachment availability. Oracle is explicit completeness predicate. Invariant: `Complete` iff every required page/field is observed under same declared scope; any failure blocks absence proof.
- **Operation MBT:** reference state machine drives provider fake and durable journal through create-history/update/readback/checkpoint, lost response and restart actions. Oracle is expected model state per operation. Invariant: active update never precedes required verified history; baseline never advances before all required readback; no unknown result gets blind retry; event history is append-only.
- **Identity/fidelity PBT:** generate arbitrary native unknown fields, order, Unicode, null/absent/empty, URLs, type tags, attachments and ID revisions. Oracle is exact source fixture round-trip or semantically normalized fixture where explicitly declared. Invariant: native ID stable through update; unknown fields preserved or explicit lossiness conflict.
- **Redaction oracle:** insert known synthetic secret/token/low-entropy marker strings into values/errors/provider stdout; scan journal, logs, errors, JSON summaries and SQL projection; assert absent. HMAC tests confirm no unkeyed digest emitted and key-version mismatch does not equal “changed”.
- **Crash matrix:** use deterministic fake store fault injection before/after each request and response, journaling/fsync, readback and checkpoint. Oracle is model's committed effects and durable stage; tests assert no duplicate `zz.delete`, no unsafe overwrite, no false completed run.

Acceptance references: spec §20.5 and §§21.1-21.6 lines 1154-1244, §36.2-36.4 lines 2230-2247, §48 gates V3-S03-S10 lines 2651-2666. No test result or benchmark is claimed here.

## Related source and code evidence

- Spec §20 details: `sources/fleet_architecture_spec_v0_3.md:1087-1166`; §21 journal/outbox/schema `:1168-1244`; §36 update `:2221-2247`; §48 gates `:2637-2695`.
- Addendum reconciliation/failure model: `sources/ADDENDUM_secrets_architecture_performance_infrastructure.md:5-11,69-80,82-92,99-103`.
- Coordinator safety/acceptance contract: `sources/COORDINATOR_summit_handoff.md:102-122,196-199,267-273`.
- Intake authorization and current census: `note.md:24-50`.
- Current code facts: `pkgs/fleet/crates/fleet-secrets/src/lib.rs:1-28,139-193,517-604`; `pkgs/fleet/crates/fleet/src/secrets.rs:1-19,73-79`; `pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs:1-39,147-188,190-266,280-346,348-440,532-577,611-624`.

## Explicit non-goals / approval blockers

- This design run must not read live vault data or perform production writes, reconciliation, `zz.delete` creation, rotation, root key generation/export/signing, DNS/trust/deployment changes. Tests use fixtures or explicitly isolated authorized test stores in a future implementation issue.
- No production scheduler enablement before an approved initial plan and successful failure-injection, restoration, audit, permissions, and rollback gates.
- No physical-delete API in routine reconciliation; no garbage collection of `zz.delete`; no unsalted equality hash; no guarantee that a multi-provider saga is atomic.
- No treating OP/BWS inventory as complete for personal OP/BW vaults; `bw` Sweep B remains separately authorized.
