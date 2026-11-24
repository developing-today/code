# `ci.events` webhook ledger on Turso — design and closeout

- **Date:** 2026-11-24
- **Status:** design record; migration/cutover implemented and verified by PR #10047
- **Tracking:** [developing-today/code#5693](https://github.com/developing-today/code/issues/5693)
- **Scope:** move the CI webhook event ledger from Supabase Postgres to Turso, preserve its reader behavior and delivery semantics, define retention, and state the separate archive-drop gate. This record does not authorize or perform any data movement or destructive operation.

## 1. Problem and context

The original `ci.events` table in Supabase project `muse-dev2` was an append-only GitHub CI webhook ledger: at the 2026-11-04 inventory it held 10,952 live rows / 31.8 MB, with 10,957 inserts and no updates in the observed statistics window. It had no foreign keys and no transactional consumer. Its `ci.recent_failures` view served the morning readout's last-24-hour failure summary. This is stream-shaped data, not relational system-of-record state, and the project's fastest-growing table by bytes. The placement rule in the Atlas scaler-era design therefore classifies it for Turso.

The write path is operationally sensitive: webhook delivery must not be acknowledged if the only durable write fails, and deploy/cutover gaps must be detectable. The reader must remain useful while the source and target differ. The issue's initial sketch proposed dual-write, backfill, reader comparison and flip, a 90-day hot window with cold export, and a read-only Postgres interval before any drop.

Historical evidence matters: PRs #6921/#6923 moved the receiver to a Cloudflare Worker and began Turso writes; the reader and webhook cutover followed on 2026-11-07. PR #10047 (merged `3b4cb177517ec1d6f8881083cd6694553b223a2a`, 2026-11-14) recorded the closeout, added the 90-day prune script, and verified the remaining Postgres archive state. The Postgres table is still retained; the archive drop is deliberately separate.

## 2. Goals and non-goals

### Goals

- Put ongoing CI webhook ledger writes and the live failure-summary read on Turso, removing their runtime dependence on Supabase Postgres.
- Preserve authenticated webhook handling, idempotency, queryable failure fields, and the policy of retaining raw payload only for failure rows.
- Make retention explicit and bounded in the hot ledger, with deletion only after the configured age boundary.
- Provide evidence for parity, write silence, reader freshness, and rollback readiness.
- Keep the legacy Postgres rows intact until the independent retention/approval gate is met.

### Non-goals

- No Postgres table/view drop, schema mutation, or archive deletion in this design pass.
- No attempt to migrate unrelated `agent.*` streams or change the queue backend.
- No guarantee of recovering webhook deliveries that GitHub did not successfully redeliver during the historical receiver transition gap; that gap is documented, not concealed.
- No general analytics warehouse or change to GitHub's event subscription policy.

## 3. Alternatives and tradeoffs

1. **Keep the ledger in Postgres.** Lowest migration risk and preserves the old view, but leaves append-only, rapidly growing stream data in the relational system of record and retains the Postgres project/runtime dependency.
2. **Turso mirror while Postgres remains authoritative.** Good transition safety and supports parity checks, but doubles writes and does not remove Postgres dependence until writer and reader are both cut over.
3. **Turso as the live ledger; Postgres retained read-only as archive (chosen).** Matches the stream's access pattern, gives the live reader and writer one home, and keeps a bounded rollback/evidence window without coupling the final archive deletion to the cutover. Requires explicit dedupe, freshness monitoring, retention enforcement, and a separately gated eventual drop.
4. **Per-day/per-repository physical databases or tables.** Rejected: excessive object/table cardinality and cross-partition query complexity. Use one Turso table with date and repository fields/indexes; retention/export can operate by day.

## 4. Chosen design and contracts

### Data and identity

The Turso target is `fleet-platform.ci_events`. The logical record carries `id`, `received_at`, `source`, `event_type`, `action`, `repo`, workflow/run identifiers, branch/SHA, status/conclusion, actor, URL, CI timestamps, payload, and `dedupe_key`. The dedupe key is the stable identity across Postgres and Turso; target-local numeric IDs are not cross-database identity. A unique constraint/index on `dedupe_key` is required for `INSERT OR IGNORE` to be idempotent.

The ingestion policy remains completed `workflow_run` events only, excludes `conclusion = skipped`, retains full webhook payload only for failure rows, and stores `{}` otherwise. The receiver validates GitHub's HMAC-SHA256 signature over the raw request body before accepting events. Turso credentials are Worker secrets, not repository configuration.

### Writer and failure behavior

The live receiver is the Cloudflare Worker `github-ci` (`pkgs/github-ci/worker`). It writes `workflow_run` records to Turso only. A successful HTTP response means the durable Turso insert succeeded or the event was an idempotent duplicate; an insert failure must fail delivery so GitHub can retry. The Postgres Edge Function was retired after webhook target verification. Fleet push/issues/pull-request events use the fleet queue sink as implemented; that path is adjacent receiver behavior, not a second CI ledger.

### Reader and retention

The morning day-start readout queries Turso for the latest ten failed runs in its 24-hour window, ordered by `received_at`, and checks latest-row freshness. The query is keyed on timestamp/conclusion and does not depend on matching Postgres IDs. Keep 90 days hot in Turso. The prune tool (`pkgs/github-ci/worker/scripts/prune-ci-events.py`) reports counts and deletes rows beyond the 90-day cutoff; run it on the fleet maintenance cadence. Any future cold export must verify the exported row count before deleting corresponding hot rows; export failure must prevent deletion.

### Archive boundary

The intended state is that the Postgres `ci.events` table remains read-only and is not part of normal serving. Its last recorded write was `2026-11-06T18:00:26Z`; PR #10047's closeout records 11,203 Postgres dedupe keys, zero missing from Turso after backfill, and Turso as a strict superset. The archive may not be dropped before **2027-02-04** (90 days after last write) and requires separate recorded approval from Drew. This design itself is not that approval. There is a documentation conflict to resolve before any archive action: the issue/PR closeout says the table remains in place, while the current Worker README says the old database project has since been deleted. Do not infer that the archive exists or authorize a drop from either statement alone; verify the live inventory through the authorized operator path.

## 5. Dependency-ordered slices and acceptance

| Order | Slice | Acceptance |
|---|---|---|
| 1 | Inventory receiver, reader, schema, retention, and consumers | Receiver and sole evidenced live reader identified; source view/query and existing retention discrepancy recorded. |
| 2 | Build Turso schema/receiver path and idempotency | Signed workflow payload is accepted; invalid signature is rejected; duplicate `dedupe_key` creates one row; failure-only payload policy is tested; Turso write failure is not acknowledged as success. |
| 3 | Backfill and shadow/dual-run evidence | Source keys are represented at target; compare dedupe-key sets and record any transition gap explicitly. |
| 4 | Flip reader and webhook producers; retire old receiver | Live readout uses Turso and freshness check; all relevant webhooks target Worker; Postgres write silence and post-retirement Turso writes are independently observed. |
| 5 | Enforce retention and document archive gate | 90-day prune implementation exists and is runnable; first execution is non-destructive when no rows are old enough; Postgres drop remains excluded pending both gates. |

Slices 1–5 have been implemented/verified in the history summarized above; PR #10047 is the implementation closeout evidence. The remaining Postgres archive drop is not a sixth automatic slice: it is a later, separately approved decision after the stated date gate.

## 6. Rollout and rollback

The safe rollout sequence is: provision target and unique dedupe index; deploy receiver dark and replay signed historical-shaped payloads; run a low-traffic parallel webhook and compare dedupe-key sets; flip production webhook targets; verify real Turso writes; flip the readout and check its freshness guard; retain the old receiver as rollback during the observation window; then retire it only after the Turso path has live proof. Postgres remains untouched/read-only throughout.

If the Worker or Turso path fails, route webhook targets back to the retained Postgres receiver only if Postgres is healthy and its deployed function is available; otherwise restore the Worker from `pkgs/github-ci/worker` and use GitHub delivery redelivery. Reader rollback is to the Postgres source only while that source is confirmed current; do not silently present stale Postgres data as current. Since the Edge Function has been deleted and Postgres has been unavailable, redeploying the Worker is the current practical recovery path; rollback claims must be revalidated against live service state before action.

Retention rollback is to disable pruning, not to widen the cutoff or drop the archive. A failed export (if cold export is later introduced) blocks deletion.

## 7. Verification plan and recorded evidence

- **Authentication:** signed/replayed payload succeeds; bad signature returns 401 (recorded in #5693 history and Worker tests).
- **Idempotency:** replay same completed workflow payload twice and prove one ledger row; Worker tests cover duplicate insertion.
- **Write/read live proof:** after Edge Function retirement, a new Turso row was observed; the readout query ran against Turso and the freshness guard passed.
- **Parity:** 2026-11-14 dedupe comparison found four late Postgres rows absent from Turso; those were backfilled and a rerun found zero Postgres keys missing. Turso was a strict superset.
- **Cutover:** all four fleet webhook endpoints were verified on the Worker; Postgres write silence was observed from `2026-11-06T18:00:26Z` while Turso continued receiving events.
- **Retention:** first prune run on 2026-11-14 deleted zero rows because none had crossed 90 days; inspect counts and cutoff behavior on each maintenance run.
- **Sources:** #5693 issue history; #5690 companion design/runbook; `doc/fleet/agent-events-turso.md` for adjacent event-sink context; `doc/fleet/queue-turso-cutover.md` for the earlier cutover boundary; `pkgs/github-ci/worker/README.md`, tests, and prune script; PR #10047.
- **Evidence discrepancy:** the Worker README says the prior database project was deleted, but #5693's PR #10047 closeout says the Postgres table remains read-only and gives the archive-drop gate. No live database inventory was performed for this design task; preserve the no-drop rule and verify actual archive state before any future action.

## 8. Open questions and named deciders

1. **May the Postgres `ci.events` archive be dropped? — Drew.** Not before 2027-02-04 and not without a separate explicit approval recorded on #5693. Until both conditions hold, retain the table and do not run a drop.
2. **Should Turso history older than 90 days be exported to cold storage? — Drew.** The live design establishes 90-day hot retention; cold export is not required for current cutover acceptance. If required, decide its destination, retention, and restore proof before enabling deletion of exported rows.
3. **What is the actual current state of the Postgres archive? — fleet operator, with Drew deciding any drop.** Reconcile the Worker README's project-deleted statement with the issue closeout's retained-table statement using a fresh live inventory. Until reconciled, make no destructive change and treat the table as protected by the 2027-02-04 + explicit-approval gate.

## 9. Change log

- 2026-11-24: consolidated the original placement/cutover intent with verified implementation state and the still-gated Postgres archive decision; no operational changes authorized or performed by this document.
