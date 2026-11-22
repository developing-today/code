# C7 report-record storage seam

`src/report-records.ts` is the canonical record interface for C7 callers. It
stores versioned report-generation and snapshot events in one append-only
PM-stack JSONL log. The default is
`~/.local/state/pm-aggregator/report-records.jsonl`; set
`PM_AGG_REPORT_RECORDS_FILE` to select another persistent location. This
follows the aggregator's existing append-only event-log pattern rather than
adding a database or writing into the disposable projector.

## Caller interface

- `makeReportRecord(input, generatedAt?)` validates timestamps, source
  attribution, stable block IDs and the Chicago period, then derives the
  stable record ID from `(period.id, templateVersion, sourceCursor)`.
- `appendReportRecord(file, record)` appends a generated event. A rerun with
  the same ID and content returns the original record without another write;
  changed content at the same ID throws `ReportRecordConflictError`.
- `readReportRecords(file)` reads retained cursor versions;
  `readCurrentReportRecords(file)` projects the latest cursor per
  `(period, templateVersion)`.
- `appendReportSnapshot(file, record, snapshotAt)` freezes the record once per
  period. A changed snapshot conflicts. `appendReportSnapshotCorrection(file,
  snapshotId, record, correctedAt)` creates a separate linked correction event
  without mutating the snapshot; correction records must match the snapshot
  period. `readReportSnapshots(file)` reads immutable snapshots, and
  `readReportSnapshotCorrections(file)` reads their correction history.
- `assertReportUpdateAllowed(period, operation, at)` enforces the daily
  08:00 CT soft-close and its quote/append/banner/event-log/feedback allowlist.
  `reportBlockConflict(stored, rendered)` treats whitespace-only differences
  as equal and reports substantive changes for callers to skip and surface.

Events retain `asOf`, source cursor, source IDs/URIs/observation timestamps,
related timestamps, block IDs and links. `reportPeriod` supplies the canonical
period identity and DST-aware boundaries for daily, Monday-based weekly,
monthly, quarterly, and annual periods. `dailyReportPeriod` remains as the
daily convenience function.

## Storage contract and current scope

The caller-facing append/read functions accept either a JSONL path or the
append-only document capability returned by `createTursoDocumentRemote`.
The file backend is local to the PM aggregator state path; writes are
serialized within one process and do not claim cross-process locking or remote
replication. It remains useful for development and rollback.

The durable adapter uses the existing `public.agg_store_documents` table from
#6815; it creates no table or separate report database. Each generated record,
snapshot, and correction is stored as one immutable document under
`c7-report:v1:<event-type>:<sha256(event-id)>`. A shared atomic content claim
under `c7-report-record:v1:<sha256(record-id)>` prevents conflicting payloads
for the same record ID across concurrent event types. `insertIfAbsent` provides
the atomic idempotency boundary: a duplicate key reads back and compares the
canonical content, returning the original event for an identical rerun and
raising a conflict for changed content. Prefix reads return the event set; projections sort by the canonical event
timestamp and stable event ID. The generic overwrite API is not used for C7
event keys.

At the composition root, callers choose the file or Turso store according to
`PM_AGG_STORE_BACKEND` and pass that single backend into the report-record API.
There is no dual write and no silent local fallback when the durable store is
selected.

## Period composition and generation

`src/report-composer.ts` composes an explicit target period from the latest
retained lower-grain records available at `asOf`, and excludes blocks after
that instant or outside the target period. Daily reports use source records;
weekly reports use daily records and separately label only the prior Friday as
reach-back context; monthly, quarterly, and annual reports prefer composed
records at the next smaller grain, falling back to source records for a child
period when no composed version exists. Stable source-time and record-ID
ordering makes the result independent of record arrival order. Blocks bubble
upward according to their `importance` grains (or the default path from their
source grain). Non-visible `bubblingCandidates` preserve explicitly selected
higher grains across an excluded intermediate grain. Classified activity
carries only explicit member links; source attribution remains in `sources`.
The template sections and composer identity are versioned as `c7-rollup-v1`.

Generate a record with `bun run report:generate --kind <daily|weekly|monthly|quarterly|annual>
--period <identity> --as-of <canonical-ISO-timestamp> [--store <jsonl-path>]`.
Period identities are explicit: `YYYY-MM-DD` (daily), Monday `YYYY-MM-DD`
(weekly), `YYYY-MM` (monthly), `YYYY-QN` (quarterly), and `YYYY` (annual).
The command validates its complete argument list, period identity, and
half-open `asOf` bounds before store I/O, selects retained records available at
that point in time, and appends the result idempotently. A changed `asOf` is
part of the derived source cursor so distinct point-in-time views cannot
conflict under one immutable record ID. It follows `PM_AGG_STORE_BACKEND`: file mode uses the local
report-record JSONL file (`PM_AGG_REPORT_RECORDS_FILE` or the default path),
and `--store` selects another JSONL path in file mode. Turso mode uses the
existing durable document adapter and never silently falls back to a file;
`--store` is rejected in that mode. The composition API also accepts the
canonical report-record storage capability directly.
