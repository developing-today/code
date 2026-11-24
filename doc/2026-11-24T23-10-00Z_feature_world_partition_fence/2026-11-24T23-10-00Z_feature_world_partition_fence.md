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
