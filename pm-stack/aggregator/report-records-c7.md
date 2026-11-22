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
  period. A changed snapshot conflicts; corrections must be separately linked
  records. `readReportSnapshots(file)` reads the immutable snapshots.
- `assertReportUpdateAllowed(period, operation, at)` enforces the daily
  08:00 CT soft-close and its quote/append/banner/event-log/feedback allowlist.
  `reportBlockConflict(stored, rendered)` treats whitespace-only differences
  as equal and reports substantive changes for callers to skip and surface.

Events retain `asOf`, source cursor, source IDs/URIs/observation timestamps,
related timestamps, block IDs and links. `dailyReportPeriod` supplies the
period identity and DST-aware boundaries.

## Storage contract and current scope

The JSONL file is append-only and local to the PM aggregator's configured
state path. Writes are serialized within one process; the current adapter does
not claim cross-process locking or remote replication. `fleet kv set/put` and
the amendment's indexed KV families are not implemented in this repository,
so this seam does not pretend to expose them. A later adapter may mirror the
same event contract into the fleet's chosen permanent store without changing
the report-record interface. This first slice supports the existing daily
period type; other calendar grains and generation commands remain subsequent
C7 work.
