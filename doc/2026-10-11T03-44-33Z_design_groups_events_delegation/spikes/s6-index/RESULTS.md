# S6: local index storage, redb versus SQLite

## Question

Which store holds the local tag index for the `id` design (§9.2, Phase 1 S6): alpha and omega keys, a unique (author, seq) mapping, and three queries (claims on a subject, subjects with name=value, claims by author)? Exit: the store is chosen with numbers, and durability and concurrency are recorded.

## Setup

- Workspace `/home/user/work/spikes-s6`, its own workspace root, not a member of `/home/user/work/spikes`. Code in `src/`, scripts in `scripts/`, output in `run/results/`.
- rustup 1.97.0 cargo, invoked as `"$CARGO"`.
- Crates: redb 4.3.0 (pinned); rusqlite 0.40.2 with `bundled` (libsqlite3-sys 0.38.2); blake3 1.8.7; hex 0.4.3; anyhow 1.0.104.
- Host: Ryzen 7 7840U, 16 threads, ext4 on NVMe, so fsync is real.
- redb: one write transaction per batch, default durability (`Immediate`).
- SQLite: WAL, `synchronous=FULL`, `BEGIN IMMEDIATE` per batch, busy timeout 5 s.
- Both stores use the same blob keys, encoded as in `pkgs/id/src/tuple.rs`. Tables: `claims` (id → body), `alpha`, `omega`, `author_seq` ((author, seq) → id).

## Commands run

```
s6-index selftest
s6-index unique {redb,sqlite} run/unique
s6-index bench {redb,sqlite} run/bench-X 200000     # one process per store
s6-index durable-test {redb,sqlite} 20 run/durability-X
s6-index concurrency run/concurrency
s6-index compact-redb <copy of the bench file>
cargo clean && cargo build --release --timings      # clean compile
cargo generate-lockfile in run/conflict/rusqlite-V  # resolver only
```

## (b) Load and lookups

N = 200,000 claims in 200 transactions of 1,000. Subjects are 32 bytes, names 8–12 bytes, values 16–24 bytes, and 5% of values are null. After a reopen, 1,000 random lookups of each shape were timed, and each result was checked against answers computed in memory.

| | redb | SQLite (WAL, FULL) |
|---|---|---|
| Insert total | 18.6 s | 37.4 s |
| Per claim | 93 µs | 187 µs |
| Per transaction p50 / p99 | 94 / 148 ms | 177 / 375 ms |
| On disk after close | 269.5 MB (134.9 MB after compaction) | 97.4 MB |
| Peak RSS (VmHWM) | 193 MB | 104 MB |
| Subject lookup (~5 rows), p50 / p99 | 24 / 68 µs | 57 / 131 µs |
| name=value lookup (~1 row), p50 / p99 | 5.7 / 24 µs | 11.9 / 22 µs |
| Author lookup (~2,000 rows), p50 / p99 | 4.0 / 11.7 ms | 14.7 / 49.3 ms |

Peak RSS includes about 41 MB of in-memory dataset in both processes. redb's default 1 GiB cache limit was not tuned.

Clean compile, release profile:

- redb 4.3.0 alone: 5.6 s wall (18.6 s CPU).
- rusqlite 0.40.2 bundled alone: 26.1 s wall (28.7 s CPU). Most of it is the bundled SQLite C build.
- Full spike, clean: 33.6 s wall. The libsqlite3-sys build script takes 29.8 s (26.3 s in an earlier run).

## (c) Durability

SIGKILL at a random time from 200 to 3,000 ms, 20 trials per store. The child commits batches of 1,000 and prints an acknowledgement after each commit. The parent reopens the store and checks that each batch is complete or absent, that each row matches its expected content, and that the count fits the acknowledgements.

| | redb | SQLite |
|---|---|---|
| Trials passed | 20 / 20 | 20 / 20 |
| Batches acked / present (total) | 1,030 / 1,039 | 430 / 440 |
| Partial batches / mismatches | 0 / 0 | 0 / 0 |
| Present = acked / acked + 1 | 11 / 9 | 10 / 10 |

"Acked + 1" means a batch committed just before the kill, before its acknowledgement printed. SIGKILL tests process death, not power loss.

## (d) Concurrency

- redb: a second `Database::open` fails with `DatabaseAlreadyOpen` while a writer holds the file. `ReadOnlyDatabase::open` fails the same way. redb offers no read-only sharing while a writer runs.
- SQLite WAL:
  - A second process read 1,000 committed rows (0.45 ms) while a third held 1,000 uncommitted rows.
  - Four readers ran at once, and all succeeded.
  - A reader holding a read transaction saw 2,000 rows while a writer committed 1,000 more. The writer took 26.5 ms and was not blocked. After COMMIT the reader saw 3,000.
  - A second writer with a 500 ms busy timeout failed after 510 ms with `database is locked`. It succeeded after the holder released (28.6 ms).

## (e) Unique (author, seq)

- redb: `insert_batch` looks up (author, seq) inside the write transaction before writing any table. An identical replay is a duplicate. A different claim at the same position is refused with a fork error, and the transaction aborts. Three refused batches, one with a conflict inside it, left the counts at [1, 1, 1, 1]. redb has no constraint of its own. A raw write of the mapping succeeded silently and left `by_author` failing, so the rule holds only if every write goes through this path.
- SQLite: `author_seq` is a WITHOUT ROWID table with `PRIMARY KEY (author, seq)`. SQLite reports it as unique index `sqlite_autoindex_author_seq_1` (unique=1, origin=pk). The same application check runs in `BEGIN IMMEDIATE`. A raw INSERT is refused with `UNIQUE constraint failed: author_seq.author, author_seq.seq`.

## (f) Dependency conflict

- The shared `Cargo.lock` (read only) pins libsqlite3-sys 0.37.0 for sqlx-sqlite 0.9.0, which p2panda-store pulls in. sqlx-sqlite 0.9.0 requires `>=0.30.1, <0.38.0` (its Cargo.toml, line 142).
- rusqlite 0.40.2 requires libsqlite3-sys `^0.38.1` / `^0.38.2`. Cargo fails with "failed to select a version for `libsqlite3-sys`", listing 0.30.1 through 0.37.0 as the versions that satisfy the other requirement. It also reports that libsqlite3-sys links the native library `sqlite3` and that only one package in a graph may do so.
- Resolver-only check (`sqlx = "0.9.0"` with runtime-tokio and sqlite, plus `rusqlite = "=V"` bundled; nothing compiled):
  - Resolves: 0.32.0 through 0.39.0. Each picks the newest libsqlite3-sys in both ranges, from 0.30.1 (for 0.32.x) up to 0.37.0 (for 0.39.0).
  - Conflicts: 0.30.0 (needs ^0.27.0), 0.31.0 (^0.28.0), and 0.40.0 through 0.40.2 (^0.38.x).
- This used sqlx directly. p2panda-store was not built. The bound is the one it inherits.

## Exit criteria

| Criterion | Result |
|---|---|
| Store chosen, with numbers | Pass. redb, below. |
| Durability recorded | Pass. See (c). |
| Concurrency recorded | Pass. See (d). |

## Recommendation: redb

Use redb 4.3.0 for the local index.

- Writes are 2× faster (93 against 187 µs per claim). Lookups are 2.1–3.6× faster at p50 on all three shapes, and at p99 1.9–4.2× faster, except name=value, where the two tie.
- Pure Rust. The clean build takes 5.6 s, against 26.1 s for bundled SQLite. redb 4.3.0 is already in the shared lock, so it adds no crate.
- It does not conflict with p2panda-store.

Costs we accept:

- The store has no unique constraint. The (author, seq) rule is code, so all index writes go through one tested module.
- The file is 2.8× SQLite's before compaction and 1.4× after. Peak RSS is 1.9× larger, partly from the untuned 1 GiB cache.
- Only one process can open the file. That fits the design, where the node is the only writer and the CLI goes through the server.

Choose SQLite WAL instead only if the CLI must read the index directly while the node runs, or SQL becomes necessary. If p2panda-store is adopted, rusqlite must then stay at 0.39.x or lower until p2panda-store relaxes its bound.

## Deviations

- The first omega prefix scan used the standard successor as its upper bound. The selftest found 4 rows where 3 were correct: name `a` matched `a\0`, whose escaped form starts with the same bytes. The fix is an upper bound of `prefix + 0xFF`. `pkgs/id/src/tuple.rs` uses the same escaping. I read it but did not run it. A prefix query ending at a string terminator likely has the same trap, and §9.2 should say so.
- SQLite's uniqueness comes from a PRIMARY KEY on a WITHOUT ROWID table, not a separate `CREATE UNIQUE INDEX`. It is one B-tree with the same enforcement. SQLite does not use native multi-column indexes, so the comparison is between engines.
- tempfile was declared, never used, and removed.
- Read-only use of `/home/user/work/spikes`: its README, `Cargo.lock`, and `Cargo.toml`. Nothing there was built or edited.
- Side effect: my first shell calls ran from `/home/user/code`. rustup applied that directory's `rust-toolchain` file and installed nightly-2023-07-20 (6 components). No file in `/home/user/code` was changed.
- Not tested: power loss, fsync failure, a tuned redb cache. Commit time is dominated by this host's fsync (p50 94 ms redb, 177 ms SQLite). I did not profile it.

## Blockers

None. All five parts ran.
