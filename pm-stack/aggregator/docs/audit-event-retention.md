# Audit event retention, archive, and restore (#3322)

The cross-stack audit stream (`src/events.ts`) is append-only and
hash-chained. This document is the retention contract for the
records that leave the hot stream, and the restore procedure the
backup generations support.

## Hot / archive limits

- **Hot stream** (`events.json` in the aggregator state dir,
  `PM_AGG_EVENTS_FILE`): at most **30 days** and at most **5000
  records**, whichever bound hits first (`EVENT_RETENTION_DAYS`,
  `EVENT_RETENTION_MAX`). Bounds are enforced on append and on load
  through one gate, `partitionRetention`.
- **On-disk layout** (#14958): since the segmented layout landed,
  `events.json` is a small **v3 manifest** (`{version: 3, segments,
  checkpoint}`) and the records live one JSON object per line in
  append-only segment files under `events.json.segments/`
  (`seg-NNNNNN.jsonl`, rolling every 1000 records). A persist
  appends only the new records' lines plus the manifest — the
  whole-file rewrite per append the v2 layout needed is gone.
  Legacy v1/v2 files still load through the pre-existing path and
  are converted to the v3 layout on the first persist after load;
  the manifest's per-segment entries and top-level `checkpoint`
  (record count + last hash) are chain checkpoints, cross-checked
  at load so a cleanly truncated segment tail — invisible to
  full-chain verification — surfaces as an integrity violation
  instead of a silently shorter history.
- **Archive**: records pushed past either bound are **archived
  before they are pruned** — never silently dropped. They are
  appended, deduplicated by stable id, to monthly partitions under
  `events.json.archive/` beside the hot file:
  `events-archive-YYYY-MM.json`, keyed by the record's `at` month.
  A partition is `{ version: 1, archivedAt, records }` with the
  records in stored-chain order and carrying their original hashes.

## Integrity

- Every archived record's hash recomputes from its fields and its
  recorded `prevHash` (`verifyArchivedRecords`); partitions are
  verified again before every merge-write, and a merged partition
  that fails verification is never written.
- If an archive write fails, **pruning is deferred**: the records
  stay in the hot stream and `EventsStore.health()` reports the
  archive failure. A smaller hot stream is recoverable; a lost audit
  record is not.
- A chain that failed verification on load is never laundered into
  the archive: load keeps the violation visible and does not archive
  from a tampered stream.
- Records that were never chained (`hash === ""`, e.g. a v1
  migration row pruned on its first load) cannot carry integrity
  evidence into the archive; they are dropped with a loud log.
- `health()` reports `archivedRecords`, the archive verdict, and
  `historyGap`: when the hot chain's retention anchor names a pruned
  predecessor that no archive partition holds (history pruned before
  this contract landed, or a lost partition), the gap is a named,
  visible field — not a silent hole. Note the persist path
  re-chains the retained stream from genesis when it rewrites the
  layout (a legacy conversion, or retention removing a record from
  the middle of stored order), so after such a persist the hot
  chain is self-contained and the anchor question applies to the
  load-time state; ordinary appends no longer re-chain (#14958) —
  they extend the stored chain, whose anchor the archive check
  above resolves.

## Backup set

The daily backup (`nixos/home/pm-dashboard-backup.sh`) carries the
full audit state in every generation, each member represented in the
manifest and verified against it:

- `aggregator/events.json` (required) — the hot stream (the v3
  manifest since #14958; a v2 events file in older generations);
- `aggregator/events.json.segments/` (optional tree, #14958) —
  the segment files holding the hot stream's records; present in
  every generation taken after the segmented layout landed, and
  required to restore one (the manifest without its segments is a
  checkpoint attesting records the generation does not hold);
- `aggregator/events.json.archive/` (optional tree) — the archive
  partitions;
- `aggregator/events.json.outbox.jsonl` (optional file) — the
  failed-append retry outbox, when present;
- `aggregator/activity-ingestion.json` (optional file) — ingestion
  checkpoints, seen IDs, and gap state.

This is deliberately separate from the #1514 database backups: no
database dumps are duplicated here.

## Restore procedure

1. Verify the generation against its manifest (sha256 + size per
   file) before restoring anything.
2. Restore `events.json`, `events.json.segments/` (when the
   generation carries it), `events.json.archive/`,
   `events.json.outbox.jsonl`, and `activity-ingestion.json` into an
   isolated state dir (never over a live state dir as a first step).
3. Start a store over the restored dir (or read `health()`):
   - the hot chain verifies (`integrity.ok`);
   - the retry outbox flushes on load — `pendingRetries` returns to
     0 and the pending records join the stream (merged by stable id,
     so a record that also landed cannot duplicate);
   - `listArchived()` returns the pruned history and
     `verifyArchivedRecords` passes; `archivedRecords` matches the
     generation's partitions;
   - `historyGap.present` is false when the generation was taken
     under this contract. (A generation taken before the #14958
     segmented layout holds a v2 `events.json` and no segments
     tree; it restores through the legacy load path and converts
     on first persist, as above.)
4. Resume ingestion from the restored `activity-ingestion.json`:
   checkpoints and seen IDs come back with it, and replaying
   already-ingested deliveries is counted as duplicates and emits
   nothing new.

## Evidence

`test/events-archive-3322.test.ts` demonstrates, against the real
stores: archival before both pruning bounds (the shared
`partitionRetention` gate) and on append; byte-identical archived
records that verify after a restart; a tampered partition failing
the archive health verdict; the pre-contract gap reported as
visible; and a full manifest-verified backup/restore round trip —
retained + archived chain integrity, pending-retry recovery, and
ingestion checkpoint/seen-ID recovery with duplicate-free replay.
`test/events-segments-14958.test.ts` demonstrates the segmented
layout: append-only segment bytes with per-append growth bounded
by the new records, legacy v1/v2 conversion on first persist with
the chain byte-identical, checkpoint detection of a truncated
tail, torn-tail drop and repair, whole-set quarantine on
mid-segment damage, and manifest-loss adoption.
