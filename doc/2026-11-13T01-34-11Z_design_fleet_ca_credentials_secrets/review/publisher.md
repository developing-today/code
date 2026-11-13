# Independent publisher review

**Decision: AGREE — the publisher is ready for a reviewed, fixed manifest, conditional on the exact canonical manifest hash being structurally validated, its dry-run plan being accepted, and proposal-content reviews being accepted separately.** This is producer/readiness approval only: it does not approve the current 154-proposal snapshot, certify complete source coverage, or attest live GitHub/queue behavior.

## Scope and verification

Reviewed the full current `tools/publish_issues.py`, `tools/test_publish_issues.py`, `evidence/issue-pipeline.md`, coordinator handoff §§4.2/5, source-backed `fleet gh` retry policy, queue validator and coverage contract. Verified the exact source snapshot by SHA-256: publisher `e85e32d550418b72d6c67489ea35874596ed1c76e8b1070546facba5f30a93a5`; tests `486b40e5384201cf450c7c2691ab541074c8184ceac0cc07ce3ba6150dca503e`; evidence `45abaafda59debdd7392a75b5814899bb70e53ddcba70564840d3ec5f85f4425`. Ran `python3 -I -B doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/tools/test_publish_issues.py` against those actual files: **48 tests passed**. No live GitHub, timeline, `fleet gh`, or queue-service calls were made by this reviewer; mocked tests do not establish live service behavior.

### Findings

1. **RESOLVED — creation is a single API operation and ambiguous outcomes stay quarantined.** `publish_issues.py:973-1004` creates with exactly one `fleet gh api --method POST repos/developing-today/code/issues --input <metadata.json>` containing only title/body/labels; native parent/blocker links are separate operations. A fsynced `create_intent` precedes the POST (`975-980`), `create_result` is recorded immediately on a valid response, and zero indexed marker hits cannot authorize retry (`965-968`). Tests assert the API request shape and no compound `--parent`/`--blocked-by` flags (`test_publish_issues.py:556-593`), plus zero-hit replay creates no second POST (`595-622`). `fleet gh` retries a single API mutation only after a classified pre-execution refusal; this review made no live write.

2. **RESOLVED — receipt data is not authoritative for issue IDs.** `read_receipt` rejects duplicate keys and validates event schemas/order; replay reconstructs IDs from fresh marker/API reads. The digest-scoped receipt path rejects symlinks, hard links and tracked targets; append uses no-follow descriptor traversal and fsync. Existing mocked tests exercise malformed/forged receipt and destination cases.

3. **RESOLVED — durable create/queue intent and replay behavior.** Issue and queue intents are recorded before their single mutations; resumes with an intent but no response do read-only reconciliation and never issue a second create/enqueue. Receipts distinguish ACK from independently verified issue/task records.

4. **RESOLVED — truncated relevant queue evidence fails closed.** `find_queue_task` raises indeterminate when a candidate is in a capped 500-row scan (`876-906`); an unrelated capped status may be followed by a separate complete live-status scan. It accepts only one exact same-row ID/full-dedupe-key/live-status observation. `--task` remains diagnostic only. Tests cover a matching row at cap and the complete-other-status case (`test_publish_issues.py:461-491`). Title is not proof; a truncated/mismatched dedupe key cannot match. Queue receipt records scan scope/count/IDs/limit/truncation and event timestamp, while not-found/malformed/mismatched results remain unverified and resume is read-only.

5. **RESOLVED — existing child marker close/reopen conflicts; epic reopen is separately allowed.** The publisher paginates bounded issue-event history and fails closed on incomplete/malformed history (`354-404`); a reused child with any close/reopen history is refused before repair/enqueue. Tests exercise pagination and the open-child conflict (`test_publish_issues.py:519-542`). The program epic #9155 remains governed only by its current OPEN pre-mutation gate; the test proves its authorized reopen history is not run through the child-marker gate (`650-653`).

6. **RESOLVED — exact native parent and blocker read-back validates every row.** `parent_children` validates positive unique issue numbers; issue metadata parent pointer and parent child list must agree. `validate_dependency_rows` requires each dependency row to have a positive issue number/database ID, rejects duplicate IDs and foreign canonical URLs, then the final pair set must exactly match intended blocker numbers/database IDs (`424-447`, `1040-1098`). Tests exercise malformed/missing/duplicate/foreign entries and unexpected edges (`test_publish_issues.py:940-990`).

7. **RESOLVED — full-manifest text validation precedes effects.** `validate_text` recursively rejects U+0000 and values that are not strictly UTF-8 encodable, including keys and nested values (`publish_issues.py:112-130`); it is the first operation in `validate_manifest` (`133-136`), before digesting, temporary files, receipt writes, or subprocesses. The regression test uses a two-proposal batch and covers title surrogate, title/body NUL, source surrogate, and unsafe key, asserting controlled refusal with no subprocess/network/temp/receipt effects (`test_publish_issues.py:203-225`). The CLI path itself also rejects NUL/surrogate argument text (`163-177`).

8. **RESOLVED — a committed parent-link response loss resumes without a second issue create.** `create_result` is durably recorded before the native parent link (`1037` then `1099`); resume uses the recorded number (`995-1000`) and reconciles the live pointer/list. The new mocked test commits the parent edge, loses the response, reloads the receipt and resumes; it asserts exactly one create API call and one parent POST (`test_publish_issues.py:652-727`). The create payload is only title/body/labels, while parent/blocker links stay separate.

## Manifest snapshot limitation

No final canonical publication manifest or proposal digest is present in this review. Do not treat the 154-proposal normalizer output as final. Approval is limited to a later exact frozen canonical manifest SHA-256 whose dry-run plan, structural validation, and separate proposal-content reviews are accepted. This review does not certify complete source coverage or approve the current proposal set.

## Verified behavior that passed this audit

- CLI uses `allow_abbrev=False`, rejects unknown args, requires `--receipt` for `--publish`, restricts the queue, and defaults to a no-subprocess dry run.
- Exact proposal keys, duplicate manifest JSON keys, duplicate IDs, allowed parents (excluding #9161), dependency references/cycles, and urgency mapping are checked before publish-side subprocesses.
- Intrinsic urgency 450 maps to `priority/high` and queue priority 20 as distinct values; native API writes use database IDs rather than issue numbers.
- Initial queue work is `triage`, not implementation, uses the live canonical issue-number dedupe family and pilot queue, validates packets, and confirms a live queue state before recording `task_verified`.
- The source-backed `fleet gh` wrapper refuses to replay ambiguous writes; its permission/rate-limit failover is restricted to classified pre-execution refusal. The publisher's issue-create phase is one API mutation with no compound convenience flags.
- The publisher does not write Project fields, consistent with the single-Project-writer boundary.
