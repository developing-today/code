# RESULTS: p2panda-store fit for n3 (traits over redb, or a fork)

- Date: 2026-10-11. Pins: p2panda `3e84cac48bba05c6857cce2ec433862b97f8d53c` (the store crate reports 0.7.1), redb `=4.3.0`, cbor-core `=0.10.1`, Rust 1.97.0.
- Scope: read-only analysis of the p2panda checkout, the 0.7.1 extraction, spike code and design docs. No repo was modified. Three small crates in this folder (`sendcheck`, `nosqlite`, `semcheck`) were built with the shared target dir and the spikes lock to check specific claims.
- Tags: **[V]** verified (command output seen, or a source line read); **[I]** inferred from [V]; **[N]** not checked.

## Questions (verbatim)

Owner's decision: "for n3 a fork isn't rejected because nothing has to keep sqlite we'd own the fork-- but i prefer traits over redb like you do."

Owner's question (context): "what does chainext do?"

Research questions:

1. Read p2panda-store at rev 3e84cac (checkout under `~/.cargo/git/checkouts/p2panda-*/3e84cac/p2panda-store`, and the 0.7.1 extraction under `/tmp/research/p2p-store-*`). List each trait: `TopicStore`, `CursorStore`, `KeySecretsStore`, `LogStore`, `OperationStore`, `GroupsStore`, `OrdererStore`, `Transaction`. For each: its methods, the semantics callers rely on (atomicity, transactions, ordering, uniqueness, latest-entry queries, deletes), and which crate calls it.
2. Read how p2panda-sync (LogSync, TopicLogSync) and p2panda-stream use these traits. Which traits does a LogSync session need? Which methods?
3. Map our needs to them: ChainExt-based chains (per-author sequence, `after` frontier), fork freeze and proof storage, waiting events with caps, tuple-keyed subject and name/value indexes, unique (author, seq). For each need, say whether a trait covers it, whether we need an extra method, or whether it sits outside the traits.
4. Find SQL-only semantics in the SQLite implementation that a redb implementation would have to reproduce (ordering by height, paging, transaction isolation, constraints). Name the lines.
5. Estimate effort in lines and risk for (a) a redb implementation of the traits, and (b) a fork that replaces the SQLite implementation with redb, keeping the traits unchanged.
6. Recommend. The owner prefers traits over redb, and accepts a fork if we own it. Say what would change the recommendation.

Do not modify any repo. Output: RESULTS.md with a fit matrix (need by trait method).

## Short answers

**ChainExt (context).** ChainExt is the header extension signed with every event: `kind` (text), `after` (at most 32 hashes: the author's causal frontier), `group` (optional hash), and a domain tag (`id.events.v1`). It sits inside the signed header, so the causal and group links cannot change without breaking the signature. The per-author chain (`seq_num` and `backlink`) is p2panda's own header, not ChainExt. [V: `spikes/s1-transport/src/header.rs:13–18`; design `explainers.md` §1]

**1. Trait inventory.** The eight traits have 36 methods between them at the pin (Transaction 3, LogStore 6, OperationStore 7, TopicStore 5, CursorStore 3, OrdererStore 8, GroupsStore 2, KeySecretsStore 2) [V]. Callers rely on one store-wide write transaction (`_tx` methods and most writes fail with `TransactionMissing` outside one [V]), latest entry by seq read inside the ingest transaction, idempotent insert by hash, heights and sizes over ranges, topic to author to logs resolution, named cursors, and group state by hash [V, call sites in the table below]. Deletes are prune (whole rows [V]), `delete_operation` (tests only), and the payload delete (no callers). No trait enforces (author, seq) uniqueness or fork handling [V].

**2. LogSync.** A LogSync session needs only `LogStore`: `get_log_heights`, `get_log_size` and `log_entries`, plus `Clone + Send + 'static`. It needs no `OperationStore`, `TopicStore` or `Transaction` [V: log_sync.rs:97, 122, 171, 292; api.rs:573–630]. TopicLogSync adds `TopicStore::resolve` [V: topic_log_sync.rs:59–60, 132–135]. The protocol writes nothing; received operations go to the application, which ingests them with `Transaction`, `OperationStore` (`has_operation_tx`, `insert_operation`), `LogStore::get_latest_entry_tx` and `TopicStore::associate` [V: ingest/operation.rs:58–61, 86, 100, 187, 191]. The 0.7.1 crate calls the sync method `get_log_entries`; the pin calls it `log_entries` [V: diff].

**3. Mapping.** Covered by trait methods: per-author append and latest (`get_latest_entry_tx` with `insert_operation` in one transaction), dedupe (`has_operation_tx`), LogSync heights, sizes and entries, and topic to logs. Partly covered: `after` (OrdererStore models dependencies and a ready queue, but "ready" means processed, not applied) and backlink uniqueness (ingest checks the backlink inside the transaction, not the store). Not covered, so an extra method or our own table: fork proof and freeze; waiting events with per-author caps and expiry (OrdererStore has neither and keeps pending items forever, design line 804); subject and name/value indexes; and store-level (author, seq) uniqueness. Range sync (S7) uses none of the traits [V]. The matrix is below.

**4. SQL-only semantics.** Twenty behaviours need reproducing or replacing; the table lists them with lines. Four matter most: (i) numeric seq ordering comes from SQLite type affinity (observed: latest is 10 over 9 [V]); (ii) (author, seq) is not unique in the schema, and two different events at one position were both stored [V]; (iii) prune deletes headers as well as bodies, although the doc says bodies [V]; (iv) one store-wide transaction, with `TransactionMissing` outside it [V]. One prediction was wrong: an empty `get_log_size` returns `Some((0, 0))`, not an error [V].

**5. Effort and risk.** (a) redb in our crate, minimal set (Transaction, OperationStore, LogStore, TopicStore, plus CursorStore if needed): about 650–900 implementation lines and 300–450 test lines [I, calibrated on S6's 162-line redb store and S7's 138-line store]. Medium risk: the store-wide transaction, streams, and no runtime evidence yet. Adding OrdererStore costs about 300–400 more lines and is the highest-risk port. (b) A fork that replaces SQLite with redb and keeps the traits: the same implementation, plus deleting 3,526 lines of SQLite code and 11 migration files [V: wc, ls], plus keeping the feature gates compiling (`groups`, `encryption`, `processor`, `spaces` each imply `sqlite` [V: Cargo.toml]), plus renaming or aliasing 209 lines that reference `SqliteStore`/`SqliteError` outside the store [V]. Medium-high risk, mostly maintenance: 18 store source files differ between 0.7.1 and the pin, so each upstream bump conflicts with the deleted files [V: diff -rq].

**6. Recommendation.** Option (a), with the minimal set, and no fork now. Two facts shape it. First, GroupsStore, KeySecretsStore, KeyRegistryStore, SpacesStore and ProcessorStore cannot be named without enabling a feature that implies `sqlite`, which brings sqlx back [V: nosqlite; Cargo.toml]. Second, p2panda-sync, p2panda-stream, p2panda-spaces and p2panda-net each enable `sqlite` [V: Cargo.toml]. So the clean path is: implement Transaction, OperationStore, LogStore and TopicStore over redb; keep our own ingest and range sync (both already in the design); keep waiting in our own table; and keep p2panda-sync and p2panda-stream out of the graph. The decision that changes this is N5: if LogSync stays for small groups, p2panda-sync brings sqlx back, and the fork becomes the only SQLite-free route, which the owner accepts. N3's premise is also wrong: S7 did not test p2panda-store (it used its own redb tables), so the fit is source-level until a port runs.

## Evidence (commands and sources)

All cargo commands used `CARGO="$(rustup which --toolchain 1.97.0 cargo)"`, `RUSTC` from the same toolchain, `CARGO_TARGET_DIR=/home/user/work/spikes/target`, `--offline`, and the copied `/home/user/work/spikes/Cargo.lock`.

**E1. sendcheck (`store-fit/sendcheck/src/lib.rs`).** `cargo check --offline`: `Checking sendcheck v0.0.0 … Finished`, exit 0 [V]. Compile-time assertions that passed:
- `WriteTransaction: Send`, `Database: Send + Sync`, and the store struct `Send + Sync`, with a `tokio::sync::Mutex<Option<WriteTransaction>>` slot and a `Semaphore`.
- An async fn that acquires the permit, runs `begin_write` through `spawn_blocking`, stores the transaction, awaits (`yield_now`), takes it back, and commits is `Send`.
- `redb::Range<'static, &[u8], &[u8]>: Send` [V].

**E2. nosqlite (`store-fit/nosqlite`).** p2panda-store with `default-features = false`. A local type implements `Transaction` and `LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash>` with stub bodies; the ungated trait modules `cursors`, `operations`, `orderer` and `topics` are imported. `cargo check --offline`: exit 0 [V]. `cargo tree -e normal`, lines matching `sqlx|libsqlite|rusqlite|tokio` other than the root crate: count 0 [V]. The normal tree is p2panda-core (cbor-core, ed25519-dalek, blake3 and others), async-stream and futures-util [V].

**E3. Feature implication (same crate).** With `groups = ["p2panda-store/groups"]` added to the root crate, `cargo tree --features groups` adds p2panda-auth, sqlx 0.9.0, sqlx-core, sqlx-macros, sqlx-postgres, sqlx-mysql and libsqlite3-sys 0.37.0 (lock update lines, exit 0) [V]. The feature table was then reverted.

**E4. semcheck (`store-fit/semcheck/src/main.rs`).** Runs against `SqliteStoreBuilder::memory()` at the pin, without `test_utils`. Two earlier runs failed on harness errors, which I corrected: inserts outside a transaction (`TransactionMissing`), and seq>0 headers without a backlink (`Decode(MissingField("backlink"))`, because `AnyHeader::decode` requires the field). The final run:

```
0 insert without tx: Err(TransactionMissing)
1 empty latest: None
1 empty heights: None
1 empty size: Ok(Some((0, 0)))
1 empty entries: []
2 latest after seq 9 and 10: Some(10)
3 hashes differ: true
3 insert fork a: Ok(true)
3 insert fork b: Ok(true)
3 log 2 size (count, bytes): Ok(Some((2, 288)))
3 log 2 heights: Some({2: 0})
3 latest on fork log is (is_a, is_b): Some((true, false))
4 log 0 all: [0, 1, 2, 3, 4]
4 log 0 after=1 until=3: [2, 3]
4 log 0 size after=1 until=3: Ok(Some((2, 348)))
4 log 0 heights: Some({0: 4})
5 prune until 3: deleted 3; has_operation(seq 0) = false
5 log 0 after prune: [3, 4]
exit=0
```

The tie on the fork log returned the first-inserted event once; the tie rule itself is not specified, so this is one observation [V]. The harness did not test a second store instance, concurrency, or other tie orders [N].

**E5. Source reads and greps.** Line numbers are from `cat -n`, `grep -n`, or `sed -n` on the pinned checkout, unless marked as 0.7.1.
- Store: `lib.rs:103–128` (module gates, re-exports); `lib.rs:112` (`macros` gate); `sqlite.rs:259–264` (store-wide transaction slot), `:275` (Semaphore(1)), `:305–313` (`tx()`, `TransactionMissing` at 310), `:341–390` (`begin`, semaphore 345–350, `BEGIN IMMEDIATE` at 386), `:396–426` (rollback, commit), `:459–476` (permit Drop: async rollback spawned); `:185–186` (connection options, no journal_mode or WAL pragma; a grep for `journal_mode|wal|synchronous|pragma` in the store source returned nothing [V]); `macros.rs:17–26` (`tx!`); `traits.rs:10–23` (Transaction).
- Trait files: `logs/traits.rs:21–89`, `operations/traits.rs:11–60`, `topics/traits.rs:41–72`, `cursors/traits.rs:8–29`, `orderer/traits.rs:16–87`, `groups/traits.rs:12–31`, `key_secrets/traits.rs:9–20`, `processor/traits.rs:9–15`, `key_registry/traits.rs:9–20`, `spaces/traits.rs:7–28`, `address_book/traits.rs:20–125`.
- SQLite impls: `operations/sqlite.rs:37–195`; `logs/sqlite/mod.rs:21–279` (SQL at 21–53); `topics/sqlite.rs:15–208`; `cursors/sqlite.rs:11–98`; `groups/sqlite.rs:14–82`; `key_secrets/sqlite.rs:14–68`; `orderer/sqlite.rs:16–528`.
- Core: `prune.rs:74–101` (`validate_prunable_backlink`); `operation/validation.rs:81–104` (`validate_backlink`); `operation/any.rs:344` (signature check in `AnyHeader::decode`); `identity.rs:196–198` (`verify` calls `verify_strict`); `operation/header.rs:121–125` (`Header::decode` calls `AnyHeader::decode`).
- Stream: `ingest/operation.rs:47–196` (ingest), `:221–247` (validate); `ingest/ooo.rs:69–74, 92–93, 292–299` (in-memory OOO buffer; `push` calls `IndexMap::pop` at 294); `orderer/orderer.rs:64–148` (CausalOrderer); `orderer/processor.rs:84–108, 122, 171, 203`; `groups/processor.rs:84–118`; `log_prune/processor.rs:57`.
- Sync: `log_sync.rs:97` (bound), `:122`, `:171`, `:188` (`if outbound_bytes > 0`; zero bytes means Done), `:292`, `:312` (`Header::<E>::decode`); `api.rs:573–589` (`log_heights`), `:596–630` (`log_ranges`), `:613` (`log_entries`), `:633–650` (`topic_log_heights`); `topic_log_sync.rs:59–60, 106–107, 132–135`; `manager/mod.rs:107–108`.
- Facade: `forge.rs:69, 70, 72, 77, 93, 105`; `streams/acked.rs:56, 74, 139`; `streams/subscription.rs:86`; `spaces/forge.rs:262`; `spaces/repair.rs:101, 133, 183`; `node.rs:584, 699, 702` (uses `SqliteStore` in production code).
- p2panda-net: `address_book/actor.rs` names `SqliteStore` at lines 8, 110, 114, 121, 126, 132, 157, all before its `#[cfg(test)]` at 374 [V].
- Identity (spaces): `identity.rs:132` (`set_prekey_secrets`), `:228` (`get_key_registry`).
- Greps: `SqliteStore|SqliteError` outside the store: 209 lines [V]. Same-named calls in `p2panda-encryption/src/ordering.rs` and `p2panda-core` (`mark_ready`, `ready`, …) are other crates and were excluded. The store's own tests call `SqliteStore::temporary()` 5 (logs), 1 (operations) and 5 (topics) times [V].

**E6. 0.7.1 vs the pin.** `diff -rq` on `src` shows 18 differing files and one new `processor` module [V]. Signature diffs on the changed trait files [V]:
- `logs/traits.rs`: `type Error: Error + Send`; `get_log_entries` renamed to `log_entries`.
- `topics/traits.rs`: `resolve_topics` and `topics` added.
- `orderer/traits.rs`: methods gained a `namespace` argument; `clear` and `clear_keys` added.
- `spaces/traits.rs`: `SpacesMessageStore<ARG>` renamed to `<T>`.

The traits in `operations`, `cursors`, `groups`, `key_secrets` and the `Transaction` file are unchanged.

**E7. Dependency facts (Cargo.toml, read [V]).** Store: `default = ["sqlite", "macros"]`; `groups`, `encryption`, `processor`, `spaces` and `test_utils` each list `sqlite`; `macros = []` (no dependency); `sqlx = "0.9.0"` optional. Downstream: `p2panda-stream/Cargo.toml:35, 44, 50` (feature gates `groups`, `processor`, `spaces`), `:61` (store, default features); `p2panda-sync/Cargo.toml:40` (default features); `p2panda-spaces/Cargo.toml:35` (`sqlite`, `spaces` forced); `p2panda-net/Cargo.toml:35, 81`; `p2panda/Cargo.toml:37`.

**E8. S1, S6, S7 (read).** S6 `sqlite_store.rs:10` uses rusqlite, not p2panda-store [V]; S6 RESULTS 82–87 document the libsqlite3-sys bound and the one-`links`-package rule [V]; S7 `RESULTS.md` 62–65 documents its own table layout; S7's crossover table covers only 1,000 and 10,000 chains [V]; S1 RESULTS 17 and 97 say sqlx and LogSync were not exercised [V]. A grep of `spikes-next` finds no p2panda crate [V].

### Corrections to the index and task docs

1. **N3 says "S7 tests it".** It does not. S7 used its own redb layout (`spikes-next/s7-range-real/RESULTS.md` 62–65), and no spike runs p2panda-store's traits [V].
2. **Design line 399** says the storage choice (p2panda-store on SQLite, or a plain store) "is decided in spike S6". S6 compared redb with rusqlite, not with p2panda-store [V].
3. **task-02 gotcha**, "p2panda-store conflicts with rusqlite": the conflict is real, but specific. rusqlite 0.40 needs libsqlite3-sys ^0.38, sqlx-sqlite 0.9.0 allows below 0.38, and cargo permits one `links = "sqlite3"` package [V: S6 RESULTS 82–87]. It disappears if p2panda-store is not in the graph.
4. **Store doc mismatches.** The cursor `set`/`delete` docs say they return `bool`, but both return `()` (`cursors/traits.rs:19–28`) [V]. The `prune_entries` doc says it deletes bodies; the SQL deletes rows [V, E4].
5. **`OooBuffer`** documents oldest-first eviction, but `push` calls `IndexMap::pop`, which removes the most recently inserted entry (`ooo.rs:292–299`; indexmap 2.14.2 `map.rs:1119–1125`) [V: read; not run].
6. **Pin vs crates.io.** The 0.7.1 API differs from the pin (E6). This bears on N14.
7. **Main design line 68** ("Fork proofs are not stored. Forks are refused and dropped.") conflicts with lines 28 and 94 (proofs are stored). I did not check its context [N], and I did not check `pkgs/id` for a proof table [N].

## Trait inventory (step 1)

Columns: method (trait line) | what callers rely on | production callers (file:line) | SQLite implementation (file:line). "None found" means no call outside the store crate in grep, not exhaustive [V].

### LogStore (`logs/traits.rs:21`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `get_latest_entry` (27) | newest entry of (author, log), committed data; None if absent | forge.rs:69 | `logs/sqlite/mod.rs:62–82` (pool, 73); SQL 21–33 (`ORDER BY seq_num DESC LIMIT 1`, 31–32) |
| `get_latest_entry_tx` (40) | same inside the open transaction; ingest reads it, then inserts | ingest/operation.rs:100, 163, 227; forge.rs:77 | `92–118` (tx at 98) |
| `get_log_heights` (51) | max seq per requested log; None when no requested log has rows | p2panda-sync api.rs:583 | `121–163` (`GROUP BY log_id`, 147; None at 158–162) |
| `get_log_size` (60) | (count, header + payload bytes) for (after, until]; Option | log_sync.rs:171 | `166–210` (SQL 173–191; pool 199); empty range gives `Some((0, 0))` [V] |
| `log_entries` (72) | sync fn returning a `'static` stream, ordered by seq; after exclusive, until inclusive | api.rs:613 (via `log_ranges`); log_sync.rs:292 | `214–247` (`ORDER BY` 51–52; pool clone 228) |
| `prune_entries` (83) | delete below `until`; returns row count; no transaction | log_prune/processor.rs:57 | `252–278` (DELETE 258–268; pool 272; doc 249–251) |

### OperationStore (`operations/traits.rs:11`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `insert_operation` (18) | insert by hash; bool = inserted; duplicate hash ignored; no same-position check | ingest/operation.rs:187; forge.rs:70, 105 | `operations/sqlite.rs:37–90` (tx 47; `INSERT OR IGNORE` 50; rows_affected 89) |
| `get_operation` (26) | committed read by hash; header re-decoded and re-verified | streams/subscription.rs:86; stream orderer/processor.rs:160 | `92–107` (pool 94; decode 218) |
| `get_operation_tx` (33) | same, inside the transaction | spaces/forge.rs:262; spaces/repair.rs:133 | `113–128` (tx 115) |
| `has_operation` (38) | committed existence | none outside tests | `130–142` (pool 131) |
| `has_operation_tx` (47) | existence inside the transaction (dedupe before insert) | ingest/operation.rs:86 | `144–156` (tx 146) |
| `delete_operation` (53) | delete by hash inside the transaction | tests only: p2panda-sync log_sync.rs:963 | `158–177` (tx 160) |
| `delete_operation_payload` (59) | sets body to NULL; committed write, no transaction | none found | `179–195` (pool 191) |

### TopicStore (`topics/traits.rs:41`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `associate` (45) | add (topic, author, data_id); bool = inserted | ingest/operation.rs:183, 191; forge.rs:72, 93 | `topics/sqlite.rs:23–59` (tx 30; `INSERT OR IGNORE` 33; `UNIQUE` in migration 20260217222329:8) |
| `remove` (53) | delete one association inside the transaction | none found | `62–95` (tx 69) |
| `resolve` (61) | topic to `BTreeMap<author, Vec<data_id>>`; order not guaranteed | p2panda-sync api.rs:642; topic_log_sync.rs:133 | `98–137` (pool 100; no `ORDER BY`, 103–110) |
| `resolve_topics` (64) | data_id to topics | none found | `140–178` (pool 146) |
| `topics` (71) | all topics inside the transaction | examples only: sneakernet.rs:155, 239 | `181–207` (tx 183; `DISTINCT`, no `ORDER BY`, 186) |

### CursorStore (`cursors/traits.rs:8`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `get_cursor` (14) | by name; committed | streams/acked.rs:56 | `cursors/sqlite.rs:19–46` (pool 21) |
| `set_cursor` (22) | upsert by name; requires a transaction; returns `()` (doc says `bool`) | acked.rs:74, 139 | `49–76` (tx 50; `ON CONFLICT` 58–62) |
| `delete_cursor` (27) | delete by name; requires a transaction; returns `()` | none found | `79–97` (tx 80) |

### OrdererStore (`orderer/traits.rs:16`; namespaced at the pin)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `mark_ready` (21) | add to the FIFO ready queue; re-queues an item that is ready but not queued | stream orderer/orderer.rs:107, 135 | `orderer/sqlite.rs:22–124` (tx 23; queue index 25–41; re-queue 76–117) |
| `mark_pending` (28) | record that an id waits for its deps; dedupe by set digest | orderer.rs:102 | `126–212` (sort 136; digest 139–147) |
| `get_next_pending` (37) | items waiting on id, as (id, deps) | orderer.rs:121 | `214–~255` (`DISTINCT`, no `ORDER BY`) |
| `take_next_ready` (44) | pop the oldest queued ready item; clear `in_queue` | orderer.rs:95 | `285–~325` (`ORDER BY queue_index` 296–297; `in_queue = FALSE` 318) |
| `remove_pending` (50) | drop pending rows that wait on id | orderer.rs:144 | `334–` (DELETE) |
| `ready` (57) | true when all keys are in the ready table; dequeued items still count | orderer.rs:100, 130 | `355–375` (count compared with `len` at 375) |
| `clear` (66), `clear_keys` (71) | delete ready and pending rows | none found (`replay.rs:67` matched `clear_keys`; not inspected) | `414–` |

Also `OrdererTestExt` (80–86): test helpers only.

### GroupsStore (`groups/traits.rs:12`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `set_groups_state_tx` (20) | upsert CBOR state by group hash; requires a transaction | stream groups/processor.rs:113; node.rs:699 | `groups/sqlite.rs:21–48` (tx 26; `INSERT OR REPLACE` 29) |
| `get_groups_state_tx` (27) | state by hash inside the transaction | groups/processor.rs:89; spaces/repair.rs:101 | `50–81` (tx 55) |

### KeySecretsStore (`key_secrets/traits.rs:9`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `get_prekey_secrets` (12) | the single `'default'` row; committed | one non-store call (not inspected) | `key_secrets/sqlite.rs:17–45` (pool 19; id at 12) |
| `set_prekey_secrets` (16) | upsert the single row; requires a transaction | spaces identity.rs:132 | `47–67` (tx 48; `INSERT OR REPLACE` 51) |

### Transaction (`traits.rs:10`; `macros.rs:17–26`)

| Method (line) | Callers rely on | Callers | SQLite impl |
|---|---|---|---|
| `begin` (16) | acquire the store's one write permit; waits while another permit is held | ingest/operation.rs:77, 222; orderer/processor.rs:122; groups/processor.rs:84 | `sqlite.rs:341–390` (semaphore 345–350; `BEGIN IMMEDIATE` 386) |
| `commit` (22) | commit; release the permit | ingest/operation.rs:137, 242; groups/processor.rs:117 | `414–426` |
| `rollback` (19) | roll back; release | none outside tests | `396–408` |
| drop without commit | rollback in a spawned task; a `tx!` body with an early `?` takes this path | `macros.rs:17–26` | `sqlite.rs:459–476` |

### Traits outside the brief (listed for completeness)

- **KeyRegistryStore** (`key_registry/traits.rs:9–20`): get and set. Used by spaces identity.rs:228. Its feature (`encryption`) implies `sqlite` [V].
- **SpacesStore, SpacesMessageStore** (`spaces/traits.rs:7–28`): `set_space_state_tx` used by `node.rs:702`, `space.rs:272`, `repair.rs:183`, stream `spaces/processor.rs:133`, and `manager.rs:652`. Feature `spaces` implies `sqlite` [V].
- **ProcessorStore** (`processor/traits.rs:9–15`): new at the pin; used by stream `orderer/processor.rs:171, 203`. Feature `processor` implies `sqlite` [V].
- **AddressBook** (`address_book/traits.rs:20–125`): ungated in `lib.rs`. Its SQLite implementation is 504 lines. p2panda-net's address-book actor names `SqliteStore` in non-test code [V]. Not analysed further.

## LogSync and TopicLogSync (step 2)

| Protocol | Store bound (file:line) | Store methods used (file:line) | Writes to the store? |
|---|---|---|---|
| LogSync | `LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash> + Clone + Send + 'static` (log_sync.rs:97) | `get_log_heights` via `log_heights` (api.rs:573–589; called at log_sync.rs:122); `get_log_size` (log_sync.rs:171); `log_entries` via `log_ranges` (api.rs:596–630; log_sync.rs:292) | No. Received operations go to the app as `OperationReceived` events in the Sync state |
| TopicLogSync | adds `TopicStore<T, VerifyingKey, L>` (topic_log_sync.rs:59–60, 106–107) | `TopicStore::resolve` (topic_log_sync.rs:132–135; api.rs:639–645), then the LogStore three | No |
| sync manager | `LogStore + TopicStore` (manager/mod.rs:107–108) | creates TopicLogSync sessions | No |
| ingest (receiver's app) | `Transaction + OperationStore + LogStore + TopicStore` (ingest/operation.rs:58–61, 152–155, 206–209) | `begin`; `has_operation_tx` (86); `get_latest_entry_tx` (100, 163, 227); `insert_operation` (187); `associate` (191); `commit` (137, 242) | Yes |

Receive check [V]: `Header::<E>::decode` (log_sync.rs:312) calls `AnyHeader::decode`, which calls `verifying_key.verify` (any.rs:344), which is `verify_strict` (identity.rs:196–198). So signatures are checked before the app sees an event, through p2panda's own path. Our canonical-form checks and the `MAX_AFTER` limit from task-02 still need to run in our code [I].

## Fit matrix (step 3)

Columns (our needs):
- **A** per-author chain: append and latest (ChainExt backlink chain)
- **B** `after` frontier: causal readiness
- **C** fork proof and freeze
- **D** waiting events with per-author caps and expiry
- **E** tuple-keyed subject and name/value indexes (task 06)
- **F** unique (author, seq)
- **G** sync: heights, sizes, entries, topic to logs (LogSync)
- **H** atomic ingest: check, insert and associate in one transaction
- **I** persisted p2panda-auth group state (task 03)

Legend: **●** covered as is; **◐** covered with a caveat or a semantic to reproduce; **○** not covered: needs an extra method or our own table; blank: not relevant.

| Trait method | A | B | C | D | E | F | G | H | I |
|---|---|---|---|---|---|---|---|---|---|
| `Transaction::begin` / `commit` / `rollback` | | | | | | | | ● | |
| `LogStore::get_latest_entry` | ◐ committed only | | ○ returns one fork; tie unspecified | | | ○ | | | |
| `LogStore::get_latest_entry_tx` | ● | | ○ | | | ◐ backlink checked in txn, not in store | | ● | |
| `LogStore::get_log_heights` | ● | | | | | | ● None when no rows | | |
| `LogStore::get_log_size` | | | | | | | ● `Some((0, 0))` when empty | | |
| `LogStore::log_entries` | | | | | | | ● ordered; (after, until] | | |
| `LogStore::prune_entries` | | | | | | | later (task 10): deletes headers too | | |
| `OperationStore::insert_operation` | ● | | ○ stores both forks | | | ○ | | ● | |
| `OperationStore::get_operation` / `_tx` | ● | ◐ | | | | | ● | | |
| `OperationStore::has_operation_tx` | | | | | | | | ● dedupe | |
| `OperationStore::has_operation` | | ● dependency present | | | | | | | |
| `OperationStore::delete_operation` / `delete_operation_payload` | | | | | | | | | later (task 10) |
| `TopicStore::associate` | ◐ one log per author: topic to authors | | | | | | ◐ | ● | |
| `TopicStore::resolve` | | | | | | | ● | | |
| `TopicStore::remove` / `resolve_topics` / `topics` | not needed | | | | | | | | |
| `CursorStore::*` | not needed (S7 restarts sessions) | | | | | | | | |
| `OrdererStore::mark_ready` / `ready` / `take_next_ready` | | ◐ "ready" means processed | | | | | | | |
| `OrdererStore::mark_pending` / `get_next_pending` / `remove_pending` | | ◐ | | ○ no caps, no expiry | | | | | |
| `OrdererStore::clear` / `clear_keys` | | | | ◐ by key only | | | | | |
| `GroupsStore::set` / `get_groups_state_tx` | | | | | | | | ◐ txn only | ◐ needs `groups`, which implies sqlx |
| `KeySecretsStore::*` | | | | | | | | | not needed unless the p2panda-encryption key managers are used |
| **Outside the traits** | | | ○ proof table and freeze flag | ○ waiting table, per-author counts, expiry index | ○ tuple-keyed tables (S6 layout) | ○ check inside the write txn (S6, S7) | ○ key-order range and fingerprints (S7) | | ○ own table |

## SQL-only semantics to reproduce (step 4)

Tags show how each behaviour was checked. "Redb must" is the behaviour an implementation has to reproduce or deliberately replace.

| # | Behaviour | SQLite (file:line) | Redb must | Tag |
|---|---|---|---|---|
| 1 | Insert is idempotent by hash: `INSERT OR IGNORE` on PK(hash); returns rows > 0 | operations/sqlite.rs:50, 89; migration 20250928132835:4 | check the hash table in the write txn; false if present | V |
| 2 | No (author, log, seq) uniqueness: `operations_v1` has only the hash PK; indexes exist only on topics, orderer and address book | migration 20250928132835 (whole file) | enforce ourselves: same position and same hash means already present; different hash goes to the fork path | V (schema); V (both forks stored, E4) |
| 3 | Latest by seq: `ORDER BY seq_num DESC LIMIT 1`; seq is bound as text (`to_string`) into an INTEGER column, so NUMERIC affinity orders it numerically | logs/sqlite/mod.rs:31–32; operations/sqlite.rs:79; migration line 11 | big-endian u32 keys; reverse range | V (10 over 9, E4) |
| 4 | Tie at equal seq: no tie-break in the query | logs/sqlite/mod.rs:31–32 | define our own rule and store the proof | V (one observation: first-inserted returned, E4) |
| 5 | Heights: `MAX(seq) GROUP BY log_id` for the requested logs; None when no rows | logs/sqlite/mod.rs:127–147, 158–162 | last key in each requested log's range; None when all are empty | V (`Some({0: 4})`; None when empty, E4) |
| 6 | Range: `seq > after` (or `>= 0` when `after` is None) and `seq <= until` | logs/sqlite/mod.rs:45–50, 184–189, 196–198, 226–227 | `(after, until]`; None means from 0 inclusive, until None means no upper bound | V (`[2, 3]` for after 1, until 3, E4) |
| 7 | Stream order by seq | logs/sqlite/mod.rs:51–52 | key order | V (E4) |
| 8 | `get_log_size` on an empty range returns `Some((0, 0))`; the trait says Option. Predicted an error from u32 fields (`models.rs:10–15`); refuted | logs/sqlite/mod.rs:173–200 | return `Some((0, 0))`. Callers treat zero bytes as nothing to send (log_sync.rs:188) | V (observed); mechanism not traced [N] |
| 9 | Prune: `DELETE seq < until` on the pool, outside any transaction; removes whole rows, headers included, although the doc says bodies | logs/sqlite/mod.rs:249–251, 258–273 | range delete (`retain_in` or `extract_from_if`, table.rs:194–288); decide headers or bodies explicitly | V (has_operation false after prune, E4) |
| 10 | One store-wide transaction shared by all clones; `_tx` methods and writes need an open one, else `TransactionMissing` | sqlite.rs:259–264, 305–313; cursors/sqlite.rs:50 | a write-slot `Option<WriteTransaction>` behind a permit; the same hazard: any task's `_tx` call uses the open transaction | V (E4, step 0; sqlite test `serialized_transactions`) |
| 11 | Single writer: `Semaphore(1)`; `BEGIN IMMEDIATE` | sqlite.rs:275, 345–350, 386 | tokio Semaphore(1); `begin_write` through `spawn_blocking` (it blocks while a writer is open, db.rs:1755–1759) | V |
| 12 | Uncommitted permit: async rollback in a spawned task; a `?` inside `tx!` takes this path | sqlite.rs:459–476; macros.rs:17–26 | WriteTransaction drop aborts (transactions.rs:2744–2750); clear the slot in Drop | V |
| 13 | Non-transactional reads (pool) see committed data; `_tx` reads see the open transaction | operations/sqlite.rs:92–107 vs 113–128; logs/sqlite/mod.rs:62–82 vs 92–118 | `begin_read` snapshot vs the write txn (read-your-writes) | V |
| 14 | Journal mode is the SQLite default: no `journal_mode` or WAL pragma is set | sqlite.rs:185–186 (none set) | nothing to copy; redb readers do not block the writer | V (none set); I (locking effect) [N, not run] |
| 15 | Upserts and sets: cursors `ON CONFLICT DO UPDATE`; groups and key_secrets `INSERT OR REPLACE`; topics `UNIQUE` plus `INSERT OR IGNORE` | cursors/sqlite.rs:53–62; groups/sqlite.rs:29; key_secrets/sqlite.rs:51; topics migration line 8, topics/sqlite.rs:33 | insert overwrites; insert-if-absent for topics | V |
| 16 | Unordered results: `resolve` and `resolve_topics` have no `ORDER BY`; `topics()` uses `DISTINCT` without order | topics/sqlite.rs:103–110, 149–156, 186–190 | key order (callers do not depend on order [I]) | V (text); I (callers) |
| 17 | Orderer: queue index is `MAX + 1` inside the txn; FIFO by `queue_index` with an `in_queue` flag; re-queue on re-mark; pending dedupe by set digest; `ready` counts dequeued rows | orderer/sqlite.rs:25–41, 76–117, 136–147, 296–297, 318, 355–375; ordering migration lines 12, 66–72 | only if OrdererStore is implemented (not recommended) | V |
| 18 | Keys: log id stored as CBOR bytes; author and hash as hex text | logs/sqlite/mod.rs:70–72, 102–103; operations/sqlite.rs:69, 75 | fixed binary keys (32-byte author, u32 BE seq, 32-byte hash), as S6 and S7 did | V |
| 19 | Every decode re-verifies the signature | operations/sqlite.rs:218; any.rs:344; identity.rs:197 | same cost unless decoded headers are cached | V |
| 20 | Schema evolution: 11 migrations, e.g. rename `public_key` to `verifying_key` | migrations/ | none; data may be discarded (task-02) | V |

## Effort and risk (step 5)

| Option | Implementation (lines) | Tests (lines) | Dependency effect | Risk |
|---|---|---|---|---|
| **(a)** Traits over redb in our crate, minimal set: Transaction, OperationStore, LogStore, TopicStore (+ CursorStore if needed) | 650–900 [I] | 300–450 [I]; p2panda-store's logs, operations and topics tests are 581 lines with 11 `temporary()` calls [V]; bodies call trait methods [I] | `default-features = false`: no sqlx, sqlite or tokio in the normal tree [V]. Add `macros` alone for `tx!` (no dependency) [V] | Medium. Store-wide transaction shared by all tasks; replicated, not fixed [V behaviour]. Redb `Range` owns its snapshot (table.rs:919–935, 1129–1140), so streams can be `'static` [V source]. No runtime test yet [N] |
| (a) + OrdererStore | +300–400 [I] | +150 [I] | same | High. The most complex SQLite port (528 lines [V]). Our waiting queue needs caps and expiry the trait lacks, so we would write it anyway |
| (a) + GroupsStore, KeySecrets, KeyRegistry, Spaces, Processor | +150–300 [I] | +100 [I] | each feature implies `sqlite`, so sqlx returns [V, E3] | Medium: sqlx in the graph, or copy the trait definitions (about 30 lines each [I]) |
| **(b)** Fork: replace the SQLite backend with redb, traits unchanged | same as (a): 650–900 [I], plus the feature impls if the features are kept | port or delete p2panda-store's suites: 1,379 lines [V] | removes sqlx and the feature implications, so every trait and crate is usable without SQLite [I] | Medium-high. Deletes 3,526 lines of SQLite code and 11 migrations [V]; 209 downstream lines to rename or alias [V]; 18 store files differ from 0.7.1, so each upstream bump conflicts with the deleted files [V diff; I consequence] |
| **(c)** Own store API, no p2panda-store (N3's alternative) | 500–700 [I] | 250–350 [I] | no p2panda crate in the store path; our ingest (about 100 lines of logic, ingest/operation.rs:97–196 [V]) and range sync | Low to medium. No interop with p2panda-sync or p2panda-stream |

## Options

| Option | What we get | What we give up | When |
|---|---|---|---|
| (a) Traits over redb, minimal set, no p2panda-sync or -stream | the owner's preferred shape; no sqlx; about 650–900 lines; no fork | GroupsStore, KeySecrets and the other feature traits (they need sqlx); p2panda's LogSync and ingest processors | Now (recommended) |
| (a) + p2panda-sync / -stream | LogSync and processors work with our store | sqlx and libsqlite3-sys in the graph; link check against libsql-ffi not done [N] | Only if N5 keeps LogSync and a SQLite-linked binary is acceptable |
| (b) Fork | no SQLite anywhere; all traits and crates usable | 3,526 lines deleted; ongoing merges against 18 changed files; ~200 downstream edits; owned maintenance | If LogSync must stay and no SQLite may be linked (the owner accepts this) |
| (c) Own API | smallest surface; explicit transactions; our fork rules directly | no interop with p2panda-sync or -stream; a new API to document | If we never use p2panda-sync or -stream |
| (d) Keep p2panda-store's SQLite | works today | SQLite stays; the owner said nothing has to keep SQLite | Not analysed |

## Recommendation (step 6)

**Choose (a): implement Transaction, OperationStore, LogStore and TopicStore over redb in our crate, with p2panda-store at `default-features = false` (plus `macros` for `tx!`). Do not fork now. Do not implement OrdererStore. Do not put p2panda-sync or p2panda-stream in the graph yet.**

Reasons:
1. It matches the owner's preference. The accepted decision in the main design (2026-10-11T16-08-10Z, "p2panda-store traits over redb", line 869) says the same [V].
2. Nothing blocks it. A type of ours implements `Transaction` and `LogStore` with default features off, and the normal tree has no sqlx, sqlite or tokio [V, E2]. The redb pieces check out: the write transaction can sit behind a permit and cross awaits in `Send` futures, and a range can own its snapshot [V, E1; table.rs].
3. The cost is small and there is nothing to rebase. About 650–900 lines plus tests, against 3,526 lines of deletions and a permanent merge with 18 changed files for the fork [I, V].
4. The SQL-only behaviours in step 4 are simple to match. The two we must decide ourselves (fork handling and the tie at equal seq) are ours anyway, because the design stores fork proofs and freezes the chain.
5. OrdererStore cannot express our caps and expiry (design line 804), so our waiting table is needed regardless, and OrdererStore is the riskiest SQL port [V].
6. GroupsStore and the other feature traits are not worth the sqlx they bring. Persist p2panda-auth state in our own table (the matrix's column I becomes ○).

**What would change it:**
- **N5 keeps LogSync (p2panda-sync) for small groups.** Then sqlx returns under (a). If the binary must not link SQLite, choose (b), which the owner accepts. If not, first measure LogSync at small chain counts: S7 measured the crossover only at 1,000 and 10,000 chains [V], so the 40-chain threshold in N5 is untested.
- **The port shows the store-wide transaction is unsafe for ingest.** For example, two tasks' `_tx` calls landing in one transaction. Then the fork with explicit transaction handles (a trait change), or option (c).
- **We use neither p2panda-sync nor p2panda-stream.** The traits then add a shape but no interop. Option (c) is about the same size and avoids the global transaction. Keep (a) if the owner still prefers traits.
- **Power-loss testing (N13) finds redb weaker than needed.** Change the engine, not the traits.
- **We need a second process to read the index while the node runs.** redb refuses a second opener (`DatabaseAlreadyOpen`, page_manager.rs:739–805 [V]). Then SQLite in WAL mode is the answer, and p2panda-store's `SqliteStore` becomes viable as is.

## Not verified

- **[N] No redb implementation of the traits exists.** The effort figures are calibrated estimates, not measurements.
- **[N] Concurrency.** The store-wide transaction slot, dirty reads by other tasks, the spawned-rollback path, and streams held across awaits were not run.
- **[N] Power loss** (N13) was not tested.
- **[I, not run] Locking.** No journal_mode is set, so SQLite runs in the default rollback-journal mode, where a long reader can delay a writer's commit. This follows from SQLite's documented locking rules; it was not measured.
- **[N] Linking.** libsql-ffi has no `links` key [V: Cargo.toml], so cargo accepts it alongside libsqlite3-sys. Whether the two can coexist in one binary (duplicate SQLite symbols) was not built.
- **[V read, N build] Downstream dependencies.** The `sqlite` implications in p2panda-sync, -stream, -spaces and -net were read from Cargo.toml, not built.
- **[N] Tie-break.** `get_latest_entry` on equal seq was observed once (first-inserted). The rule is unspecified.
- **[N] Other traits.** AddressBook, Spaces, Processor and KeyRegistry were listed, not analysed.
- **[N] OrdererStore.** Read, not run.
- **[N] get_log_size mechanism.** The result is observed; why SUM over no rows decodes to 0 was not traced.
- **[N] Performance.** S6 and S7 numbers are for their own layouts; no trait-level throughput was measured.
- **[N] Test bodies.** I did not read every p2panda-store test body to confirm it uses only trait methods. The constructor swap is the expected port work.
