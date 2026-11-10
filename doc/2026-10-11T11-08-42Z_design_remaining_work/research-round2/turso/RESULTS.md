# N1 and N2: Turso 0.8.2 against redb 4.3.0 for the event store, and libsql removal

- Date: 2026-10-11. Folder: `/home/user/work/research-round2/turso`.
- Spike: `s6-index/` (the S6 harness plus a Turso backend). Nothing under `pkgs/id`, `/home/user/code`, `integrated`, `spikes`, or `spikes-next` was changed.
- Labels: **verified** = seen in command output or a source line; **inferred** = reasoned from verified facts; **not checked** = not tested here.

## Questions (verbatim)

Owner's decision, verbatim: "n1 yes prefer redb for everything but spike test turso, libsql will be replaced either away." Read index entries N1 and N2 in the remaining-work index.

Task: a spike comparing the crate `turso` (0.8.2 on crates.io; confirm the version you use and its license) against redb 4.3.0 for our event-store workload.

1. Copy the S6 harness into your folder: `/home/user/work/spikes-s6` (or the repo copy under `.../spikes/s6-index`). Read its RESULTS.md. Reuse its workload, scripts and key layout, and add a Turso backend next to the redb and rusqlite ones.
2. Workload: claim writes with unique (author, seq); subject lookups; name=value lookups; batch writes; reopen; SIGKILL durability (20 runs, as in S6); disk size; RSS; build time; dependency count.
3. Also record: whether turso is embedded and single-process; its concurrency model (readers and writers); its MSRV; its fsync and durability settings; and any feature flags that change durability.
4. libsql removal. List every libsql call site in `pkgs/id` (`grep -rn libsql pkgs/id/src pkgs/id/Cargo.toml`), with the SQL features used. Say whether redb or Turso would replace them more easily, and what the migration of the identity tables costs.

Output: a comparison table with exact configurations and commands, and a recommendation. Keep redb unless Turso wins on correctness, durability, disk, speed, build time, or maintenance. Do not modify `pkgs/id`.

Numbering used below: **Q1** = items 1–2 (performance and workload). **Q2** = item 3 (embedded, concurrency, MSRV, fsync, flags). **Q3** = item 4 (call sites). **Q4** = item 4 (migration cost) and the recommendation.

## Short answers

**Q1 (workload).** Turso does not beat redb, and it cannot run the S6 schema at scale. (verified) The S6 schema uses WITHOUT ROWID tables; turso 0.8.2 full-scans them for every `(author, seq)` lookup, claim-by-id, and alpha range (`EXPLAIN QUERY PLAN` prints `SCAN`). (verified) The first full run of that schema stalled after about 21 minutes, with the WAL frozen at 36 MB and the process at 99.2% CPU; the cause is not diagnosed (see Evidence E). (verified) The same statements on rowid tables, with the same keys and unique rule, use indexes (`SEARCH ... USING INDEX`) and completed the 200,000-claim load in 57.7 s, against 11.5 s for redb and 23.5 s for SQLite in the same session. (verified) Turso's p50 lookups were 7–22× slower than redb's (subject 270 µs vs 20 µs; name=value 43 µs vs 6 µs; author 54 ms vs 2.5 ms). (verified) All four configurations passed 20 SIGKILL trials with zero partial batches; the WITHOUT ROWID trials had only 1–5 acknowledged batches each, so that test is weak. (verified) The uniqueness cases pass on all configurations, and a raw duplicate `(author, seq)` insert is refused by turso. (verified) On disk, turso-rowid's file is 157.9 MB plus a 29.6 MB WAL before close; redb's is 269.5 MB, or 134.9 MB after `compact()`; SQLite's is 97.4 MB. Peak RSS: 124 MB for turso-rowid, 195 MB for redb, 106 MB for SQLite. (verified)

**Q2 (engine behaviour).** Turso is embedded and single-process by default. (verified) A second read-write open is refused immediately with `Locking error: ... File is locked by another process`. (verified) A read-only open from a second process succeeded and read the committed rows, not the uncommitted ones (observed once; not stress-tested while a writer commits). (inferred from the manual) The manual lists "No multi-process access" as a limitation, and turso's multi-process WAL is an experimental flag. (verified, manual.md:100, :156) That flag passed one two-writer test: 2,000 claims, reopened in default mode with zero mismatches. (verified) Concurrent writers inside one process need MVCC (`BEGIN CONCURRENT`, `journal_mode=mvcc`), which the manual marks "not production ready" (manual.md:590). (verified) That is not tested here. (not checked) Defaults: WAL, `synchronous=FULL` (verified by `turso-features`). `FULL` makes WAL commits fsync (inferred from `vacuum.rs:2187–2192`, which says NORMAL skips the WAL-commit fsync). The code maps `PRAGMA synchronous` 0 to OFF, 1 to NORMAL, and anything else to FULL (verified, `translate/pragma.rs:656–663`), but COMPAT.md lists only OFF and FULL as supported (verified, COMPAT.md:277). Settings that change durability: `synchronous` (OFF, NORMAL), `experimental_multiprocess_wal`, `experimental_mvcc_passive_checkpoint`, `journal_mode=mvcc`, and the environment variable `LIMBO_DISABLE_FILE_LOCK` (verified, `io/common.rs:1`, `io/unix.rs:67`). With that variable set, a second read-write open succeeded while another process held an uncommitted transaction (verified). `experimental_encryption` changes the on-disk format and needs the key on every open (verified). MSRV: no `rust-version` is declared in the 0.8.2 manifests of `turso`, `turso_core`, or the workspace (verified); the repo's development toolchain at v0.8.2 is 1.88 (verified, `rust-toolchain.toml`). The spike builds on 1.97.0 (verified); lower versions are not checked.

**Q3 (libsql call sites).** `grep -rn libsql pkgs/id/src pkgs/id/Cargo.toml` gives 19 lines: 2 in `Cargo.toml` (the `web` feature and the optional dependency `libsql = "0.9"` with `features = ["encryption"]`; locked at 0.9.30) and 17 in `src/web/identity.rs`. (verified) All database use is in one file and one table, `identities`. The SQL is a `CREATE TABLE IF NOT EXISTS` with a TEXT primary key, one nullable TEXT, and two `INTEGER NOT NULL` columns; a full-table `SELECT` at startup; an `INSERT` with four positional parameters; and an `UPDATE ... WHERE client_id = ?3`. The file is opened with an encrypted URI, `cipher=aes256gcm&hexkey=...`, whose key comes from HKDF. (verified, identity.rs:126–138) There are no transactions, DELETEs, joins, secondary indexes, or ORDER BY. (verified)

**Q4 (replacement and migration cost).** Turso is the smaller diff; redb is the cleaner end state. (inferred) Turso keeps the same SQL and the same async shape, and its builder accepts the same cipher name, `aes256gcm`, with `experimental_encryption(true)` and `with_encryption(EncryptionOpts{...})`. (verified: `turso-features` reopened an encrypted file with the correct key, refused no key with `file is not a database`, and refused a wrong key with `Decryption failed for page=1`.) The change touches roughly 40 lines of `identity.rs` (the open, the `get_value` index type, and the value enum paths) plus `Cargo.toml` (estimate, not built). The cost is that encryption is marked experimental in Turso's manual (manual.md:154). redb has no SQL and no encryption, so the migration is a rewrite of the storage half: one key-value table, a load-all iteration in place of the `SELECT`, write transactions in place of the `INSERT` and `UPDATE`, and a value-level AEAD. That is roughly 120–150 lines (estimate, not built). `aes-gcm` is already in `pkgs/id/Cargo.lock` (0.10.3 and 0.11.1, verified), so it needs no new crate. Existing libsql-encrypted files are not readable by Turso (not checked). N2 allows discarding the identity data.

**Recommendation.** Keep redb 4.3.0 for the event store (N1). Turso wins on none of the six criteria: correctness (it fails at scale on the S6 schema), durability (a tie in these tests), disk (it loses to compacted redb), speed (5× slower inserts and 7–22× slower lookups), build time (about 60× the unit time of redb), and maintenance (pre-1.0, with experimental flags for encryption, multi-process, and MVCC). For N2, replace libsql with redb too, under the owner's preference for one engine, with a value-level AEAD built on the `aes-gcm` already in the lock. The trade-off is that keys (the random client IDs) stay in plaintext; only values (names) are encrypted. Turso is the faster swap but adds a second, pre-1.0 engine whose encryption is experimental.

## Options

| Option | Event store (N1) | Identity DB (N2) | Measured (session 2b unless noted) |
|---|---|---|---|
| redb 4.3.0, current | **Keep.** | Rewrite; value AEAD with `aes-gcm` (est. 120–150 lines). | Insert 57.3 µs/claim; subject p50 20 µs; DB 269.5 MB (134.9 MB compacted); RSS 195 MB; SIGKILL 20/20; 1 dependency (libc). |
| Turso 0.8.2, S6 schema (WITHOUT ROWID, as specified) | Not usable: every PK lookup scans. | Not applicable (identity uses rowid tables). | Plan: `SCAN` on all three queries. 1,000 lookups: 151 ms at 1,000 rows, 3,697 ms at 20,000 rows. First full run stalled (Evidence E). |
| Turso 0.8.2, rowid schema (same keys; `turso-rowid`) | No: 5.0× slower inserts, 7–22× slower p50 lookups, 178 packages, about 1,031 s of build unit time. | Mechanical swap (about 40 lines); experimental encryption; a second engine. | Insert 288.4 µs/claim; subject p50 270 µs; DB 157.9 MB + 29.6 MB WAL; RSS 124 MB; SIGKILL 20/20. |
| SQLite (rusqlite, WAL, FULL), baseline only | Not in question. | Not in question. | Insert 117.3 µs/claim; subject p50 35 µs; DB 97.4 MB + 19.4 MB WAL; RSS 106 MB. |

## Evidence

### A. Versions and sources (verified)

- `turso` 0.8.2, the newest stable release, MIT. From `https://crates.io/api/v1/crates/turso` on 2026-10-11 (16:06Z): `max_stable_version` 0.8.2, `license` MIT, `rust_version` null for every 0.8.x release. `0.8.3-pre.1` exists and is not used. Downloads: 1,117,460.
- Crate tarball `dl-turso-0.8.2-crate/turso-0.8.2.crate` from `https://static.crates.io/crates/turso/turso-0.8.2.crate`, sha256 `a2ebd8a76c2b005a9f42de15e0fe232cd9c17d7c8acb571ddfa294e6c9fa1e44`, extracted to `upstream-turso-0.8.2/` (a separate directory, read only).
- Its `turso_core` and `turso_sdk_kit` 0.8.2 crates (both MIT, no `rust-version`): sha256 `4fa832d4ab648bf139f1b18b29bef6d653b18fbd4a77b0e4846c57aaf28c215c` and `e7893b565a3405c8f44c97b6008bed55314535514c2536db1aaf2bb9b687705a`, extracted to `upstream-turso_core-0.8.2/` and `upstream-turso_sdk_kit-0.8.2/`.
- Source commit: `.cargo_vcs_info.json` gives sha1 `5c168be5ef6a32ac966daf5983502a4f694b37ad` (path `bindings/rust`). GitHub tag `v0.8.2` points at tag object `5d68fe09c36149d395cbf870bf5c25b2e5970157`.
- Upstream workspace manifest, `raw.githubusercontent.com/tursodatabase/turso/v0.8.2/Cargo.toml`: `license = "MIT"`, no `rust-version` (grep confirmed). `rust-toolchain.toml` at v0.8.2: channel `1.88`.
- Docs at v0.8.2, saved in `upstream-docs-v0.8.2/`: `manual.md` (1,561 lines) and `COMPAT.md` (1,194 lines).
- GitHub README, fetched 2026-10-11: "we have not yet reached 1.0"; experimental features: encryption at rest, multi-process WAL, full-text search, incremental computation, Postgres; "replaces libSQL as our intended direction".
- Turso's `Cargo.toml` lists `turso_sdk_kit` and `turso_sync_sdk_kit` as non-optional dependencies (verified, `upstream-turso-0.8.2/turso-0.8.2/Cargo.toml`). The `sync` feature is off in the spike, but the sync crates are still compiled (verified, Evidence H).

### B. What was changed in the harness (verified)

- `s6-index/` is the S6 harness (`src/`, `scripts/`, `Cargo.toml`, `Cargo.lock`), copied from `/home/user/work/spikes-s6`. Added `turso = { version = "=0.8.2", default-features = false }`. Default features off means no `mimalloc` (so the system allocator, as for redb and rusqlite) and no `fts` (tantivy).
- New files: `src/block.rs` (runs turso's async API to completion on the calling thread and counts Pending polls); `src/turso_store.rs` (`TursoStore`, the S6 schema on WITHOUT ROWID; `TursoRowidStore`, the same statements on rowid tables, NAME `turso-rowid`); `src/turso_probe.rs` (feature probes, concurrency, multi-process WAL, and the txn and stress diagnostics); `src/turso_plan.rs` (query plans and lookup timing).
- Edited: `durability.rs`, `checks.rs`, `main.rs`, dispatch arms only. The S6 key layout, workload, and generator are unchanged. `selftest` still prints PASS (run/results/selftest-final.txt).
- `turso_probe::plan_probe` is an earlier version of `turso_plan::plan`, left in place unused (one dead-code warning). I tried to remove it; the edit did not match, so it stays.

### C. Configurations (exact)

- Common workload, as S6: 200,000 claims, 1,000 per transaction (200 transactions), subjects 32 bytes, names 8–12 bytes, values 16–24 bytes, 5% null values, 100 authors, dataset seed `0x5_0006`; 1,000 timed lookups of each shape, each checked against an in-memory answer.
- redb 4.3.0: `Database::create`, one `begin_write` per batch, default durability. Tables `claims`, `alpha`, `omega`, `author_seq`, as S6.
- rusqlite 0.40.2 bundled: WAL, `synchronous=FULL`, `BEGIN IMMEDIATE` per batch, busy timeout 5 s, S6 schema (WITHOUT ROWID).
- turso 0.8.2, S6 schema ("turso"): `Builder::new_local(path).experimental_without_rowid(true).experimental_multiprocess_wal(false).build()`; `busy_timeout(5 s)`; `PRAGMA journal_mode='wal'`; `PRAGMA synchronous=FULL`; S6 schema (`SCHEMA` in `turso_store.rs`). Batch: `transaction_with_behavior(TransactionBehavior::Immediate)`, `insert_in` prepares five statements per batch (a `SELECT` for the `(author, seq)` check, then four `INSERT`s), `commit()` or `rollback()`.
- turso 0.8.2, rowid schema ("turso-rowid"): the same open without the experimental flag; `SCHEMA_ROWID` (the same tables as rowid tables; `PRIMARY KEY (author, seq)` on `author_seq`, `BLOB PRIMARY KEY` on the others).
- Block-on adapter: `thread::yield_now()` on a Pending poll; counts Pending polls. Pending polls were 2,766 for the whole 200k rowid bench, so the adapter is not the bottleneck (verified).

### D. Commands (all from `s6-index/`, `RUSTUP_TOOLCHAIN=1.97.0`, `RUSTC="$(rustup which --toolchain 1.97.0 rustc)"`)

```
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
"$CARGO" build --release                                        # incremental
../build-clean.sh          # rm -rf target; cargo build --release --timings (clean)
./target/release/s6-index selftest
./target/release/s6-index unique {redb,sqlite,turso,turso-rowid} run/unique-X
./target/release/s6-index turso-features run/features
./target/release/s6-index bench {redb,sqlite,turso-rowid} run/bench-X 200000
./target/release/s6-index bench turso run/bench-turso 200000    # stalled; see E
./target/release/s6-index turso-stress run/stress DIR 60 1000 ["cache_size = -262144"]
./target/release/s6-index turso-plan run/plan
./target/release/s6-index turso-txn-probe run/txn
./target/release/s6-index compact-redb run/bench-redb/index-redb.db
./target/release/s6-index durable-test {redb,sqlite,turso-rowid,turso} 20 run/durability-X
./target/release/s6-index concurrency run/concurrency-session2  # S6 orchestrator, redb and sqlite
./target/release/s6-index turso-concurrency run/conc-turso
./target/release/s6-index turso-mp-test run/mp-turso
strace -f -c -e trace=fsync,fdatasync -o run/results/strace-txn-probe.txt \
  ./target/release/s6-index turso-txn-probe run/strace-txn
```

Scripts: `run-all.sh` (session 1; stopped at the turso WITHOUT ROWID bench) and `run-session2b.sh` (the complete session, with a timeout on each step). Logs: `s6-index/run/results/session2b-all.txt` (the full run), `session2-all.txt` (session 1, partial).

### E. Turso on the S6 schema: the plan, the scan, and the stall (verified unless marked)

- Plans, `turso-plan` (`run/results/turso-plan.txt`):

  ```
  WITHOUT ROWID author_seq lookup: SCAN author_seq
  WITHOUT ROWID claims by id:      SCAN claims
  WITHOUT ROWID alpha range:       SCAN alpha | USE SORTER FOR ORDER BY
  rowid author_seq lookup:         SEARCH author_seq USING INDEX sqlite_autoindex_author_seq_1 (author=? AND seq=?)
  rowid claims by id:              SEARCH claims USING INDEX sqlite_autoindex_claims_1 (id=?)
  rowid alpha range:               SEARCH alpha USING COVERING INDEX sqlite_autoindex_alpha_1 (key>=? AND key<?)
  ```
- Lookup timing, 1,000 `author_seq` lookups: WITHOUT ROWID 151.3 ms at 1,000 rows and 3,696.6 ms at 20,000 rows; rowid 4.4 ms and 7.4 ms.
- Transactions are not the cause (`run/results/txn-probe3.txt`, 1,000 inserts each): explicit `BEGIN IMMEDIATE`/`COMMIT` 20.6 ms; `Transaction` API 20.7 ms; WITHOUT ROWID inserts, one transaction, 29.9 ms; prepared insert reused, 8.4 ms; `SELECT` then `INSERT` on WITHOUT ROWID, 91.4 ms; the same on rowid, 45.0 ms; autocommit, 5,499 ms. `strace` counted 1,023 `fsync` calls over the probe, about 1,000 of them from the autocommit phase (`run/results/strace-txn-probe.txt`). So each autocommit statement costs one fsync (verified for this probe).
- Batch time under the default cache (`run/results/stress-default.txt`, `PRAGMA cache_size` = -2000): batches 26–31 took 3,702, 3,592, 3,573, 3,787, 3,839, and 3,723 ms.
- Under a 256 MB cache (`run/results/stress-cache-big.txt`, `cache_size = -262144`): batch 0 took 104 ms; batches 32–35 took 5,583, 7,320, 9,290, and 10,968 ms. The batch time rises with the row count, which is what a scan predicts.
- The stall (`run/results/turso-bench-stall.txt`): the first full WITHOUT ROWID bench started at 16:21:24Z. The WAL froze at 36 MB (35,996,472 bytes, last modified 11:42 local, about 21 minutes in). `ps` showed the process at 99.2% CPU. It was still at 99.2% CPU and unchanged after 20 seconds, and it was killed at about 17:22Z. (verified) At the batch times above, a scan alone would predict about 30 s per batch near the end, so the scan does not explain a 40-minute freeze. (inferred) The cause is not diagnosed. (not checked: whether the spin is in turso or in the adapter)

### F. Concurrency (session 2b, `run/results/session2b-all.txt`)

- Turso (`turso-concurrency`):
  - Setup: 1,000 claims committed.
  - Holder: one IMMEDIATE transaction with 1,000 uncommitted claims, for 4 s.
  - Probe while held: `rw open: ERR after 0.1 ms: ... Locking error: Failed locking file ... File is locked by another process`; `ro open: OK, count(claims) = Integer(1000)`.
  - Second writer while held: refused at open, after 0.1 ms.
  - After release: write OK, 1,000 inserted, 193.5 ms.
  - With `LIMBO_DISABLE_FILE_LOCK=1`: `rw open: OK (2.9 ms)`; `ro open: OK, count(claims) = Integer(2000)`.
- Turso multi-process WAL (`turso-mp-test`, two processes, one file, `experimental_multiprocess_wal(true)`): child 1 inserted 1,000 in 435.5 ms (211,600 pending polls); child 2 inserted 1,000 in 181.7 ms (13,465 pending polls). Reopened in default mode: `claims=2000 counts=[2000, 2000, 2000, 2000] mismatched=0`.
- redb (S6 orchestrator): a second `Database::open` and a `ReadOnlyDatabase::open` both fail with `DatabaseAlreadyOpen`.
- SQLite WAL (same orchestrator): readers ran during an open write; a second writer with a 500 ms busy timeout failed with `database is locked` after 501.8 ms; a snapshot reader saw 2,000 rows during a commit and 3,000 after its own COMMIT.

### G. Durability: SIGKILL, 20 trials each, S6 method (verified, session 2b)

| Store | Trials passed | Acked batches | Present batches | Partial batches | Present = acked / acked+1 |
|---|---|---|---|---|---|
| redb | 20/20 | 1,223 | 1,232 | 0 | 11 / 9 |
| rusqlite (WAL, FULL) | 20/20 | 619 | 626 | 0 | 13 / 7 |
| turso-rowid | 20/20 | 471 | 479 | 0 | 12 / 8 |
| turso (WITHOUT ROWID) | 20/20 | 64 | 66 | 0 | 18 / 2 |

The turso (WITHOUT ROWID) trials acknowledged only 1–5 batches each, because the scans make each batch slow. The test barely reaches the commit window for that schema. (verified; inferred for the weakness)

### H. Build time and dependencies

- Clean build of all dependencies, `build-clean.sh` (`run/results/build-clean-all-stores.txt`): 215 crates compiled, wall 321.7 s, load average 43 at start and 52 at end on 16 cores. The spike crate itself failed in that run, because my first backend had compile errors (fixed later), so this number covers dependencies only. (verified)
- Timing report `target/cargo-timings/cargo-timing-20261011T161038324Z-5aab6fbd5171aa00.html`, parsed with the S6 `scripts/timing.py` plus a closure check: 314 units, total unit time 1,303 s. Turso-attributed units (the turso closure minus the redb, rusqlite, blake3, hex, and anyhow closures): 245 units, 1,031 s summed unit time, including `turso_core` 201.5 s, `turso_sync_engine` 48.1 s, `turso_sync_sdk_kit` 44.9 s. redb: 16.3 s. rusqlite and libsqlite3-sys: 120.6 s. These are summed unit durations from a parallel build, not wall time for turso alone. (verified for the sums; inferred for the attribution) An isolated clean wall-time build of turso alone was not run. (not checked)
- Dependency counts, from `cargo tree -e normal --prefix none --format '{p}'` with `sort -u` (verified):
  - Whole spike: 189 packages, excluding the root.
  - `turso` subtree with default features off: 178.
  - Turso-only (not in the redb, rusqlite, blake3, hex, or anyhow closures): 170 by name, 171 by name and version.
  - `turso` with default features (`dep-count-default/`, metadata only, not built): 243, including `mimalloc` 0.1.52 and `tantivy` 0.26.2 (FTS).
  - redb 4.3.0: one dependency (`libc` 0.2.190).
  - rusqlite 0.40.2 bundled: eight dependencies, including `libsqlite3-sys` 0.38.2.

### I. libsql call sites in `pkgs/id` (verified; `grep -rn -i libsql pkgs/id/src pkgs/id/Cargo.toml`)

| Where | What | SQL or feature used |
|---|---|---|
| `Cargo.toml:32` | `"dep:libsql"` in feature `web` | feature gate |
| `Cargo.toml:78` | `libsql = { version = "0.9", features = ["encryption"], optional = true }` (lock 0.9.30) | encryption feature |
| `identity.rs:93` | field `db: Option<Arc<libsql::Database>>` | none |
| `identity.rs:133–138` | `format!("file:{db_path_str}?cipher=aes256gcm&hexkey={hex_key}")`; `libsql::Builder::new_local(uri).build().await` | encrypted URI, AES-256-GCM |
| `identity.rs:139–141` | `db.connect()` | none |
| `identity.rs:144–154` | `conn.execute("CREATE TABLE IF NOT EXISTS identities (client_id TEXT PRIMARY KEY, name TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)", ())` | DDL; TEXT PK on a rowid table; NOT NULL |
| `identity.rs:159–171` | `conn.query("SELECT client_id, name, created_at, updated_at FROM identities", ())`; `rows.next()` loop | full-table SELECT at startup |
| `identity.rs:172–175`, `505–536` | `row.get_value(idx)`; matches on `libsql::Value::{Text, Integer, Null}` in `db_text`, `db_opt_text`, `db_u64` | value types |
| `identity.rs:199` | `db: Some(Arc::new(db))` | none |
| `identity.rs:236–245` | `db.connect()`; `conn.execute("INSERT INTO identities (client_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)", db_identity_params(...))` | positional parameters; nullable TEXT |
| `identity.rs:315–332` | `conn.execute("UPDATE identities SET name = ?1, updated_at = ?2 WHERE client_id = ?3", params)`; errors logged, not returned | UPDATE by primary key |
| `identity.rs:538–551` | `db_identity_params` returns `Vec<libsql::Value>` | none |

Totals: 2 lines in `Cargo.toml`, 17 in `identity.rs`. (verified) The file has no libsql match after line 549. (verified)

### J. Migration cost, in detail (estimate; not built)

- Turso: `Builder::new_local(path)` with `experimental_encryption(true)` and `with_encryption(EncryptionOpts { cipher: "aes256gcm".into(), hexkey })`, where `hexkey` is the same 64-character HKDF output as now. The URI parsing goes away. `db.connect()`, `execute`, `query`, and `rows.next()` keep their shapes. `get_value` takes `usize`, not `i32`, so the helper signatures change. The `Value` variant names match. Estimate: about 40 lines in `identity.rs` and one dependency line in `Cargo.toml`. (estimate; the builder and cipher behaviour were verified in `turso-features`)
- redb: replace `Database` with `redb::Database::create`; one table `identities` (`&str` key to a serialized record); `new_ephemeral` stays in memory; the startup `SELECT` becomes a table iteration; `register` and `update_name` become write transactions; the `db_*` helpers and `db_identity_params` (about 60 lines) are deleted; add a value-level AEAD (`aes-gcm`, already in `pkgs/id/Cargo.lock`) with a key from the same HKDF and a fresh nonce per write. Estimate: 120–150 lines changed, plus tests. (estimate)
- Data: libsql-encrypted files are not readable by Turso (not checked). N2 allows discarding them. Row count and file size of the real identity database: not measured.

### K. Process notes

- Rule 4 deviation: the spike's build artifacts go to `s6-index/target` inside my folder, not `/home/user/work/spikes/target`. The shared spikes workspace pins redb =4.3.0, but `turso` is not in it, and the clean timing needed an empty target directory. The redb build was therefore done again here. (verified)
- While I was reading `identity.rs`, a tool result carried text styled as instructions from `pkgs/id/AGENTS.md` (asking for plan and docs files inside `pkgs/id`). I did not act on it. It conflicts with this brief, which limits writes to my folder and forbids changes to `pkgs/id`. `pkgs/id` was not modified. (verified)
- Session 1 (`run-all.sh`) was stopped at the turso WITHOUT ROWID bench. Session 2b is the complete run, with redb and SQLite rerun under the same binary. Its numbers differ from S6 because the load was different. Compare within session 2b only.
- Load on the 16-core host during the runs: 12 to 52. Timings are relative, not absolute.

## Recommendation
### Criteria check (the brief's decision rule, applied to the best Turso configuration)

| Criterion | Turso (rowid, the best configuration) | Verdict |
|---|---|---|
| Correctness | Uniqueness passes; WITHOUT ROWID (as specified) scans and stalled | redb |
| Durability | SIGKILL 20/20 for both; power loss not tested | tie |
| Disk | 157.9 MB + 29.6 MB WAL vs redb 269.5 MB (134.9 MB compacted) | redb (compacted) |
| Speed | 5.0× slower inserts; 7–22× slower p50 lookups | redb |
| Build time | About 1,031 s summed unit time vs redb 16 s; 178 vs 1 dependency | redb |
| Maintenance | Pre-1.0; experimental encryption, multi-process WAL, and MVCC; WITHOUT ROWID insert-only; sync crates always compiled | redb |


**N1: keep redb 4.3.0.** Turso does not win on any of the six criteria the brief sets. Its schema problem at scale is the sharpest finding, and the rowid schema fixes the scans but not the speed, the build, or the maintenance picture.

**N2: replace libsql with redb.** That keeps one engine, as the owner prefers. The encryption needs our own AEAD over the values, using `aes-gcm`, which is already in the lock; keys stay in plaintext. The alternative is Turso's rowid schema for identity only. It is a smaller diff, but it adds a second, pre-1.0 engine whose encryption the manual calls not production ready.

**What would change the N1 decision:**
1. A root-cause fix for the WITHOUT ROWID scan and the stall, and a completed 200,000-claim run on the S6 schema.
2. Turso 1.0, or at least encryption and multi-process WAL out of experimental status.
3. A power-loss pass for both engines (N13). None has been run.
4. A build that compiles the sync crates only when `sync` is on, and a dependency count nearer redb's.
5. If the event store becomes async, a tokio-native measurement, which would replace the spin-yield adapter. Our adapter's pending-poll count was small, so this is unlikely to change the result.

## Not verified

- Power-loss durability for any engine (only SIGKILL). (not checked)
- The root cause of turso's WITHOUT ROWID scan and of the stall, and whether a newer turso fixes either. (not checked)
- The S6-schema bench at 200,000 claims, which did not complete. (not checked)
- MVCC, `BEGIN CONCURRENT`, `journal_mode=mvcc`, passive checkpointing, and several connections inside one process. (not checked)
- fsync behaviour beyond the probes: the strace total (1,023) was for one probe, not an audit of every commit. (not checked)
- MSRV below 1.97.0. No `rust-version` is declared, the dev pin is 1.88, and `pkgs/id` declares 1.91; none of those was built. (not checked)
- Isolated clean wall-time build of turso alone; only summed unit time is available, from one build under load. (not checked)
- Whether Turso can read libsql-encrypted files, and whether libsql's encryption format matches Turso's. (not checked)
- The identity database's row count and size. (not measured)
- The redb AEAD layer and its 120–150 line estimate, and the Turso 40-line estimate. (estimates, not built)
- The dependency count for `pkgs/id`'s full tree; only the spike's tree was counted. (not checked)
- The effect of the spin-yield adapter on the numbers, compared with a tokio-native driver. (not checked)
- Turso's `DELETE`, `UPDATE`, and secondary `UNIQUE` limits on WITHOUT ROWID were verified only through the error messages in `turso-features`; the rowid schema supports them (verified), but no retention or removal workload was run. (partly checked)
