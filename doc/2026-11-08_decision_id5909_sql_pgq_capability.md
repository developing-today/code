# Decision: SQL/PGQ capability boundary for ID triples

- **Date:** 2026-11-08
- **Status:** design decision; no implementation is included.
- **Issue:** [#5909](https://github.com/developing-today/code/issues/5909),
  SQL/PGQ capability investigation for ID triples.
- **Parent model:** [#5688](https://github.com/developing-today/code/issues/5688)
  fixes atomic triples as the canonical data model and SQL/PGQ as the graph
  query target. This record does not reopen those decisions.
- **Decomposition boundary:** [#5907](https://github.com/developing-today/code/issues/5907)
  owns compound-to-triple rules; [#5908](https://github.com/developing-today/code/issues/5908)
  owns the payload boundary. This record does not choose which object values
  become ID references versus attached payloads.
- **Query-surface composition:** [#5675](https://github.com/developing-today/code/issues/5675)
  is the local/remote query design. Its child [#5816](https://github.com/developing-today/code/issues/5816)
  is closed with a v1 structured query object: single-table only, conjunctive
  field predicates, no joins, no expressions or subqueries. Therefore PGQ
  graph-pattern traversal is **not part of #5675 v1**. This sequences the two
  designs; it does not silently expand or overturn either one.
- **Evidence baseline:** `origin/main` at `83b7a024b8`, checked 2026-11-08.
  At work start, #5909, #5688, and #5675 were OPEN; the #5675 decision record
  was on open PR #7361 (`dd0a70c0e4`). #5816 was CLOSED and its recorded v1
  shape was checked before this recommendation.

## Decision summary

1. Treat SQL:2023 SQL/PGQ as the **logical graph-query target**, not a native
   feature requirement for every ID storage engine. Neither SQLite/libSQL nor
   ID's current redb event store supplies a documented native `GRAPH_TABLE`
   execution path in the evidence checked here.
2. Do not build a general SQL string front-end or full SQL/PGQ compiler as part
   of this design. A future graph-query surface should use a typed,
   backend-independent pattern representation, with an explicit capability
   profile per engine. Relational engines can lower the supported subset to
   parameterized joins/CTEs; redb needs a Rust evaluator over indexed keys.
3. The initial graph-pattern subset, if authorized by a follow-on query-shape
   design, is finite and bounded: labeled/directed paths with a fixed maximum
   hop count, ID equality and conjunctive scalar restrictions, and explicit
   projection of IDs/provenance. Unbounded paths, shortest-path semantics,
   optional patterns, arbitrary expressions, and aggregates are unsupported
   in that profile. This is **not** an extension to #5675 v1; its no-joins
   decision remains in force unless separately amended.
4. An engine must reject an unsupported operation with a stable structured
   error. No silent narrowing, scan-all/client-side filtering, or result that
   looks complete after dropping unsupported pattern clauses. Remote
   execution and sensitivity filtering remain server-side under #5675.
5. Use three cyclic composite indexes/ordered key encodings as the starting
   triple lookup layout: `(subject, predicate, object)`,
   `(predicate, object, subject)`, and `(object, subject, predicate)`.
   Keep a stable triple-row identity and statement/provenance reference; do
   not make the triple tuple unique if that would collapse distinct claims or
   their provenance.
6. **Require a rows-versus-document bake-off before freezing physical storage
   and index costs.** It compares a row-per-triple representation with a
   document/provenance anchor plus derived triple indexes on the same
   workloads. A document-only design is not an admissible outcome: it would
   contradict #5688's atomic-triples query model.

## Capability matrix

`Native` means the engine itself exposes SQL/PGQ graph-definition and
`GRAPH_TABLE`/`MATCH` execution. `Translated` means the operation can be
lowered to a different execution mechanism without changing its declared
semantics. `Unsupported` means reject it; do not approximate. “Proposed” is a
design capability, not an implementation claim.

| Capability                                                       | SQLite                                                                                 | libSQL                                                                                                              | ID current redb event store                                                                |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| Native SQL/PGQ (`CREATE PROPERTY GRAPH`, `GRAPH_TABLE`)          | **Unsupported** in the documented SQLite SQL surface checked                           | **Unsupported / not documented**; libSQL is a SQLite fork, but its exact deployed build still needs a feature probe | **Unsupported**; no SQL parser or graph-query API                                          |
| Basic bounded pattern to the same graph result                   | **Translated** (proposed): parameterized joins over triple rows                        | **Translated** (proposed), subject to build-specific SQLite-compatibility probes                                    | **Translated** (proposed): Rust evaluator using ordered B-tree keys/multimaps              |
| Recursive CTE execution                                          | **Native** (`WITH RECURSIVE`), with SQLite's documented recursive-select restrictions  | **Native by SQLite compatibility**; verify the exact version used before relying on details                         | **Unsupported as SQL**; traversal would be application code                                |
| JSON extraction/table-valued JSON functions                      | **Native** (`json_extract`, `json_each`, `json_tree`, subject to SQLite version/build) | **Native by SQLite compatibility**; verify the exact version/build                                                  | **Unsupported as SQL**; decode typed values in Rust instead                                |
| Composite lookup indexes                                         | **Native** relational indexes                                                          | **Native** relational indexes                                                                                       | **Translated** to application-maintained composite keys / multimap tables; not SQL indexes |
| Unbounded paths / shortest paths in the proposed initial profile | **Unsupported**                                                                        | **Unsupported**                                                                                                     | **Unsupported**                                                                            |

The SQLite and libSQL “translated” entries are a feasible implementation shape,
not evidence that an adapter exists today. SQLite's official documentation
describes recursive CTEs for tree/graph traversal and JSON functions, but does
not document SQL/PGQ syntax. libSQL describes itself as a SQLite fork with the
same file format/API and backwards compatibility; no libSQL-specific PGQ
implementation was found. The current `pkgs/id` package has no production
SQLite/libSQL store: `rusqlite` is a dev-dependency used by a bridge PoC, and
the manifest disables `p2panda-store` default features specifically to avoid
pulling in SQLite. The production event store uses redb directly.

PostgreSQL 19's official documentation is a useful native SQL/PGQ reference
implementation: it describes property graphs as read-only views over regular
tables and documents `GRAPH_TABLE`. The documentation page identifies the
version as unsupported/development documentation, so it is a comparison
target, not a deployment recommendation or compatibility guarantee.

## Store evidence and present gap

- `pkgs/id/src/event_store/mod.rs` describes the event store as holding
  verified events and applied state “in one redb file”; its tables are
  `TableDefinition` / `MultimapTableDefinition` entries (`events`, `hashes`,
  `chains`, `forks`, `groups`, `proof_ids`, `proof_headers`, `waiting`,
  `waiting_by_dep`, `meta`, `blocks`, `actions`, and `applied`). Their keys
  encode event positions, hashes, or other lookup keys. There is no
  subject-predicate-object row table.
- `LocalEvents::open` is the active path into `EventStore`; the `Storage` seam
  abstracts file operations but still returns a `redb::Database`, rather than
  selecting between SQL and redb engines.
- `pkgs/id/tests/bridge_poc.rs` contains an in-memory SQLite bridge schema for
  `events`, `chains`, `forks`, and proof tables. It mirrors event-store state;
  it is test-only and is not a triple schema. No relational claim/edge schema,
  `CREATE INDEX` for triples, `WITH RECURSIVE`, SQL JSON-function call, or
  SQL/PGQ implementation was found under `pkgs/id`.
- Existing graph-like behavior is in application code. For example,
  `tag_claims::causal_past` follows event-parent links in Rust. The tag index
  uses three `BTreeMap`s and tuple-encoded keys, which is useful precedent for
  indexed lookups, not a relational triple store.
- `store_traits.rs` exposes fixed `LogStore` / `TopicStore` reads and a
  closure-based `read` over `EventStore`; it does not expose a general filter
  struct or graph query language. The tag search DSL is a separate,
  domain-specific interface.
- `pkgs/id/benches/world_bench.rs` is the only registered benchmark harness
  found; it measures world-journal workloads, not event/triple storage. A
  separate graph-store benchmark or new benchmark group is needed for the
  bake-off.

The current state is therefore **not** “SQLite with PGQ disabled”; it is a
redb-backed event/document model with fixed APIs, plus a test-only SQLite
mirror. The triple store and its query path remain future work.

## Translation boundary and fallback semantics

For a future graph-query operation, use a typed graph-pattern IR as the
semantic input. It should not accept raw SQL/PGQ text. The adapter does the
following:

1. Validate the pattern against the selected capability profile and the
   requester's query shape before touching data.
2. For SQLite/libSQL, lower supported finite paths to parameterized relational
   operations over the triple rows. Use recursive CTEs only where the declared
   semantics require recursion and the bounded cost policy permits it; their
   availability is not a promise of full PGQ equivalence.
3. For redb, execute the same typed pattern through indexed point/range reads
   and in-process matching. No SQL text is generated or interpreted there.
4. Apply local/remote permissions, classification, limits, and ordering at
   the executing server. Unsupported clauses fail closed with the same
   validation/error semantics on local and remote targets.

For #5675 v1 specifically, adapters remain limited to its single-table query
object. A graph pattern that would require joining multiple triples is a
validation error or a later-version feature; it is not implemented by hidden
joins, per-result client queries, or adding a second query dialect. A follow-on
must explicitly reconcile SQL/PGQ graph traversal with #5816's v1 boundary
before implementation. This leaves #5688's graph-query target intact while
making the current API limit explicit.

## Triple layout and indexes

Logical fact rows must preserve the distinction between the asserted triple,
the authored statement/document that introduced it, and the event/author
provenance. A candidate relational shape is:

```text
triples(triple_id, subject_id, predicate_id, object_id,
        statement_id, source_event_id, ordinal, ...)
```

`triple_id` identifies a stored assertion, not necessarily the mathematical
tuple. The same `(subject, predicate, object)` may be asserted more than once
with distinct provenance; deduplication is an explicit semantic decision, not
an index side effect. `statement_id` links back to the canonical authored
compound-statement/provenance record. The triple decomposition rules in
#5907 and the reference-versus-payload boundary in #5908 determine how object
values are represented; this record does not invent that mapping.

Start with these three composite lookup orders:

| Index/key order                         | Leading lookup                | Typical graph use                           |
| --------------------------------------- | ----------------------------- | ------------------------------------------- |
| `(subject_id, predicate_id, object_id)` | subject; subject + predicate  | outgoing edges / predicate-qualified facts  |
| `(predicate_id, object_id, subject_id)` | predicate; predicate + object | reverse lookup by edge type and destination |
| `(object_id, subject_id, predicate_id)` | object; object + subject      | incoming edges and object-anchored patterns |

These cyclic orders give each component a leading-key path while retaining the
other two values for covering lookups where the engine supports them. Add
non-cyclic orders only when the workload shows a material gap; each extra
index costs writes and storage. On redb, encode equivalent ordered composite
keys in dedicated tables/multimaps and update them transactionally with the
canonical fact row. The current event-store schema is not itself that layout.

## Rows-versus-document bake-off

The bake-off is required before implementation freezes the physical store,
but it does not reopen the logical atomic-triple decision. Compare at least:

- **Rows:** one indexed row/key per atomic triple, plus a statement/provenance
  anchor.
- **Document + projection:** retain the authored compound document as the
  canonical provenance anchor and maintain an indexed per-triple projection.
  A document-only implementation is excluded because it cannot satisfy the
  parent query model without scanning and decomposing documents at read time.

Use one correctness corpus and equivalent data for both shapes. Measure ingest
and update cost, disk footprint (including indexes), outgoing/incoming and
predicate-anchored bounded-pattern latency, pagination/order cost, and
provenance reconstruction. Include sparse and high-degree nodes, duplicate
assertions with distinct sources, and realistic payload sizes. The oracle is
the corpus's reference triple decomposition and expected query results; a
performance win cannot excuse a different result. The existing world benchmark
is not a substitute for this store-specific workload.

## Alternatives rejected

- **Require native PGQ in every backend:** rejected; it would exclude the
  current redb path and turn engine feature parity into a prerequisite when
  the parent asks for a logical query target over different stores.
- **Wait for SQLite/libSQL to add PGQ:** rejected as the only path; it blocks
  the current design on upstream schedules and does nothing for redb.
- **Add PostgreSQL or DuckDB as a mandatory runtime:** rejected; the evidence
  shows PostgreSQL 19 as a native reference and DuckPGQ as research/extension
  precedent, not a reason to replace the shipped embedded store.
- **Use raw SQL as the public v1 query language:** rejected by #5816, which
  chose a data-shaped query object for stable validation and local/remote
  parity. SQL/PGQ text may be used in examples or tests, not as a dialect
  escape hatch around that contract.
- **Store only a JSON/document blob and scan/decompose on each query:**
  rejected; it conflicts with #5688's atomic-triples query model and the
  server-side execution and cost constraints of #5675.

## Implementation invariants

1. Atomic triples remain the logical query facts; every returned fact can be
   traced to its statement and source event/author where applicable.
2. Index maintenance is atomic with fact writes, and all index permutations
   agree with the canonical facts after restart/rebuild.
3. Duplicate tuple assertions do not lose distinct provenance unless a
   separate decision explicitly defines their merge semantics.
4. Capability is explicit per engine/version. Unsupported graph syntax or
   semantics is rejected; no silent fallback changes meaning.
5. The same supported typed pattern has identical semantics and structured
   errors across local and remote execution; filtering and cost limits happen
   at the executing server under #5675.
6. #5675/#5816 v1 remains single-table/no-join until a follow-on design
   explicitly changes it. This record authorizes no implementation.
7. The bake-off's correctness oracle is the reference decomposition/query
   corpus; performance results are measured only after both candidates pass
   that oracle.

## Evidence and sources

Repository evidence is pinned to `developing-today/code@83b7a024b8`:

- [`event_store/mod.rs`](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/event_store/mod.rs#L1-L3),
  [`TableDefinition`s](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/event_store/mod.rs#L89-L117),
  and [`Storage` seam](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/event_store/mod.rs#L172-L188).
- [Active local event-store open](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/local_events.rs#L225-L234).
- [ID dependencies and SQLite dev dependency](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/Cargo.toml#L94-L111),
  [`rusqlite` test dependency](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/Cargo.toml#L168-L176),
  and [SQLite bridge PoC schema](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/tests/bridge_poc.rs#L345-L389).
- [Tag tuple indexes](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/tag_index.rs#L43-L75)
  and [causal graph traversal in Rust](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/tag_claims.rs#L282-L298).
- [Fixed store traits](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/src/store_traits.rs#L430-L472)
  and [world benchmark harness](https://github.com/developing-today/code/blob/83b7a024b8/pkgs/id/benches/world_bench.rs#L1-L32).
- [SQLite recursive CTE documentation](https://www.sqlite.org/lang_with.html)
  and [SQLite JSON functions](https://www.sqlite.org/json1.html).
- [libSQL description and compatibility claims](https://docs.turso.tech/libsql).
- [redb crate description](https://docs.rs/redb/latest/redb/).
- [PostgreSQL 19 property graphs](https://www.postgresql.org/docs/19/ddl-property-graphs.html),
  [`CREATE PROPERTY GRAPH`](https://www.postgresql.org/docs/19/sql-create-property-graph.html),
  and [graph queries](https://www.postgresql.org/docs/19/queries-graph.html).

No build or test was run: this change is a design record only. No implementation
was started.
