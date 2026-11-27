# Backing up `fleet kv`

`fleet kv` keeps every host's local rows in one SQLite file,
`<state dir>/kv/local.db`, in WAL mode. Values over 1 MiB live beside it as
content-addressed working files in `local.db.working/`, and the rows refer
to them. This file and its snapshots are the host's own copy; a session's
unswept writes are also archived centrally, see
[Running the `fleet kv` central plane](kv-central.md). This page
covers taking snapshots, restoring one on a fresh host, and what a restore
can lose. Design: #13314, acceptance 6.

## Take a snapshot

```sh
fleet kv snapshot /backups/kv-$(date -u +%Y%m%dT%H%M%SZ).db
```

- The snapshot is one database file, `DEST`, plus `DEST.files/` when any
  version refers to a working file. It holds every committed row, including
  rows that are still only in the WAL, so never copy `local.db` and its
  `-wal` file by hand.
- Before the copy is taken, the store records a manifest row inside it.
  The command prints that manifest:
  `manifest=N taken_at_ms=… schema_version=… overlay_rows=… pinned_files=…`.
- The manifest pins every working file that any version refers to, history
  included, by content id, length and hash (`blake3-256:…`). Each pinned
  file is checked against its pin as it is copied into `DEST.files/`.
- The snapshot appears atomically: the files are copied under
  `DEST.files.partial/` and the database under `DEST.partial`, each synced
  and then renamed, the database last. If `DEST` exists, the snapshot is
  whole. `fleet kv snapshot` never overwrites `DEST` or `DEST.files`
  (`backup_target_exists`).
- Like every `fleet kv` verb, it runs only in a session with a trusted
  launch identity. Without one it exits 69 with `identity_unavailable`.

## Cadence and recovery point

- Take a snapshot every **6 hours**, and before upgrading or moving a host.
- A restore brings the host back to the snapshot. Writes made after it are
  lost unless the old host's `local.db` and `local.db-wal` survive. In that
  case, open them in place instead of restoring: SQLite replays the WAL on
  open, so no committed write is lost.
- When only snapshots survive, the recovery point is the age of the newest
  snapshot, at most 6 hours. Lost local writes were never swept to a
  central store, because Phase 1 has none.

## Restore on a fresh host

```sh
fleet kv restore /backups/kv-20261125T180000Z.db
```

`restore` checks the snapshot before it writes anything:

| Refusal                  | Meaning                                                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------ |
| `not_a_snapshot`         | The file has no fleet-kv backup manifest.                                                  |
| `snapshot_incomplete`    | A pinned working file is not in `SNAPSHOT.files/`.                                         |
| `unknown_schema_version` | A newer `fleet` wrote it. Upgrade first.                                                   |
| `snapshot_corrupt`       | The file failed `PRAGMA integrity_check`, or a pinned working file does not match its pin. |
| `backup_target_exists`   | This host already has a store, or a stray `-wal` file.                                     |

It copies the pinned working files into the new store's `local.db.working/`,
checking each one, before the database appears. It never overwrites an
existing store. To replace one, move `local.db`,
`local.db-wal` and `local.db-shm` aside first, and keep them until the
restored store reads correctly. On success it prints the manifest that
the snapshot carried. The restored store returns the same reads, history
and pending outbox intents as the source did when the snapshot was taken.

## Crash safety

A `kill -9` in the middle of `fleet kv set` leaves either the old row or
the whole new row, never a half row. Each committed version has exactly
one pending outbox intent. `pkgs/fleet/crates/fleet-kv/tests/crash_recovery.rs`
checks this by killing a real writer process at six points.
