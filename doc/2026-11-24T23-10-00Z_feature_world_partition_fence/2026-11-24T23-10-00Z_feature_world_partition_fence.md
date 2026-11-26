# Durable world partition fence (#3898)

This slice persists a world's explicit jurisdiction beside its journal and
checks it under the existing exclusive journal lock before opening or repairing
journal bytes. Local/unassigned journals remain usable locally; they must never
be adopted into a hosted partition on first access. Assigned journals cannot be
opened through the existing unscoped production opener.

The scoped journal API receives trusted jurisdiction context from its caller,
not a request header. It does not establish that context, enable guest grants,
provision partitions, or implement partition-local discovery. The account-to-world
admission seam, ServeWorlds records initialization before journal open, cached
hub slots, participant authorization and shards remain outside this slice.

Metadata is immutable through this API. Creation and checking use the stable
journal lock, so competing first opens cannot replace the winning partition.
A missing record on an existing journal, malformed record, mismatched world ID,
or mismatched jurisdiction refuses without journal repair. Sidecar persistence
precedes journal creation; an interrupted creation can resume in the same
partition, never in another one. No jurisdiction is inferred from a directory.

Verified when: targeted `world_store` tests prove persisted matching opens,
wrong/unscoped refusal with byte preservation (including torn tails), refusal
to adopt legacy journals, and concurrent first-open exclusion. The oracle is
the immutable binding invariant, not a residency claim. No complete #3898
acceptance criterion is claimed by this storage-level slice alone.

References: [design](../2026-11-01_design_id3397.md),
[issue #3898](https://github.com/developing-today/code/issues/3898).

---

## 2026-11-24T23-18-00Z Review: filesystem aliases

Review found an unscoped symlink alias could evade the adjacent sidecar and its
lock. Journal opens now refuse final-component symlinks and, on Unix, hard-linked
journals (including local ones). Ordinary local journals are unchanged; alias
users must select the canonical, singly-linked journal path. Ancestor directories
remain operator-owned, not a sandbox boundary against a filesystem administrator
who can remove metadata or replace locks. No path is derived from a jurisdiction.
Sidecar creation uses fsynced, same-directory temporary storage and a no-clobber
persist followed by directory fsync on Unix. Bare relative paths use `.` as their
parent. Tests cover aliases without repairing the target's torn tail.

---

## 2026-11-26T08-20-00Z Slice: scoped world open and discovery peek

`world_store::open_world_in_partition` opens a whole durable world through
`FileJournal::open_in_partition`, so the fence covers the service opener and
not only the journal: a world bound elsewhere, an unassigned legacy world or a
malformed binding is refused before restoration and before the directory,
petname and outbox files are created. `world_bound_to_partition` is a lock-free,
read-only peek for existence checks and listings: a missing world, a legacy
world, a world bound to another partition, a malformed or unknown-version
record and an aliased journal all read as absent, which is what a
partition-scoped server needs to keep `exists` and `list` from becoming an
existence oracle. It grants nothing; the open re-checks under the lock.

Still open: a serve-side partition setting that switches `ServeWorlds` to these
calls, partition-keyed hub slots, account admission and shards.
