# Decision: Turso PITR is not Atlas corpus retention

- **Date:** 2026-11-05
- **Status:** Decision record; no implementation is included here.
- **Issue:** [#6043](https://github.com/developing-today/code/issues/6043)
- **Parent design:** [#6026](https://github.com/developing-today/code/issues/6026),
  content versioning and retention.
- **Evidence:** the decision captured in #6043 and Turso's [point-in-time
  recovery documentation](https://docs.turso.tech/features/point-in-time-recovery).

## Decision

Treat Turso Cloud point-in-time recovery (PITR) as **whole-database disaster
recovery**, not as Atlas's corpus-retention or file-version mechanism. Atlas
retention is a separate, logical policy over file versions and their lineage,
implemented through Atlas metadata and `rustfs`. The two mechanisms may coexist;
neither substitutes for the other.

The issue records current PITR windows as 24 hours on the free plan and 10/30/90
days on paid tiers. A restore creates a new database and consumes the plan's
database quota. These are provider-plan facts, not guarantees about corpus
history; verify the current Turso terms before operationalizing them.

## Rationale and rejected alternative

PITR restores a database at a point in time. Its granularity is the whole
database, its window is plan-bounded, and a restore produces another database.
It does not express which version of one file is current, preserve rename
lineage, or apply per-class retention and deletion policy. Using PITR as version
history would couple unrelated files and make ordinary historical retrieval
depend on disaster-recovery windows and database quota.

Atlas retention answers a different question: which versions of each corpus
object remain addressable, when older versions move to cold storage, and when
policy permits retirement. Treating PITR as Atlas retention is therefore
rejected; treating Atlas retention as a replacement for database recovery is
also rejected.

## Consequences and invariants

1. A database restore does not create, select, or retire an Atlas file version.
   A corpus version's identity and lineage remain Atlas-owned.
2. Cold rollout to `rustfs` preserves resolution until an explicit retention
   rule retires the version. PITR windows do not set that lifecycle.
3. Keep the #6026 default: retain unless both a named trigger and the applicable
   class rule authorize retirement. Retirement must not orphan chunks still
   referenced by retained versions.
4. PITR remains an operational recovery path for a damaged database; per-object
   history, cold storage, and deletion remain Atlas policy.

This record clarifies the layer boundary only. Version identity, lineage,
per-class retention defaults, triggers, and retrieval semantics remain governed
by #6026 and its design record.
