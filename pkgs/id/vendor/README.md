# Vendored crates

## p2panda-sync 0.7.1

**Origin.** `https://github.com/p2panda/p2panda`, rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, crate `p2panda-sync/`. The same rev as the other p2panda crates in `pkgs/id/Cargo.toml`. License `MIT OR Apache-2.0` (`p2panda-sync/LICENSES/`).

**Why.** p2panda-sync depends on p2panda-store with its default features, and the default includes `sqlite`, which pulls in `sqlx`. The store is redb (explainers section 14), so the copy removes that default. Vendoring was chosen over a fork, so nothing is pushed or published.

**The substantive change** (one line, in `p2panda-sync/Cargo.toml`):

```diff
-p2panda-store = { workspace = true, default-features = true }
+p2panda-store = { git = "https://github.com/p2panda/p2panda", rev = "3e84cac48bba05c6857cce2ec433862b97f8d53c", default-features = false }
```

**Mechanical changes**, needed because this copy is outside the p2panda workspace:

- Every `workspace = true` is replaced by the root's value (`[workspace.dependencies]`). `p2panda-core` and `p2panda-stream` become the same git source and rev.
- `[lints] workspace = true` is removed, so the copy takes no lints from this crate.
- `[dev-dependencies]` is removed. Its tests use the SQLite test helpers and are not built here.
- `examples/` is removed. The examples construct `SqliteStore`.
- `LICENSES/` is added.

**Verify.** The copy matches upstream except the files above:

```sh
SRC=$(ls -d ~/.cargo/git/checkouts/p2panda-*/3e84cac/p2panda-sync)
diff -rq "$SRC" pkgs/id/vendor/p2panda-sync   # Cargo.toml differs; examples/ and LICENSES/ differ
diff -u "$SRC/Cargo.toml" pkgs/id/vendor/p2panda-sync/Cargo.toml
git -C "$SRC/.." rev-parse HEAD                # 3e84cac48bba05c6857cce2ec433862b97f8d53c
```

`sqlx` is absent from the build. `cargo tree -i sqlx` reports no match, and `store_traits`'s `test_the_lock_has_no_sqlite_with_logsync_wired` fails if `Cargo.lock` names `sqlx`, `sqlite`, `rusqlite` or `libsqlite3-sys`, or if the `p2panda-store` line above loses `default-features = false`.

**To update.** Change the rev in the three places it appears (this file, the two git lines in `vendor/p2panda-sync/Cargo.toml`, and the git lines in `pkgs/id/Cargo.toml`). Then copy the crate again and reapply the changes above.

## p2panda-encryption 0.7.1 (DataScheme fork)

**Origin.** `https://github.com/p2panda/p2panda`, rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, crate `p2panda-encryption/`. The same rev as the other p2panda crates in `pkgs/id/Cargo.toml`. License `MIT OR Apache-2.0` (`p2panda-encryption/LICENSES/`).

**Why.** The crate's `process_welcome` overwrites the joiner's membership with the welcome's copy, and that copy lacks the joiner, so the joiner is never welcomed and reads nothing. The fix, and `add_from_join` (the welcome carries no earlier secret, so the group needs an immediate update after each join), are in our fork and not upstream. Decision (owner): keep the fix in this repo. Source: the spike `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/shared/s3-history/` (`logs/patch.diff`, `RESULTS.md`).

**The changes** (the S3 patch, applied by hand):

- `src/data_scheme/dcgka.rs`: `process_add` skips `DGM::add` for the joiner. `process_welcome` takes the add's `seq`, calls `DGM::from_welcome`, then `DGM::add` for the joiner itself. Both are needed; the test is `joiner_is_welcomed_and_reads_messages_after_the_add` in `src/data_scheme/tests.rs`.
- `src/data_scheme/group.rs`: new public `EncryptionGroup::add_from_join` (25 lines). It calls `Dcgka::add` with an empty bundle and runs the same local steps as `add`. No existing item changes visibility.
- `src/data_scheme/tests.rs`: the regression test above, which fails on the stock crate (the joiner's roster is `[0, 1, 2]`).
- `Cargo.toml`: every `workspace = true` is replaced by the root's value, since this copy is outside the p2panda workspace. `[lints] workspace = true` is removed. `p2panda-core` is the same git source and rev, with `default-features = false`, as upstream's workspace declares it. `[dev-dependencies]` is unchanged (none).
- `LICENSES/` is added.

**Size.** `src/` differs from upstream by 63 lines added and 2 removed (3 files); `Cargo.toml` by 13 changed lines.

**Features.** `data_scheme` on, `test_utils` and `message_scheme` off. The `test_utils` tests in the fork use the crate's own `Network` and `TestDgm`, which the test cfg enables.

**Verify.**

```sh
C=$(ls -d ~/.cargo/git/checkouts/p2panda-*/3e84cac)
diff -rq "$C/p2panda-encryption" pkgs/id/vendor/p2panda-encryption   # the files above differ, nothing else
git -C "$C/.." rev-parse HEAD                                         # 3e84cac48bba05c6857cce2ec433862b97f8d53c
cd pkgs/id/vendor/p2panda-encryption && cargo test --lib data_scheme  # run standalone; delete the Cargo.lock it writes
```

**To update.** Change the rev in the three places it appears (this section, the `p2panda-encryption/Cargo.toml` git lines, and `pkgs/id/Cargo.toml`). Copy the crate again, then reapply the changes above and run the fork's test.
