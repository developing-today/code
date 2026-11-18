# Supabase recovery to Turso: final data receipt

Date: 2026-11-18. Scope: recover the deleted `muse-dev2` snapshot without
replacing newer live state. This is a data receipt, not a claim that every
Supabase engine feature or remote application has been reproduced.

## Complete archive

- Dedicated Turso archive: `fleet-supabase-recovery-20261117`.
- All **130 tables and 623,584 records** retained, including the zero-row
  partition parent and extension-owned COPY tables.
- Original backup compressed SHA-256:
  `e1ce4339a316f6402de27f8e0373558216afe52bbd570b81e52be155ebfd2634`.
- Verified complete SQLite archive SHA-256:
  `7473d9b37dc4361b720afdca19e792ce9ed04368a667ffad925ff279b5f1f632`.
- Hardened source parser independently reproduced every table's columns,
  counts and ordered decoded COPY-value hashes. Cloud archive counts match
  all 130 tables. Archive values are TEXT/NULL, preserving original numeric
  and timestamp precision; this is not the operational schema.

## Operational inserts

No existing row was updated or replaced by these recovery importers.

| Phase | Destination | Inserted rows |
| --- | --- | ---: |
| Typed runtime tables and 18 missing ingest parents | fleet-platform | 22,385 |
| Three missing AA sources and 157 missing sheet rows | fleet-platform | 160 |
| Four source-configuration tables | fleet-platform | 5 |
| Four historical mirror tables | fleet-mirror | 3,595 |
| Missing historical mirror/quota records in 16 existing tables | fleet-platform | 6,886 |
| **Total** | | **33,031** |

Fresh-connection verification checked every imported value. TEXT, INTEGER
and NULL values matched exactly. Five analytical measurement REAL cells
in the initial runtime import and three quota REAL cells in the final
historical import differed by at most one IEEE-754 ULP each. The verifier
checks finite values and the per-value ULP bound independently; a
14-significant-digit hash alone is not treated as proof. The full-precision
source values remain in the archive.

The 18 original ingest-parent rows and all 42 preexisting AA source rows
were independently compared and unchanged. Broader existing-row
preservation is supported by audited INSERT-only SQL, unchanged target
schemas and absence of target mutation triggers, not a full before/after
hash of every old row. Existing same-key differences were retained.
All 18 operational append-only trigger definitions match the reviewed
candidate token-for-token; their behavior was tested on staging, not by
attempting destructive mutations in production.

## Coverage and deliberate non-replay

The [130-table coverage matrix](2026-11-18_turso-recovery-coverage.json)
accounts for every archive table:

- 27 operational tables restored and verified.
- 18 existing operational tables received missing-only inserts.
- 28 additional existing platform destinations compared.
- 8 queue/control destinations key-reconciled, with no stale-state replay.
- 44 empty tables retained in the archive.
- 5 provider-specific system tables retained in the archive.

All 47 initially existing platform source/destination pairs were compared.
One apparent missing CI event already existed under a different ID: its
unique dedupe key, content and normalized timestamp instants matched.
That existing event was kept, and the duplicate candidate excluded.

The eight queue/control tables have **538 source keys absent from the
current queue**, but their exact records remain in the archive. They were
not replayed into live task, lane, worker, ledger, checkpoint, event,
snapshot or watermark state. In particular, stale queued tasks, enabled
lanes and working-worker statuses must not silently reactivate. Queue
comparison proves key coverage, not complete source/current value parity.

D1 configuration was not modified. The five recovered configuration rows
were added to Turso as separately verified records.

## Transaction failures and final completion

An initial historical import hit the CI unique dedupe constraint and
explicitly rolled back. Fresh absence checks verified no candidate rows
remained. A subsequent attempt received SQLITE_BUSY during read-only
preflight, before any INSERT statement; rollback acknowledgment was
uncertain. Fresh read-only checks again verified all candidate keys absent
and the database readable. Neither attempt was automatically retried.

The fix checks alternate unique keys before writes, excludes the semantic
CI duplicate, and batches SELECT-only guard requests while retaining
transactional schema, key, count, hash and baton checks. The final reviewed
single production transaction committed **6,886 rows in 19.714 seconds**;
independent fresh readback verified every recovered value and counts at
least equal to the transaction's captured baseline plus inserts. Newer
quota rows were preserved. All 61 focused recovery tests passed after
reconciling main. Full repository CI was not run as part of this receipt.

Implementation PRs: #11911, #11943, #11949, #11953; mirror history recovery
has its separate receipt. Exact private execution logs, row-level checks
and candidate artifacts remain in the recovery workspace; no recovered
row contents or credentials are included here.

## Live checks and remaining limits

The coordinating operator's post-commit check observed `/api/health` and
`/api/alerts.json` returning HTTP 200, the aggregator active, and collector,
backup, daily, weekly, quota and ainventory timers active. This is a bounded
service check, not proof of every background run or notification delivery.

The backup contains **zero push subscriptions**, so it cannot restore phone
registrations. No forced phone notification was sent and phone delivery is
not claimed verified. Remote Muse runtime coverage is outside this data
receipt. Provider-specific cron/auth/storage internals remain archival
rather than being blindly replayed as live services.
