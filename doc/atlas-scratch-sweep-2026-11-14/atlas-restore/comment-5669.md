## Core-tier shard restore — executed 2026-11-04 (per Drew's stop-and-redirect ruling, 18:24 CST)

The in-flight index rewrite was stopped (precondition verified here: laptop `atlas-index-rewrite.service` inactive; hosted shadow frozen at 28,760 across two reads ~5 min apart) and the corpus was restored into fresh shards. Production `atlas` was only ever read — untouched.

### Engine + creation route (Drew ruling 18:28 CST: all new DBs are TursoDB)

- Standing creation route, identified from turso-cli source (`CreateDatabaseV3Body.use_tursodb`) and verified empirically: `POST https://api.turso.tech/v3/organizations/{org-UUID}/databases` with `{"name": ..., "group_id": ..., "use_tursodb": true}`. The path org must be the org **UUID** — the slug returns "organization not found".
- Gate probe (`atlas-tursodb-probe`, created via the API route): `turso db show` Type=Turso, `sqlite_version()` = 3.50.4 (libSQL = 3.47.0), `F32_BLOB(768)` accepted, `vector_distance_cos` scan top-10 identical to production on the same 200 real chunks.
- Scan path everywhere: plain SQL exact-cosine scan over `vec F32_BLOB(768)`, fan-out + merge. No `libsql_vector_idx` (TursoDB rejects the syntax; scan-over-shards is the design regardless). TursoDB's metric set for future corpora: cosine, dot product, L2, Jaccard — this corpus is cosine by design (source index metadata).
- **Deviation recorded:** FTS5 is unavailable on TursoDB ("Virtual tables are not supported in MVCC mode"), so the new scan stores carry no `chunks_fts`. Keyword search stays on the libSQL monolith until the consumer work decides the FTS answer (see #5755).

### Shard map (tier → topic → shards; sessions atomic; ≤4,800 chunks/shard per §5.2)

Source: hosted `atlas`, 107,639 chunks = 107,639 vectors (all 3,072 B; 0 missing/orphan). Sensitivity → tier: secret-bearing → secret; personal + local-only → internal (default); public = none in this corpus.

| Shard(s)                                                       | Chunks             |
| -------------------------------------------------------------- | ------------------ |
| atlas-core-internal-work-engineering-01..06                    | 4,435 ×4, 4,434 ×2 |
| atlas-core-internal-personal-admin-01..04                      | 4,360 ×3, 4,359    |
| atlas-core-internal-agent-tooling-01..04                       | 3,866 ×4           |
| atlas-core-internal-models-and-plans-01..02                    | 4,551 ×2           |
| atlas-core-internal-infra-nix-01..02                           | 4,094 ×2           |
| atlas-core-internal-home-lab-01..02                            | 4,070, 4,069       |
| atlas-core-internal-fleet-repos-01..03                         | 3,458, 3,457 ×2    |
| atlas-core-internal-people-pets-01                             | 4,502              |
| atlas-core-internal-security-01                                | 3,489              |
| atlas-core-internal-media-01                                   | 1,607              |
| atlas-core-internal-cloudflare-01                              | 1,250              |
| atlas-core-internal-misc-01 (finances+health+travel+unlabeled) | 841                |
| **atlas-core-secret-main-01**                                  | **638**            |

Plus per-level content stores (§4.2): `atlas-core-internal-content` (sessions 18,235; messages 70,987) and `atlas-core-secret-content` (sessions 90; messages 564). (The content message load needed one idempotent resume: the first pass dropped at 84% on a transport disconnect — `RemoteDisconnected` escaping the HTTP client's retry filter — and a targeted second pass inserted only the missing ids to the exact counts above. Scan-shard loads were unaffected.) Every scan shard carries chunks + vec with DDL identical to source, plus an `atlas_manifest` self-description. Secret chunks exist **only** in the two secret DBs.

### Verification (all independent of the loader's own counts)

- Per-shard chunk/vec counts == map expectations, 29/29 exact. Tier sums: internal 107,001, secret 638, total 107,639.
- Per-tier content-hash reconciliation: sorted-set SHA-256 of all content_hashes, source vs shards — **internal MATCH, secret MATCH** (no chunk lost, duplicated, or mis-tiered).
- Vector bytes: 60/60 random spot-checks byte-identical to source.
- Parity (held-out real queries, query chunk excluded): merged fan-out top-10 == production exact-scan top-10 on 5/5 internal queries and 2/2 secret queries (an early "False" on secret was a LIMIT artifact in the test harness — fan-out fetched top-10 including the excluded query row; corrected run: identical ids, identical order, max distance delta 8e-7).
- Latency (sandbox → us-east-1 HTTPS, network-inclusive, per-shard top-10 scan): 1.1–1.9 s at 3.5–4.5k rows — above the §5.2 libSQL-based budget; flagged for the consumer work (#5755: re-measure from the laptop path; §5.5 replicas are the lever).

### Corrections to the restore packet's premises (verified, second-sourced)

- The laptop local DB contributes **zero** chunks: its chunk content-hash set is identical to hosted (same sorted-set SHA-256). The "2,226 local-only + 90 secret-bearing laptop chunks" were hosted _session_ counts; at chunk level everything already lived in the hosted source of record (6,941 local-only + 638 secret-bearing chunks, all restored above). The laptop DB holds no vectors/labels; its extra messages belong to unchunked sessions and are referenced by no chunk.

### Leftovers

- `atlas-tursodb-probe` (200 test chunks from the gate) kept as a test artifact — cleanup is Drew's call, along with the nine spike DBs (untouched).
- Consumer wiring (fan-out, tier ceilings, hydration, FTS answer): #5755. Cache/flags remain #5668; aggregator promotion into these core stores remains this issue's open scope.
