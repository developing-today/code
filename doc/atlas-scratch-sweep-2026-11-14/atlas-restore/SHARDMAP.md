# Atlas core-tier shard restore — working notes (2026-11-04)

## Precondition (verified)

- Laptop `atlas-index-rewrite.service` = **inactive** (parallel coordinator's stop landed).
- Hosted shadow count stable: 28,660 (spike reads) → 28,760 (read #3, one final batch) → 28,760 (read #4, ~5 min later). Rewrite stopped; production atlas untouched by this pass (reads only).

## Engine ruling (Drew 18:28 CST) + API route (empirically verified)

- ALL new DBs = **TursoDB**. Standing creation route:
  `POST https://api.turso.tech/v3/organizations/{org-UUID}/databases`
  body `{"name": "...", "group_id": "16d3864a-3343-11ee-b78e-3698c249076c", "use_tursodb": true}`
  (org in path must be the org UUID `5980ebf0-b064-45ba-9d5f-f2f9ec077f6a`, NOT the slug — slug returns "organization not found". Field identified from turso-cli source `internal/turso/databases_v3.go`: `CreateDatabaseV3Body.UseTursoDB` json `use_tursodb`; the CLI `--tursodb` flag sets it.)
- Probe gate PASSED on `atlas-tursodb-probe` (created via the API route):
  `turso db show` Type=**Turso**; `sqlite_version()` = **3.50.4** (libSQL atlas = 3.47.0);
  `F32_BLOB(768)` accepted; `vector_distance_cos` works; top-10 over 200 real chunks
  **identical** (ids + distances to 5 dp) vs production scan restricted to the same 200.
- Scan path = plain SQL exact-cosine scan over `vec F32_BLOB(768)` blobs, merged fan-out.
  TursoDB vector metric set (Drew 18:35 CST): **cosine, dot product, L2 distance, Jaccard** —
  this corpus is cosine by design (source index metadata metric=cosine); the other metrics
  are available to future corpora on this engine.
- **Deviation: FTS5 is NOT available on TursoDB** — `CREATE VIRTUAL TABLE ... USING fts5`
  fails: "Virtual tables are not supported in MVCC mode". TursoDB core scan stores are
  vector-scan only; keyword/FTS parity stays with the libSQL production DB for now. Recorded on #5669.
- Insert economics (measured on probe): multi-row `INSERT` = 1.6 s/100 vec rows vs 14.5 s for a
  100-statement pipeline of single inserts → loader uses multi-row inserts (250 vec rows/stmt, 50 chunk rows/stmt), `INSERT OR IGNORE` for resumability.

## Source inventory (hosted `atlas`, source of record; all counts double-queried)

- chunks 107,639 = vec rows 107,639; every vec exactly 3,072 B (768×f32); 0 chunks missing a vec; 0 orphan vecs; manifest set `snowflake_m_v2_768_f32` (Snowflake/snowflake-arctic-embed-m-v2.0, 768).
- Sensitivity (chunk label == session label in 100% of rows): personal 100,060; local-only 6,941; secret-bearing 638.
- Tier map (§4.2, exactly 3 levels, default internal): secret-bearing → **secret** (638); personal + local-only → **internal** (107,001); public = none in this corpus.
- Sessions are atomic (never split): 11,981 chunk-bearing sessions; max session = 2,911 chunks.
- **Laptop correction:** the laptop local DB's chunk content_hash set is _identical_ to hosted
  (107,639 distinct; sorted-join SHA-256 `c7b50281…` both sides). The laptop adds **zero chunks**:
  the packet's "2,226 local-only + 90 secret-bearing laptop chunks" were hosted _session_ counts.
  Laptop DB also has no vec table / no sensitivity / no labels; its extra messages (≈50k) belong to
  unchunked sessions and are referenced by no chunk (all chunk message_ids resolve inside hosted messages). Restore is hosted-only.

## Shard map (tier → topic → ordinal; sessions atomic; cap 4,800 chunks/shard per §5.2)

Internal (28 shards, 107,001 chunks): agent-tooling ×4 (3,866 ea), cloudflare ×1 (1,250),
fleet-repos ×3 (3,458/3,457/3,457), home-lab ×2 (4,070/4,069), infra-nix ×2 (4,094 ea),
media ×1 (1,607), misc ×1 (841; finances+health+travel+unlabeled merged — each <500),
models-and-plans ×2 (4,551 ea), people-pets ×1 (4,502), personal-admin ×4 (4,360×3+4,359),
security ×1 (3,489), work-engineering ×6 (4,435×4+4,434×2).
Secret (1 shard, 638 chunks): atlas-core-secret-main-01.
Content stores (§4.2): atlas-core-internal-content, atlas-core-secret-content
(messages + sessions rows for that tier; hydration for --expand).
Naming: `atlas-core-<tier>-<topic>-<nn>`. Full machine-readable map: shardmap.json (+ assignments.pkl).
Every scan shard carries chunks + vec (identical DDL to source) + an `atlas_manifest` table.
NO `libsql_vector_idx` anywhere (scan-over-shards default; TursoDB rejects the syntax regardless).

## Off-limits (untouched)

Production `atlas` (reads only), the nine spike DBs, the two TursoDB pilots.
`atlas-tursodb-probe` (200 test chunks) is a test artifact of this pass — kept, flagged for Drew's cleanup call.

## Completion addendum (2026-11-04, final)

- All 29 scan shards loaded, counts exact (loader log: BAD: none). Content stores complete:
  internal sessions 18,235 / messages 70,987; secret sessions 90 / messages 564.
- Verification (verify.py): per-tier content-hash set SHA-256 MATCH (internal + secret);
  60/60 vec byte spot-checks; fan-out parity 7/7 (see #5669 comment for the secret LIMIT-artifact note);
  per-shard scan latency 1.1–1.9 s (sandbox→us-east-1 HTTPS).
- Incidents: content message insert pass 1 died at 59,775/70,987 (RemoteDisconnected escaping the
  spike client's retry filter — it doesn't catch http.client.HTTPException); resume v1 died in its
  rescan on IncompleteRead of a 16.7 MB page; resume v2 (id-index pass + 200-id text pages + broad
  retries) completed exactly. Scan shards never affected.
- Filings: code#5669 comment 6647793163 · hatch-personal#20 comment 6647794729 ·
  hatch-personal#21 comment 6647796180 · consumer wiring = code#5755.
