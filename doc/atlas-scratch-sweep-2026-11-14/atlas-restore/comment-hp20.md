## Shard restore executed (follow-through on the spike verdict + Drew's 2026-11-04 18:24 CST ruling)

The rewrite was stopped and the restore is complete. Full record: developing-today/code#5669 (restore comment). Summary:

- **Precondition verified before starting:** rewrite unit inactive on the laptop; hosted shadow count frozen at 28,760 across two reads ~5 min apart (one final +100 batch landed between the spike's 28,660 and the stop). Production `atlas` was read-only throughout — untouched.
- **Result:** 29 TursoDB scan shards + 2 TursoDB content stores in group great-star-lord — internal tier 28 shards / 107,001 chunks by topic (work-engineering ×6, personal-admin ×4, agent-tooling ×4, …), secret tier exactly 1 shard / 638 chunks (secret material exists only in the secret DBs), public tier empty (no public corpora in this corpus). All new DBs are **TursoDB** per Drew's engine ruling (API `use_tursodb` route; gate probe verified Type=Turso + sqlite 3.50.4 + cosine scan parity before the full set was created).
- **Verification:** per-shard counts exact 29/29; per-tier content-hash set digests match source exactly (internal + secret); 60/60 vector byte spot-checks identical; fan-out top-10 == production exact-scan top-10 on 7/7 held-out real queries.
- **Notes:** scan latency measured 1.1–1.9 s/shard over sandbox HTTPS (flagged on the consumer issue); FTS5 does not exist on TursoDB (MVCC) — keyword search stays on the monolith pending the consumer decision. Consumer fan-out wiring filed as developing-today/code#5755.
- One premise correction, double-verified: the laptop's local DB holds no chunks beyond the hosted set (hash sets identical), so the restore is entirely from the hosted source of record.
