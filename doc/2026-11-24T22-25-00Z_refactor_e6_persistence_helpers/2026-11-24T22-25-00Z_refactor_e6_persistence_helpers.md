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
which the inherited row adapter omitted. A private SQLite pipeline fixture in
id-events tests exercises bound SQL without shipping SQLite by default.

Acceptance census: #12801's EventStoreBackend implementation and full conformance
remain unchecked; this slice only shares four existing persistence helpers.
No caller above the store seam gains hosted-specific behavior. PR #14439 is the
unmerged row-layer predecessor, incorporated by merge rather than rewritten.

Transaction review: only `&mut TenantWrite` implements `RowWrite`; consuming a
guard in a helper is a compile-time error. An attempted BEGIN with an uncertain
response retires the session; local nested-BEGIN rejection preserves the original
transaction. Failed COMMIT returns its original typed error and attempts rollback.
If rollback cannot be confirmed, Drop closes the session rather than reusing an
uncertain baton. An applied COMMIT with a lost response may have durable rows;
this adapter does not promise exactly-once commit or safe automatic retry.

Evidence: failing-first run
[45500166347](https://github.com/dezren39/fleet/actions/runs/45500166347)
exposed missing `compacted`. Initial shared-helper run
[45500954292](https://github.com/developing-today-forks/fleet/actions/runs/45500954292)
actually executed two tests successfully (shared persistence and partial rollback).
Generated rows and transaction-fault tests were added after that run and require
their own verification; submission alone is not passing evidence.
