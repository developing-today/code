# Real encoding bake-off (A-GRAPH-9125-03, #10219)

This is the candidate-engine measurement that design part A §4 (#9125) asks for
before the V2 storage and encoding choice is frozen. It replaces nothing: the
stdlib toy in `graph-encoding-toy.py` measured serialization only, and none of
its numbers are used here. No universal winner is asserted.

## How to reproduce

```
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/validate_real_encoding_bakeoff.py --run-and-check
```

The runner invokes the exact harness,
`cargo test -p id-store --test graph_encoding_bakeoff -- --exact graph_encoding_bakeoff`
with `BAKEOFF_RESULTS` set, and then validates the fresh raw results, the
committed `real-encoding-bakeoff-results.json` and this report. The validator
is standard-library Python and independent of the Rust code: it regenerates the
corpus with its own splitmix64 generator and canonical-CBOR encoder, recomputes
the fixture hash, recomputes every expected query, page and write output from
its own reference model, checks that every engine/workload cell is present for
each of the 3 runs, checks engine versions, schemas, indexes and query
plans, checks that every deterministic field (counts, digests, storage bytes)
is identical across runs and between the fresh run and the committed file, and
checks that the tables below are exactly what the committed results say.
`--check-committed` skips the cargo run.

## Fixture

6129 triples over 1000 subjects, generated from seed
`0x91250003` (generator `splitmix64/v1`), fixture SHA-256
`73d1772be3c809f9aa38d6f9ba702f726eec5690636ac5f30c52e3b70cf869ec` (over the sorted canonical triple bytes, each framed by its
u32 big-endian length). Terms are the canonical typed-byte terms of slice
A-GRAPH-9125-01 (#10196): typed IRIs, UTF-8 text, 8-byte integers, opaque
byte blobs that always contain `00` and `FF` bytes, content references and
signed-envelope references. Each subject has a name, a score, a blob, zero to
five `follows` edges, and sometimes an avatar content reference or a `cites`
envelope reference.

## Engines

| engine             | what is stored                                                                                                                                  | engine and version                        |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| `chunked_binary`   | sorted canonical triples in u32-length-framed chunks of at most 4096 bytes, with a chunk directory and a trailer; immutable, no secondary index | in-test file format `idcb/1`              |
| `redb_tuple`       | SPO and POS tables keyed by `id_core::tuple` encodings of the canonical term bytes (the current tuple-key materialization)                      | redb 4.6.0 (pinned)                       |
| `sqlite_jsonb`     | one JSONB document per triple: structured terms plus hex term keys, with a unique (s, p, o) and a (p, o) expression index                       | SQLite 3.54.1 via rusqlite 0.42 (bundled) |
| `sqlite_blob_rows` | interned `terms(bytes)` table plus a `WITHOUT ROWID` (s, p, o) table and a (p, o, s) index                                                      | SQLite 3.54.1 via rusqlite 0.42 (bundled) |

The exact DDL, index list, durability settings and the `EXPLAIN QUERY PLAN`
output for both indexed queries are in the results file under `engines`. Both
SQLite plans search their indexes; neither scans the table.

## Workloads

- **load:** all triples in one transaction.
- **storage:** file bytes after the load, and after compaction (redb
  `compact`, SQLite `VACUUM`).
- **round_trip:** every stored triple is read back through the engine's own
  pager and must decode to exactly the original terms, type tags and
  reference kinds included.
- **query_subject:** 32 subject lookups.
- **query_predicate_object:** 16 `follows`/object lookups returning subjects.
- **page_stream:** the whole store in keyset pages of 128.
- **write:** 200 single-triple inserts, then a delete of every
  37th original triple, one transaction each; the final
  state must equal the reference model.

## Results

Timings are medians over the 3 runs, in microseconds, from a
`debug` build. Storage bytes are identical in every run.

<!-- bakeoff-table:begin -->

| engine             | load µs | subject µs | p/o µs | page µs | write µs |   bytes | compacted | bytes after writes |
| ------------------ | ------: | ---------: | -----: | ------: | -------: | ------: | --------: | -----------------: |
| `chunked_binary`   |    7982 |      17806 | 173268 |   12489 |  1085359 |  312475 |    312475 |             311511 |
| `redb_tuple`       |  199808 |      20790 |   8298 |   27712 |   633849 | 1118208 |    974848 |            1945600 |
| `sqlite_jsonb`     |  277592 |      25624 |   9329 |  116002 |   353003 | 2641920 |   2523136 |            2600960 |
| `sqlite_blob_rows` |  178440 |      23801 |  10503 |   30377 |   357208 |  475136 |    438272 |             475136 |

<!-- bakeoff-table:end -->

Lowest value per column in this run (a reading of the table, not a ranking):

<!-- bakeoff-leaders:begin -->

- load µs: `chunked_binary`
- subject µs: `chunked_binary`
- p/o µs: `redb_tuple`
- page µs: `chunked_binary`
- write µs: `sqlite_jsonb`
- bytes: `chunked_binary`
- compacted: `chunked_binary`
- bytes after writes: `chunked_binary`

<!-- bakeoff-leaders:end -->

## What the numbers say

- The deterministic chunked file is the smallest and the fastest to load,
  scan and page, because it stores each triple once with no index. It pays
  for that everywhere else: predicate/object lookups scan every chunk, and
  every single-triple write rewrites the whole file, so its write column is
  the largest.
- redb tuple keys answer both indexed queries by prefix range scans and need
  no query planner, but storing SPO and POS copies of every key costs about
  between three and four times the chunked size, and the file grows under single-row writes
  until it is compacted.
- SQLite JSONB is the largest store, roughly eight times the chunked file,
  and the slowest to page, because every row carries structured terms and
  hex keys and every read parses JSON back into terms. Its expression
  indexes do serve the indexed queries.
- Normalized SQLite BLOB rows stay at about one and a half times the chunked size because terms
  are interned once, keep both lookups indexed, and are among the cheapest
  to write. Deleted triples leave their interned terms behind.

Which of these trade-offs matters depends on the V2 workload mix (append
rate, query shapes, page sizes, compaction policy), which this slice does not
decide.

## Limits

- single machine, single process, one thread; timings are wall-clock microseconds for the given build profile and are only comparable within one run
- corpus is synthetic (about 6000 triples, 1000 subjects); production graphs may differ in size, skew and term mix
- durability differs by engine: chunked_binary does not fsync, redb and SQLite fsync on each commit with their default settings
- chunked_binary has no secondary index, so predicate/object queries scan the whole file and every write rewrites it
- sqlite_blob_rows never collects interned terms whose triples were deleted
- storage bytes depend on the pinned engine versions and page sizes; other versions or settings may differ

## Rollback

Remove `pkgs/id/crates/id-store/tests/graph_encoding_bakeoff.rs`, this report,
`real-encoding-bakeoff-results.json` and `validate_real_encoding_bakeoff.py`,
and the `rusqlite` and `sha2` dev-dependencies of id-store. No production
codec, schema or index is changed.
