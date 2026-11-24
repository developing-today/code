# Design: move `agent.events` and `agent.quota_records` to Turso staging (#5690)

- **Date:** 2026-11-24
- **Status:** Design record; implementation state and live cutover gates are distinguished below.
- **Tracking:** [developing-today/code#5690](https://github.com/developing-today/code/issues/5690); queue slice 21669.
- **Related records:** [agent.events Turso sink and cutover](fleet/agent-events-turso.md), [queue Turso cutover](fleet/queue-turso-cutover.md), [Turso recovery coverage](2026-11-18_turso-recovery-coverage.json).

## 1. Problem and context

The append-only `agent.events` and `agent.quota_records` streams originated in Supabase Postgres, while the queue and its operational consumers are moving to Turso. Keeping writes, readers, retention, backup, and historical data split across stores risks missing audit events, breaking event-based parked-task readiness, or presenting incomplete health/quota views. The existing `doc/fleet/agent-events-turso.md` records the event-stream inventory, implementation, and cutover procedure; the recovery ledger tracks the PG outage and backup-covered windows. This design consolidates the target architecture and remaining safe slices without treating historical build notes as proof of current live state.

## 2. Goals and non-goals

**Goals**

- Keep queue transitions and their event records in the same Turso broker/database dispatch; preserve explicit failure semantics for external writers.
- Make event-based readiness, health, quota reads, retention, and backup behavior explicit at the destination.
- Preserve historical PG records as a separate archive with source IDs and provenance; never mix PG and Turso ID spaces.
- Make cutover reversible until source/destination completeness and backup coverage have been independently verified.

**Non-goals**

- Rewriting the queue state model or changing existing task/event meaning.
- Dropping PG tables, disabling remaining PG jobs, changing production defaults, or claiming a cutover complete as part of this design-only slice.
- Importing or exposing secret-bearing event/quota payloads in logs or issue comments.
- Treating an unavailable PG source or an old count as proof of destination completeness.

## 3. Alternatives and tradeoffs

1. **Keep PG as the permanent source (do nothing).** Lowest immediate migration risk, but leaves queue operations dependent on the split store and keeps PG recovery/decommission as a permanent operational dependency. Rejected as the target; acceptable as a temporary fail-safe while gates are unmet.
2. **Dual-write, then cut readers/writers to Turso with a separate PG history table.** Chosen. It allows bounded validation and rollback before final source decommission; costs temporary duplicate writes and explicit reconciliation. Historical IDs remain isolated from live IDs.
3. **Big-bang export/import and switch.** Fewer intermediate states, but a missed watermark, partial export, or unobserved external writer could silently lose audit history. Rejected because the source has outage/recovery uncertainty and the streams have multiple writers/readers.
4. **Mirror only the queue's own writes and leave external writers/readers on PG.** Simpler broker path, but leaves webhook/snapshot writes, readiness checks, health, backup, and quota consumers split. Rejected as an incomplete end state.

## 4. Chosen approach and contracts

Use the existing Turso queue database as the live append destination. Queue transition events are written by the broker alongside task updates; external writers use explicit `pg | dual | turso` selection with the existing PG-safe default until cutover approval. Event readers use broker endpoints for append/read/count/prune/health, including a destination-side `events-seen` operation for `event_seen` readiness. Quota records follow the analogous append-only staging pattern and retain their own query/retention contract; they are not merged into `agent.events`.

For PG history, export only through a verified watermark to a distinct archive table whose primary key preserves the PG ID. A history import is idempotent and count/checksum reconciled before it is called complete. The live tables retain Turso IDs and do not acquire foreign keys to PG task IDs. Readiness/health must fail closed or report an explicit unresolved result when the destination or source evidence is unavailable; no fallback may silently resurrect stale PG state.

Security: payloads remain trace-safe (no event/quota row content in routine logs); credentials are resolved by name and scoped to the target database; only approved internal routes can read secret-tier data. No cutover or destructive source action is implied by this document.

## 5. Dependency-ordered build slices

Slices below describe the complete migration boundary. Existing implementation records may satisfy a slice, but each must be rechecked against its stated acceptance evidence before being counted done.

1. **Inventory and contract freeze.** Enumerate all writers, readers, retention jobs, backups, schema/ID relationships for both streams. **Acceptance:** dated inventory names owner, transaction semantics, failure behavior, and evidence for every producer/consumer; no unclassified endpoint remains.
2. **Turso append/read primitives.** Build idempotent schema and broker operations for append, readiness lookup, counts, prune, health, and quota-specific access. **Acceptance:** automated tests cover validation, authorization, idempotence, query bounds, and explicit errors; local contract tests show no PG-ID/Turso-ID join.
3. **Writer dual-feed and observability.** Add opt-in dual mode for every external writer while retaining PG default. **Acceptance:** a bounded observation window shows each expected event/quota kind at the destination and reconciles source/destination counts for the same interval; failures are surfaced and do not silently drop writes.
4. **Reader and readiness cutover.** Move health, event-seen readiness, and quota consumers to destination APIs. **Acceptance:** shadow/observe comparison agrees for the agreed window; each parked readiness case is either released correctly or remains explicitly unresolved; destination outage behavior is tested.
5. **Historical export and recovery proof.** Export to isolated history tables using a verified backup or green source window and a recorded watermark. **Acceptance:** rerun is idempotent; source/destination counts, ID bounds, nullability, and canonical row digests reconcile; provenance and backup source are recorded without payload disclosure.
6. **Writer flip and source retirement proposal.** Switch external writers only after slices 1–5 pass; propose PG cron/table retirement separately. **Acceptance:** full observation window passes, backup/restore ownership is documented, rollback switches are exercised, and the issue records evidence. No source drop occurs in this slice.

## 6. Rollout and rollback

Roll out by stream and writer: provision destination, deploy dual-write, observe, switch readers, verify history, then flip each external writer to Turso. Keep PG writes and readers available during the agreed overlap. If destination health, reconciliation, readiness, or backup checks fail, set the affected writer back to PG and restore the prior reader route; preserve both stores for reconciliation and do not delete either side. Any cron disable or source-table retirement requires a separate explicit evidence review after recovery and backup coverage are proven.

## 7. Verification plan

- **Automated:** schema/endpoint tests; queue transition plus event behavior; readiness oracle tests; quota read/write tests; idempotent history import; failure-injection for destination loss and partial dual-write; `git diff --check`.
- **Operational:** count and time-window reconciliation by stream/kind; source/destination watermark and digest comparison; health/readiness shadow comparison; backup restore test; audit of every writer's selected mode and every cutover timestamp.
- **Safety oracle:** a missing or mismatched destination is reported as incomplete/unresolved and cannot trigger source deletion or an implicit exposure fallback.
- **Evidence boundary:** use metadata/counts and approved checksums for verification; do not copy row payloads into logs, chat, or issue comments.

## 8. Open questions and deciders

- **What exact observation interval and tolerances qualify as count parity for each stream?** Decider: Drew (service owner), with the fleet/Atlas operator supplying measurements.
- **Which verified backup snapshot is authoritative for the PG history export, and who signs its restore proof?** Decider: Drew; backup operator supplies the snapshot and restore evidence.
- **When may PG cron jobs be disabled and the frozen source tables be retired?** Decider: Drew, only after history, backup, and rollback evidence is attached to #5690.
- **What is the final quota-record retention window and which consumers require historical reads?** Decider: Drew; implementation owners provide consumer evidence before slice 2 is considered complete.

## 9. Record and tracking

This dated record is the design deliverable for queue task 21669 and source issue #5690. The implementation/cutover runbook remains `doc/fleet/agent-events-turso.md`; completed slices must be linked to concrete tests, merged changes, or timestamped operational evidence there or on #5690. This queue slice does not execute migration or change production state.
