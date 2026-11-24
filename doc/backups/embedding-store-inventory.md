# Embedding / vector store inventory (backup mesh)

- Record for: #13824 (inventory leg) · Parent: #13461 · Program: #8983 ·
  Design: `doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md` §§2, 6.
- Verified against `origin/main` `fc438864a` (2026-11-23) by code + doc
  evidence, not by a live `turso db list` run: the sandbox holds no Turso
  credentials, so live existence checks are owned by #14032 (see
  "Runtime checks still open"). Every store below is named with the file
  that proves it exists.
- Kept honest by `pkgs/cold-store-reaper/tests/test_embedding_store_inventory.py`:
  every `vec_<model>_<dims>_f32` table the code uses and every store class
  below must stay named here, every row must carry a coverage disposition,
  and every uncovered row must name its follow-on issue.

## Inventory

| Store | Vectors | Home | Mesh coverage path | Status |
| --- | --- | --- | --- | --- |
| Canonical Turso DB `atlas` — `vec_snowflake_m_v2_768_f32` | Production vectors, 107,639 chunks at the 2026-10-31 census; Snowflake `snowflake-arctic-embed-m-v2.0` rev `95c27414`, 768d | Turso DB `atlas` (`atlas-dezren39.aws-us-east-1.turso.io`) | Class A rotation leg #13455 (weekly staggered restic → R2 `restic-turso-mesh`; slice 1 PR #13476 in review). Today: cold-store-nightly full export → Drive + PITR 30d + Delete Protection; the local source `~/atlas/turso-atlas/data/atlas.db` is covered daily by atlas-backup (local + R2 + Drive) | Covered by existing legs; mesh leg pending #13455 |
| Canonical DB — `vec_bge_small_384_f32` | Dormant legacy fallback, `BAAI/bge-small-en-v1.5`, 384d; in the publish and in local side-store files; no Rust code names it | Same DB / file as the canonical row above | Rides the canonical row's legs (rotation + existing leg) | Covered (dormant) |
| Scan shards `atlas-core-<tier>-<topic>-<nn>` — `vec_snowflake_m_v2_768_f32` per shard | Per-shard vectors; 29 topic shards, 107,639 chunks at the 2026-10-31 census (28 internal / 1 secret / 0 public) | Turso DB per shard (if `ATLAS_SHARDS=turso[:org]`) or local `.db` file (if `ATLAS_SHARDS=dir:<path>`) | Turso-homed: class B rotation #13456 (14-day, enumerated by `turso db list`) + today cold-store-nightly + PITR. Local-file: no leg — uncovered, follow-on #14032 | Partially covered — local-file home uncovered, follow-on #14032 |
| Content stores `atlas-core-<tier>-content` | No vectors (messages/sessions for hydrating shard-served hits only) | Turso DB per tier | Out of scope — no vectors; the DBs themselves ride the class B rotation as Turso DBs | Out of scope (no vectors) |
| Local side-store vector files (`ATLAS_VEC_DB`) | Vectors joined to the local publish by `content_hash` | Local files (path via env; not set in any shipped config) | No leg today — uncovered, follow-on #14032 (restic file leg, or recorded exclusion if unset live) | Uncovered — follow-on #14032 |
| Query-embedding cache DB (`ATLAS_EMBED_CACHE_TURSO_URL`, table `query_embedding_cache`) | Cached query vectors (pure function of query text + model; no TTL needed) | Turso DB, only when the env var is set; not set in any shipped config — existence unverified | If present: class B rotation #13456 via enumeration — covered by design. Verify with `turso db list` (#14032) | Covered-by-design if present; existence unverified |
| In-memory query-embedding cache (~10,000 entries) | Cached query vectors, ephemeral | Process memory | Exclusion — not durable, regenerable on miss; no backup applies | Excluded with reason |
| Worker Durable Object `QueryCache` entries (`entry.embedding_json`) | Per-key query embedding + result, retained indefinitely, no TTL | Cloudflare Durable Object SQLite (hosted) | Exclusion — regenerable cache, re-embed on miss; no export path named in the design | Excluded with reason |
| D1 hot cache `atlas-hot-cache` | No vectors (opaque cached payloads; storage layer only, not wired into the request path) | Cloudflare D1 | Out of scope — no vectors; D1 itself is class D (#13458) if in scope | Out of scope (no vectors) |
| OpenClaw memory-lancedb store | Agent-memory vectors, Qwen3-Embedding-0.6B, 1024d — incomparable with the Atlas Snowflake space; Atlas re-embeds at promotion | Local files under `~/.openclaw` (plugin default; subpath not pinned in repo) | Existing leg: openclaw-backup daily tar.zst → local + R2 + Drive, 30-day retention | Covered by existing leg |
| Staging DBs / registry DB / gate records | No vectors by design (a staging DB never stores vectors) | Local SQLite/libSQL files | Out of scope — no vectors; staging files are working state, rebuilt from sources | Out of scope (no vectors) |
| Feedback JSONL + query traces (`~/.local/state/atlas-search/`) | No vectors (votes, timings) | Local files | Out of scope — no vectors | Out of scope (no vectors) |

## Uncovered stores and their follow-ons

- Local scan-shard files and local side-store vector files: **#14032**
  (mesh file leg — verify the live homes, then land a restic file leg or
  record the exclusion). Queued as `repo-health-code-pilot` task 21637.

## Runtime checks still open (owned by #14032)

- Which shard mode the live service runs (`ATLAS_SHARDS` — `turso[:org]`
  or `dir:<path>` — and the dir if local).
- Whether `ATLAS_VEC_DB` is set anywhere live.
- Whether the query-embedding cache Turso DB exists (`turso db list` —
  if it exists it is a class-B rotation member by enumeration).

## Checked and found to hold no vector store

- `fleet-search` (`pkgs/fleet/crates/fleet-search`) embeds queries via
  atlas-core but stores no vectors.
- The `embedding` hits outside atlas-search are HTML/text embedding
  (`pkgs/id`, `pkgs/lab-ops-hub`), not vector storage.
- No qdrant / pgvector / Chroma / Weaviate / Pinecone / Milvus / FAISS /
  LanceDB (other than OpenClaw's memory-lancedb above) / `sqlite-vec`
  anywhere in the tree.
- The ONNX model snapshot is model weights, not an embedding store; it
  is re-provisionable and out of scope.

## Mesh leg status at inventory time (2026-11-23)

- #13455 (class A) — open, `status:in-review`; slice 1 PR #13476.
- #13456 (class B rotation) — open. #13457 (replication) — open.
- Until those land, Turso-homed stores rely on cold-store-nightly
  (full export → Drive, nightly) + Turso PITR (30d) + Delete Protection.
