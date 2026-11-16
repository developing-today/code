## Standing rule: creating Turso databases (Drew ruling 2026-11-04 18:28 CST)

**All new Turso databases are created as TursoDB engine, via the platform API equivalent of the dashboard toggle:**

```
POST https://api.turso.tech/v3/organizations/{org-UUID}/databases
{"name": "<db-name>", "group_id": "<group-UUID>", "use_tursodb": true}
```

Facts established empirically during the atlas shard restore (2026-11-04):

- The field is `use_tursodb` (from turso-cli source, `internal/turso/databases_v3.go`: `CreateDatabaseV3Body.UseTursoDB`; the CLI's `--tursodb` flag sets the same field). Same route the CLI v1.0.34 uses.
- The `{org-UUID}` path segment must be the organization **UUID** (`5980ebf0-b064-45ba-9d5f-f2f9ec077f6a` for dezren39) — the org slug returns `organization not found`. Group is likewise the group **UUID** (great-star-lord = `16d3864a-3343-11ee-b78e-3698c249076c`).
- Auth: platform API token (`turso auth token` / a minted API token), `Authorization: Bearer …`. Database access afterwards is per-DB tokens as usual (`turso db tokens create`).
- Verify engine after creation with two independent signals: `turso db show <db>` → Type: Turso, and `SELECT sqlite_version()` → 3.50.4 (libSQL DBs report 3.47.0).
- Existing libSQL databases (e.g. production `atlas`) are unaffected — there is no in-place conversion; the rule applies to every NEW database (staging DBs, core stores, registry, caches). First production use: the 31-DB atlas core restore (record on developing-today/code#5669) and the registry/embedding-cache pilots.
- Engine capability notes for schema design: `vector_distance_cos` works, `F32_BLOB(n)` accepted; metric functions available: cosine, dot product, L2, Jaccard. `libsql_vector_idx` is rejected, and **FTS5 virtual tables are not supported in MVCC mode** — schemas needing FTS must plan for that explicitly.
