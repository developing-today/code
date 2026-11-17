# Atlas shadow-cost spike — findings (2026-11-04, cloud-side, laptop untouched)

## Mechanism (measured, fresh DBs + production reads)

- Production atlas shadow table `vec_snowflake_m_v2_768_f32_idx_shadow (index_key INTEGER, data BLOB)`:
  every `data` blob is EXACTLY 160,576 B (verified on production sample of 50 and on every fresh index).
- Index metadata (`libsql_vector_meta_shadow`, 81 B) decodes to params: dims=768, alpha=1.2 (f64),
  70, 200, and a stored constant 160576 — the per-row blob size is computed at CREATE INDEX time.
- Blob anatomy (fresh 100-row index): 16-byte header; own vector at offset 16; neighbor vectors at
  offsets 16 + k*3072. A node's blob contains FULL f32 copies of its DiskANN graph neighbors' vectors.
  Hits per blob at N=100: 7–19 (actual degree + 1) inside a fixed-capacity arena.
- Byte budget (exact, fits dims 384/768/1536): blob = 52 x raw_vector_bytes + 832
  (16 B header + 52 vector slots x dims*4 + 816 B tail). dims sweep: 128 -> 19,536 (small-dims rounding differs),
  384 -> 80,704, 768 -> 160,576, 1536 -> 320,320.
- Growth law: LINEAR in dims and in rows (constant per row). Not nonlinear, not content-driven.
  Full atlas index at completion = 107,639 x 160,576 B = ~17.3 GB. That is the index's honest cost
  in this implementation: each node materializes up to ~51 neighbors' full vectors for traversal/re-ranking.

## Backfill (the production anomaly)

- Fresh DB, 100 real rows, CREATE INDEX: shadow populated 100/100 in 2.7 s. Backfill WORKS fresh.
- Fresh DB, index-first + insert 100: shadow 100/100 (11.4 s inserts — index maintenance on the write path).
- Production CREATE INDEX + ~75-min REINDEX left 0 rows. So production's empty shadow is NOT the
  implementation's normal behavior at small N: it is scale- or state-dependent (wedged primary /
  build not surviving at 107k rows on the hosted instance). Shard test at 5,000 rows probes the middle.

## THE WEDGE — reproduced exactly at 5,000 rows (fresh DB atlas-shard-spike-a/b)

- CREATE INDEX on 5,000 populated rows ran >2 min without committing (build is a single transaction:
  shadow stays 0 until commit). Two of my own client timeouts (120 s pipeline cap; a 280 s shell wrapper)
  interrupted builds — after which the DB is left in production's exact state:
  shadow table + meta row EXIST, but sqlite_master has NO index entry ("DROP INDEX" -> "no such index"),
  shadow = 0 rows forever, every later CREATE INDEX fails instantly:
  "vector index: unable to update global metadata table".
- Manual cleanup (DELETE meta row + DROP shadow table) then blocked for minutes server-side and the
  pipeline died with HTTP EOF; CREATE INDEX still meta-blocked afterwards. In-place recovery of a
  wedged index is unreliable. Practical recovery = a fresh DB.
- Build-time scaling: 100 rows = 2.7 s committed; 5,000 rows > 121 s uncommitted. Production's 107,639-row
  build would need >= ~1 h at linear extrapolation (superlinear in practice) — far beyond any plausible
  client/statement lifetime on the hosted platform. This is almost certainly what killed production's
  original CREATE INDEX and REINDEX, and a dangling build transaction is the leading unified explanation
  for Defect 2's ~22-min writer stall as well.
- Write-path population cost (index-first, incremental): ~200 ms/row at N=1,000 (202.8 s / 192.7 s for
  two 1k shards), degrading with N — consistent with production rewrite's ~1 row/s at N=10-28k under load.

## Fan-out / parity (two disjoint write-path shards, 1,000 rows each; 5 held-out real queries)

- Index top-10 == scan top-10 on every shard, every query: 10/10 x10.
- Merged fan-out (union of both shards' index top-10, exact-cosine rescore) == exact global top-10
  over all 2,000: 10/10 x5. Recall of the populated index on this corpus slice is perfect.
- Latency over the wire (sandbox -> us-east-1 HTTPS) is network-dominated at this scale:
  index 577-767 ms, scan 611-748 ms per query — index buys ~nothing at N=1k; its payoff is server-side
  CPU at large N (scan grows with N; cf. revision's ~6.2 s scan at 107k). Fan-out parallel wall ~= one
  shard's latency; merge cost in Python is negligible. Scan@5k spot check: 664 ms warm (one 12.9 s cold outlier).

## TursoDB engine check on the pilot

- TursoDB REJECTS the index syntax: "invalid expression in CREATE INDEX: libsql_vector_idx(vec)".
  The ANN index design is libSQL-specific today; engine-switching is not a drop-in index fix.
  (Pilots remain fine for registry/embedding-cache tables, which don't need the ANN index.)

## Production rewrite observation (no contact made)

- Production shadow count read 28,660 at 23:11 UTC and again 28,660 ~50 min later. Reported as an
  observation only; the rewrite's own watcher owns its health.

## VERDICT (for #20)

- (i) for the SIZE, (ii) for the EMPTINESS, with a mechanism for both:
  - 160,576 B/row is the index's honest, universal fixed cost (52x raw + 832) — verdict (ii) on size:
    at 107,639 rows the full index is ~17.3 GB no matter how healthy the build. ANN via libsql_vector_idx
    is the wrong default at our scale unless a corpus genuinely needs it.
  - Production's empty shadow is verdict (i): a wedged/orphaned index state from an interrupted build —
    but in-place un-wedging is unreliable (meta cleanup blocked/EOF), so the remediation is the same as
    (i)'s: dump -> fresh shard DBs.
- RECOMMENDATION: stop treating the in-flight row-rewrite as the road to a searchable production index
  (recommendation only — the stop is Drew's word; the rewrite was NOT touched). Remediate by restoring
  atlas into fresh shard DBs (topic-first, ~5k chunks/shard per revision section 5 — builds stay in the
  minutes regime and parallelize), populated via the write path or patient single-shot builds, never
  interrupted mid-build. Default NEW corpora to scan-over-shards; reserve libsql_vector_idx for small,
  hot shards that earn 52x storage. The fresh-DB restore is also the only "restart" the platform offers.

## Engine (Drew's question — resolved, 3 independent signals)

- `turso db show atlas`: Type = SQLite, URL libsql://, has a primary instance (aws-us-east-1).
  New --tursodb DBs: Type = Turso, URL turso://, no instance rows.
- sqlite_version(): atlas 3.47.0 (libSQL fork vintage) vs pilot TursoDB 3.50.4.
- Platform API `type` field is logical/physical (schema-DB feature), not engine — not a signal.
- CONCLUSION: production atlas runs libSQL (sqld), definitively. TursoDB exists in this org only
  where a DB is created with --tursodb.

## TursoDB pilot (Drew approval a)

- CLI v1.0.34 supports `turso db create --tursodb` — NO dashboard click needed.
- Created, empty, labeled: pilot-tursodb-registry, pilot-tursodb-embedcache (Type=Turso, group great-star-lord).

## Restart (item 4)

- No instance restart/cycle operation exists: `turso db` subcommands = branch/config/create/destroy/
  export/import/inspect/list/locations/replicate/shell/show/tokens/unarchive; platform API instance
  ops = list/retrieve/delete/wait + add/remove instances. No restart endpoint in docs.
- Restart-equivalent = restore/branch into a fresh DB (which the shard remediation does anyway).

## Shard experiment (Drew's proposal)

PENDING shards.log numbers.

## DBs created in this spike (all kept, cleanup = Drew's call)

atlas-spike-bf100 (100 rows, indexed, healthy), atlas-spike-wp100 (100 rows, write-path, healthy),
atlas-spike-dims (dims sweep), atlas-shard-spike-a (5,000 rows + chunk text, index WEDGED - production replica),
atlas-shard-spike-b (5,000 vec rows, index WEDGED), atlas-spike-n1000 (1,000 rows, indexed, healthy),
atlas-spike-n2500 (1,000 rows, indexed, healthy),
pilot-tursodb-registry, pilot-tursodb-embedcache
