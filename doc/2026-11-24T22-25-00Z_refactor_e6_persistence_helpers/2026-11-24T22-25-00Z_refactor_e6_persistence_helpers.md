# E6 shared persistence helpers (Refs #12801)

The bounded slice ports the existing event/body/chain/fork persistence helpers
to a storage-only `RowWrite` interface in id-store. Native ingest continues
calling those same helpers, not a parallel ingest implementation. Tenant writes
must hold an explicit transaction; failed helper batches roll back together.
The interface initially needs get/put/multi_put only; waiting-row reads/removals
are a later extension, not speculative methods in this change.

Verified when: targeted id-events tests compare event bytes, indexes, occupied
positions, body/chain encodings, retained fork headers and rollback against the
native-redb oracle on both backends. Tenant schema must also accept COMPACTED,
which the inherited row adapter omitted. The inherited SQLite pipeline fixture
will be shared through opt-in test support, without shipping SQLite by default.

Acceptance census: #12801's EventStoreBackend implementation and full conformance
remain unchecked; this slice only shares four existing persistence helpers.
No caller above the store seam gains hosted-specific behavior. PR #14439 is the
unmerged row-layer predecessor, incorporated by merge rather than rewritten.
