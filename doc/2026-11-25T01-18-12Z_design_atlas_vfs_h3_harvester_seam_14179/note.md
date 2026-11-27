# H3 — VFS harvester seam and tombstone consistency

Date: 2026-11-25
Status: design of record for [#14179](https://github.com/developing-today/code/issues/14179)
(workstream H, parent [#9100](https://github.com/developing-today/code/issues/9100),
parent design `doc/2026-11-24_design_atlas_vfs_workstream_h_9100.md` §6, §7
and §12). Design only. No implementation in this record's PR.

This record wires the H2 tables
(`doc/2026-11-25T01-16-52Z_design_atlas_vfs_h2_tables_manifest_14178/note.md`)
as a read projection over the #9115 harvester
(`pkgs/id/crates/id-store/src/location_index.rs`). It covers the immediate and
batch paths, the tombstone flow, the reconciliation cursor, how each state is
kept, and the reference signals for the #9084 collector. The H1 profile
(`doc/2026-11-25T01-14-10Z_design_atlas_vfs_h1_location_profile_14177/note.md`)
fixes what a row looks like. #9115 is the backend and is not redesigned here.

## 1. Single-writer invariant

**Each scope has exactly one writer of its `vfs_*` tables: that scope's
projector.** Producers write signed records to the bucket and nothing else.
The query layer only reads. No producer, API handler, migration or repair job
writes rows directly; repair is a projector rebuild (§6).

The invariant is enforced, not trusted. Every projector transaction is a
compare-and-swap on the scope's `vfs_scope_cursor` row:

```sql
UPDATE vfs_scope_cursor
   SET head_generation = :new_g, head_snapshot_id = :new_head,
       last_tombstone_basis = :new_b, last_tombstone_sequence = :new_s
 WHERE namespace = :ns AND tenant_scope = :scope
   AND head_generation IS :old_g AND last_tombstone_basis IS :old_b
   AND last_tombstone_sequence IS :old_s;
-- zero rows changed → another writer moved the cursor: roll back, reload, retry
```

Row writes, the cursor swap and the outbox rows (§5) commit in one
transaction or not at all. Two projectors racing on one scope therefore
serialize: the loser's transaction changes nothing, and it reloads from the
winner's cursor. A projector may be scheduled anywhere (one per scope or one
for many scopes); correctness does not depend on how many run.

The scope's first transaction inserts the cursor row (`INSERT … ON CONFLICT DO
NOTHING`, then the same CAS from the empty cursor).

## 2. Records and notifications

A producer publishes in this order:

1. Write the signed record object to the bucket at its H2 §4.2 key
   (snapshots also in the #9114 store if that is where they live).
2. For a snapshot, advance the scope's #9114 sidecar head by CAS (A-9114.2).
3. Send a notification `{namespace, tenant_scope, kind, key}` to the
   projector.

The notification carries no record content. The projector always re-reads
the object from the bucket and verifies it, so a forged or replayed
notification can at most cause a re-read. A lost notification is healed by
the reconciler (§4).

Producer rule from the #9115 fold order: a tombstone issued against basis
generation `g` must be published before the producer advances the head to
`g + 1`. If the projector rejects a tombstone as out of order (its basis is
behind the folded head), the producer re-issues it against the current head
with a new sequence. The projector reports the rejection; it never reorders.

## 3. The fold paths

Both paths use the same function, so they cannot drift.

```text
project(scope, records in fold order):
  load index  <- vfs_locations + vfs_tombstone_only + vfs_scope_cursor   (§3.1)
  for record in records:
      verify(record)                         # signature, payload, scope
      snapshot  -> index.apply_snapshot      # successor of head, or reject
      tombstone -> index.apply_tombstone     # strictly after last order, or reject
      changed   += entries whose state, provenance or presentation moved
  rows    <- H1 projection of changed entries, presentation per H2 §2
  signals <- §5 transitions of changed entries
  one transaction:
      upsert rows; upsert/delete vfs_tombstone_only rows
      append (order, kind, id, key) to vfs_unsealed
      insert signals into vfs_reference_outbox
      CAS vfs_scope_cursor (§1)
```

- **Immediate path:** one notification gives one record and one transaction.
  Interactive adds are visible when that transaction commits.
- **Batch path:** bulk ingest folds many records in order and commits in
  chunks of at most 1,000 records. Each chunk is its own transaction, so a
  crash loses at most one chunk, which is re-folded from the bucket.
- A rejected record stops the batch at that record. The records before it
  commit; the rest wait for the reconciler.

### 3.1 Projector state

The serving tables hold every field of `LocationEntry` (H2 §3 columns map one
to one: `sidecar_ref` = `asserted_by`, `asserted_generation`, `tombstone_ref` =
`tombstoned_by`, plus state, lineage, access class, signer and `observed_at`).
H2 does not project tombstone-only entries, so H3 adds one private table for
them:

```sql
CREATE TABLE vfs_tombstone_only (
  namespace     TEXT    NOT NULL,
  tenant_scope  TEXT    NOT NULL,
  content_id    TEXT    NOT NULL,
  locator       TEXT    NOT NULL,
  state         TEXT    NOT NULL CHECK (state IN ('stale', 'deleted')),
  observed_at   INTEGER NOT NULL,
  signer        TEXT    NOT NULL,
  tombstone_ref TEXT    NOT NULL,
  PRIMARY KEY (namespace, tenant_scope, content_id, locator)
) WITHOUT ROWID;

CREATE TABLE vfs_unsealed (
  namespace    TEXT    NOT NULL,
  tenant_scope TEXT    NOT NULL,
  order_key    TEXT    NOT NULL,  -- H2 §4.1 epoch key of this record
  kind         TEXT    NOT NULL CHECK (kind IN ('snapshot', 'tombstone')),
  record_id    TEXT    NOT NULL,
  record_key   TEXT    NOT NULL,
  PRIMARY KEY (namespace, tenant_scope, order_key)
) WITHOUT ROWID;
```

With the cursor row, these tables hold the whole `LocationIndex` for a scope,
so a projector restarts from the database without re-reading the bucket. The
implementation loads them through `LocationIndex::decode` of the equivalent
canonical value, or through a new from-entries constructor. Either way, the
test is that the loaded index equals `LocationIndex::rebuild` from the bucket
at the same cursor.

When a later snapshot asserts a tombstone-only location, the row moves from
`vfs_tombstone_only` into `vfs_locations` in the same transaction.

## 4. Reconciliation cursor

The cursor is the projector's position and the reconciler's measuring stick.
The reconciler runs per scope on a timer (default every 5 minutes) and after
any rejected record:

1. **Missed snapshots.** Read the scope's #9114 sidecar head. If its
   generation is above `head_generation`, walk `parent_snapshot_id` back from
   the head to the folded head and fold the missing snapshots in order.
2. **Missed tombstones.** List the scope's `tombstones/` prefix (tombstones
   are few). Fold any not yet folded whose order follows the cursor. Report
   any unfolded tombstone whose order is at or before the cursor: it can no
   longer fold, so its producer must re-issue it (§2).
3. **Seal.** If `vfs_unsealed` passes the H2 §4.4 thresholds, seal a
   generation (§4.1).
4. **Audit.** Rebuild the scope from the bucket at the latest sealed manifest
   (H2 §5, steps 1–6, without the swap). Its digest must equal that
   manifest's `projection`. A mismatch is a divergence (§6).

### 4.1 Sealing a generation

1. Read `vfs_unsealed` in order. Compute the projection digest of the current
   rows; the transaction that last moved the cursor is the epoch.
2. Build and sign the H2 manifest, and upload it to its epoch key. Ed25519 is
   deterministic and the inputs are fixed, so a retried upload writes the same
   bytes. An object already present with different bytes is a divergence.
3. In one transaction, delete those `vfs_unsealed` rows, set
   `vfs_scope_cursor.manifest_id`, and CAS the cursor at the same value
   (proving no fold slipped in). If the CAS fails, a fold committed in
   between: the uploaded manifest still describes a valid earlier epoch, and
   the next seal chains from it.

The manifest is uploaded before the database marks it sealed, so a crash
between the two only repeats the upload.

## 5. States and reference signals

### 5.1 What each state keeps

| State        | Set by                                     | Row keeps                                                                       | Served by default |
| ------------ | ------------------------------------------ | ------------------------------------------------------------------------------- | ----------------- |
| `fresh`      | snapshot, access class `public` or `scope` | `sidecar_ref`; `tombstone_ref` NULL                                             | yes               |
| `unverified` | snapshot, access class `stored_unverified` | `sidecar_ref`; `tombstone_ref` NULL                                             | yes, with grant   |
| `stale`      | stale tombstone                            | `sidecar_ref` of the last assertion and `tombstone_ref`; presentation unchanged | yes               |
| `deleted`    | delete tombstone                           | as stale; retained indefinitely (parent §11)                                    | no                |

Hard purge of deleted rows is a retention action owned with #9084 and #3125,
never a projector side effect.

### 5.2 Traceability (the acceptance invariant)

- Every `fresh` or `unverified` row has a `sidecar_ref` naming a snapshot in
  the bucket that verifies, is folded at or before the cursor, and asserts
  exactly this identity in its `locations`; its `tombstone_ref` is NULL.
- Every `stale` or `deleted` row has a `tombstone_ref` naming a tombstone in
  the bucket that verifies, has that kind, targets this identity, and is the
  last record to touch it.
- Nothing else changes a row's state. `apply_snapshot` and `apply_tombstone`
  are the only transitions, and they run only inside `project`.

The audit (§4 step 4) checks the first two for every row of a sealed epoch.

### 5.3 Reference signals for #9084

Parent §7 makes each location one reference on (`content_id`, scope). The
projector emits signals on state **transitions**, never on re-folds that
leave the state alone:

| Transition (per identity)                              | Signal    |
| ------------------------------------------------------ | --------- |
| no row, or `deleted` → `fresh` or `unverified`         | `assert`  |
| `fresh`, `unverified` or `stale` → `deleted`           | `release` |
| anything else (fresh ↔ unverified, → stale, re-assert) | none      |

Stale and unverified rows keep their reference: a location that may still
hold the bytes keeps them alive until a delete tombstone says otherwise. That
errs toward keeping bytes. Tombstone-only entries never asserted emit
nothing.

Signal shape (canonical JSON):

```json
{
  "cause": "<snapshot or tombstone id>",
  "content_id": "content/raw-blake3-256.…",
  "epoch": "00000000000000000002.1.00000000000000000001",
  "format": "atlas.vfs-reference/v1",
  "kind": "release",
  "locator": "drive/atlas/1A2bFixture/https://drive.google.com/file/d/1A2bFixture/view",
  "namespace": "atlas",
  "tenant_scope": "tenant:atlas-home"
}
```

- Signals go into `vfs_reference_outbox` in the fold transaction
  (transactional outbox), keyed by (`namespace`, `tenant_scope`, `epoch`,
  `locator`, `content_id`). A relay delivers them to #9084's collector at
  least once and deletes each after the collector acknowledges it.
- The collector dedupes on that key and orders by `epoch` within a scope. An
  `assert` after a `release` for the same identity is a later epoch, so
  counting or marking stays correct under redelivery.
- The VFS emits; #9084 decides. Under its fence, an `assert` that reaches
  content in `Tombstoned(g)` is the collector's to reject or resurrect. The
  row still shows the location claim. Whether the bytes still exist is the
  existence-receipt question (#10256), which VFS reads do not answer.

## 6. Failures and divergence

| Failure                                        | Effect                                           | Recovery                                                                  |
| ---------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------- |
| Notification lost                              | record unfolded                                  | reconciler §4 steps 1–2                                                   |
| Record object missing or bytes mismatch its id | record rejected; batch stops there               | producer republishes the object; reconciler retries                       |
| Snapshot not the head's successor              | rejected                                         | reconciler walks the chain from the head                                  |
| Tombstone behind the cursor                    | rejected and reported                            | producer re-issues against the current head (§2)                          |
| Two projectors on one scope                    | loser's CAS fails                                | loser reloads and retries                                                 |
| Crash mid-batch                                | committed chunks stand; current chunk rolls back | next run folds from the cursor                                            |
| Crash between manifest upload and seal commit  | manifest present, rows still unsealed            | next seal re-uploads identical bytes                                      |
| Audit digest mismatch                          | tables diverge from the bucket                   | stop serving the scope's writes, rebuild-and-swap from the bucket (H2 §5) |
| Outbox relay down                              | signals queue up; serving unaffected             | relay drains in order when back                                           |

A divergence never edits rows by hand. The bucket is the authority, and a
rebuild replaces the scope's tables in one transaction.

## 7. Paper check against the parent §9 example

Using H2 §6's records and the H1 vectors:

- S0 folds: six rows, six `assert` signals (Drive `unverified` still asserts).
- S1 folds: the r2 row's presentation changes, `sidecar_ref` moves to S1 on
  all six, and no signals are emitted (no transitions).
- S2 folds: one new row (`laptop-incoming-moved`), one `assert`.
- T0 folds: `laptop-incoming` → `deleted`, one `release` with cause T0.
- T1 folds: `drive-shared` → `deleted`, one `release` with cause T1.
- T2 folds: `tar-member` → `stale`, no signal.

Totals: seven asserts and two releases. Bytes `9f2c…` keep four live
references (`laptop-photos`, `r2-upload-renamed`, `tar-member` (stale) and
`laptop-incoming-moved`), so no collector model may reclaim them. The rows
equal H1's `delete-and-stale` step. Replaying the notifications for S1 and T0
a second time changes nothing: S1's generation is at or below the head and T0's
order is at or before the cursor, so the projector skips both as already
folded. A record already folded is recognized by its id and is never an
error; only an unfolded record behind the cursor is reported (§4 step 2).

## 8. Verification for the implementation slice

- **Golden:** the §7 walk, checking rows, the outbox contents and the
  manifests against H1 and H2.
- **Equivalence:** for random streams, immediate folding, batch folding in
  random chunk sizes, and rebuild from the bucket all give the same rows,
  cursor and signal multiset.
- **Single writer:** two projectors on one scope with interleaved
  transactions; the final state equals one projector's, and no signal is
  emitted twice for one transition.
- **Crash points:** kill the projector before and after each step of §3 and
  §4.1; the next run converges to the same state and emits no lost or extra
  signal.
- **Traceability:** the §5.2 audit over the golden and random streams, plus
  tampered cases (a row whose `sidecar_ref` lacks the assertion, a deleted row
  whose tombstone is stale) that the audit must flag.

## 9. Open questions

| Question                                           | Default here                                                               | Decider                           |
| -------------------------------------------------- | -------------------------------------------------------------------------- | --------------------------------- |
| Does `stale` hold a #9084 reference                | yes (keep bytes until a delete)                                            | #9084 owner, when its model lands |
| Notification transport                             | whatever Atlas ingest already uses; content always re-read from the bucket | Atlas program owner (#8983)       |
| Reconciler and batch chunk defaults (5 min, 1,000) | as stated; tune from first measurements                                    | owning namespace operator         |

## 10. Evidence checked (2026-11-25)

- #14179 issue body and comments; parent design §§6–7, 9, 11 and 12; H1 (PR
  #14706) and H2 (PR #14708) notes.
- `location_index.rs` on `main` at 8f92c98: module doc (fold order, rebuild
  equivalence), `apply_snapshot` (re-assertion revives, tombstoned-by
  cleared), `apply_tombstone` (delete wins, stale never resurrects,
  tombstone-only entries), `rebuild`, `ScopeCursor`, `LocationEntry`.
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/versioned-design.md`
  on #9084's reference and tombstone generation authority (`Live(g)` and
  `Tombstoned(g)` fence).
- A-9114.2 sidecar head CAS and A-9115.2 in
  `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`.
